//! Bounded float64 reference scoring and optional numerical-only Mojo IPC.
use serde::Serialize;
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender},
    thread::JoinHandle,
    time::Duration,
};
pub const DIM: usize = 17;
pub const MAX_BATCH: usize = 4096;
#[derive(Debug, Serialize)]
pub struct ScoreBatch {
    pub scores: Vec<f64>,
    pub engine: String,
    pub fallback_reason: Option<String>,
}
pub trait DecisionScorer {
    fn score(&mut self, features: &[[f64; DIM]]) -> Result<ScoreBatch, String>;
}
#[derive(Clone)]
pub struct LinearScorer {
    weights: [f64; DIM],
}
impl LinearScorer {
    pub fn new(version: &str, weights: Vec<f64>) -> Result<Self, String> {
        if version != "1.0"
            || weights.len() != DIM
            || weights.iter().any(|x| !x.is_finite() || x.abs() > 1e6)
        {
            return Err("invalid_model".into());
        }
        Ok(Self {
            weights: weights.try_into().map_err(|_| "invalid_model")?,
        })
    }
}
fn validate(rows: &[[f64; DIM]]) -> Result<(), String> {
    if rows.is_empty()
        || rows.len() > MAX_BATCH
        || rows
            .iter()
            .flatten()
            .any(|x| !x.is_finite() || x.abs() > 1.0)
    {
        return Err("invalid_features".into());
    }
    Ok(())
}
impl DecisionScorer for LinearScorer {
    fn score(&mut self, rows: &[[f64; DIM]]) -> Result<ScoreBatch, String> {
        validate(rows)?;
        Ok(ScoreBatch {
            scores: rows
                .iter()
                .map(|r| r.iter().zip(self.weights).map(|(a, b)| a * b).sum())
                .collect(),
            engine: "rust".into(),
            fallback_reason: None,
        })
    }
}
fn numeric(v: Option<&Value>) -> Result<(f64, f64), String> {
    match v {
        None | Some(Value::Null) => Ok((0., 1.)),
        Some(v) => match v.as_f64() {
            Some(x) if x.is_finite() && x.abs() <= 1. => Ok((x, 0.)),
            _ => Err("invalid_numeric_feature".into()),
        },
    }
}
pub fn extract_features(context: &Value, candidate: &Value) -> Result<[f64; DIM], String> {
    let mut out = [0.; DIM];
    for (i, key) in [
        "cost",
        "effort",
        "speed",
        "reuse",
        "customer_impact",
        "irreversibility",
    ]
    .iter()
    .enumerate()
    {
        let (v, m) = numeric(candidate.get("attributes").and_then(|a| a.get(key)))?;
        out[i] = v;
        out[i + 6] = m;
    }
    let fact = |key: &str| -> Result<f64, String> {
        let found = context
            .get("facts")
            .and_then(Value::as_array)
            .and_then(|fs| {
                fs.iter().find(|f| {
                    f.get("key").and_then(Value::as_str) == Some(key)
                        && f.get("evidence_status").and_then(Value::as_str) == Some("explicit")
                })
            });
        if let Some(f) = found {
            if f.get("value").is_some_and(|v| !v.is_null())
                && f.get("unit").and_then(Value::as_str) != Some("ratio")
            {
                return Err("context_feature_requires_ratio_unit".into());
            }
        }
        numeric(found.and_then(|f| f.get("value"))).map(|(v, _)| v)
    };
    out[12] = fact("deadline_pressure")? * out[2];
    out[13] = fact("asset_importance")? * out[3];
    out[14] = fact("loss_tolerance")? * out[5];
    out[15] = fact("customer_impact")? * out[2];
    out[16] = fact("budget_pressure")? * out[0];
    Ok(out)
}
struct Worker {
    child: Child,
    tx: Option<SyncSender<Vec<u8>>>,
    rx: Receiver<Result<Vec<f64>, String>>,
    thread: Option<JoinHandle<()>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
pub struct MojoScorerAdapter {
    reference: LinearScorer,
    worker: Option<Worker>,
    reason: Option<String>,
    timeout: Duration,
}
impl MojoScorerAdapter {
    pub fn new(reference: LinearScorer, executable: &Path, timeout: Duration) -> Self {
        let mut this = Self {
            reference,
            worker: None,
            reason: None,
            timeout,
        };
        match Self::start(executable) {
            Ok(w) => this.worker = Some(w),
            Err(e) => this.reason = Some(e),
        }
        this
    }
    fn start(path: &Path) -> Result<Worker, String> {
        let mut child = Command::new(path)
            .env("MODULAR_TELEMETRY_ENABLED", "false")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("startup_failed: {e}"))?;
        let mut input = child.stdin.take().ok_or("missing_stdin")?;
        let output = child.stdout.take().ok_or("missing_stdout")?;
        let (tx, requests) = mpsc::sync_channel::<Vec<u8>>(1);
        let (results, rx) = mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || {
            let mut reader = BufReader::new(output);
            while let Ok(bytes) = requests.recv() {
                let result = (|| -> Result<Vec<f64>, String> {
                    input
                        .write_all(&bytes)
                        .and_then(|_| input.flush())
                        .map_err(|_| "worker_write_failed")?;
                    let mut line = Vec::new();
                    (&mut reader)
                        .take(256 * 1024)
                        .read_until(b'\n', &mut line)
                        .map_err(|_| "worker_read_failed")?;
                    if line.last() != Some(&b'\n') {
                        return Err("worker_response_unbounded_or_eof".into());
                    }
                    serde_json::from_slice(&line).map_err(|_| "invalid_worker_response".into())
                })();
                let failed = result.is_err();
                if results.send(result).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Worker {
            child,
            tx: Some(tx),
            rx,
            thread: Some(thread),
        })
    }
}
impl DecisionScorer for MojoScorerAdapter {
    fn score(&mut self, rows: &[[f64; DIM]]) -> Result<ScoreBatch, String> {
        validate(rows)?;
        if let Some(w) = &self.worker {
            let mut bytes = serde_json::to_vec(
                &serde_json::json!({"weights":self.reference.weights,"features":rows}),
            )
            .map_err(|_| "serialization_failed")?;
            bytes.push(b'\n');
            let result =
                w.tx.as_ref()
                    .ok_or("worker_closed")?
                    .try_send(bytes)
                    .map_err(|_| "worker_queue_failed".to_string())
                    .and_then(|_| {
                        w.rx.recv_timeout(self.timeout)
                            .map_err(|_| "worker_timeout_or_exit".to_string())
                    })
                    .and_then(|x| x);
            match result {
                Ok(scores)
                    if scores.len() == rows.len() && scores.iter().all(|x| x.is_finite()) =>
                {
                    return Ok(ScoreBatch {
                        scores,
                        engine: "mojo".into(),
                        fallback_reason: None,
                    });
                }
                Ok(_) => self.reason = Some("invalid_worker_scores".into()),
                Err(e) => self.reason = Some(e),
            }
            self.worker.take();
        }
        let mut result = self.reference.score(rows)?;
        result.fallback_reason = self.reason.clone();
        Ok(result)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_and_interactions() {
        let f=extract_features(&serde_json::json!({"facts":[{"key":"deadline_pressure","value":0.5,"evidence_status":"explicit","unit":"ratio"}]}),&serde_json::json!({"attributes":{"speed":0.5}})).unwrap();
        assert_eq!(f[12], 0.25);
        assert_eq!(f[6], 1.);
        assert_eq!(f[8], 0.);
    }
    #[test]
    fn rejects_wrong_or_absent_context_units() {
        for unit in [serde_json::Value::Null, serde_json::json!("day")] {
            let context = serde_json::json!({"facts":[{"key":"deadline_pressure","value":1.0,"evidence_status":"explicit","unit":unit}]});
            assert!(
                extract_features(&context, &serde_json::json!({"attributes":{"speed":0.5}}))
                    .is_err()
            );
        }
    }
    #[test]
    fn rejects_nonfinite_and_falls_back() {
        assert!(LinearScorer::new("1.0", vec![f64::NAN; DIM]).is_err());
        let mut scorer = MojoScorerAdapter::new(
            LinearScorer::new("1.0", vec![1.; DIM]).unwrap(),
            Path::new("/nonexistent/cdna-mojo"),
            Duration::from_millis(20),
        );
        let out = scorer.score(&[[0.; DIM]]).unwrap();
        assert_eq!(out.engine, "rust");
        assert!(out.fallback_reason.is_some());
        assert!(scorer.score(&[[f64::INFINITY; DIM]]).is_err());
    }
}

#[cfg(all(test, unix))]
mod worker_failure_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn timeout_is_bounded_and_sticky() {
        let dir = std::env::temp_dir().join(format!("cdna-timeout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("worker");
        std::fs::write(&path, "#!/bin/sh\nexec sleep 10\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let start = std::time::Instant::now();
        let mut s = MojoScorerAdapter::new(
            LinearScorer::new("1.0", vec![1.; DIM]).unwrap(),
            &path,
            Duration::from_millis(50),
        );
        let a = s.score(&[[0.; DIM]]).unwrap();
        assert_eq!(a.engine, "rust");
        assert!(a.fallback_reason.unwrap().contains("timeout"));
        assert!(start.elapsed() < Duration::from_secs(2));
        assert_eq!(s.score(&[[1.; DIM]]).unwrap().scores, vec![17.]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
