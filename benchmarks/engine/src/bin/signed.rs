//! Synthetic evaluator signatures test trust gates and cost; never real acceptance evidence.
use anyhow::{Result, ensure};
use cdna_app::{Authority, Engine, evolution_cli};
use cdna_evolution::{
    Candidate, Dataset, DatasetCase, EvaluationReceipt, HumanApproval, InferenceSupport,
    ModelConfig, Outcome, PairedCase, Signed, Split, content_hash, hash, signing_bytes,
};
use cdna_store::Store;
use ed25519_dalek::{Signer, SigningKey};
use serde::Serialize;
use serde_json::{Value, json};
use std::{hint::black_box, time::Instant};
use uuid::Uuid;
use zeroize::Zeroizing;
fn signed<T: Serialize>(payload: T, key: &SigningKey) -> Signed<T> {
    let signature = key
        .sign(&signing_bytes(&payload).unwrap())
        .to_bytes()
        .to_vec();
    Signed { payload, signature }
}
fn op(e: &mut Engine, v: Value, eval: &SigningKey, human: &SigningKey) -> Result<Value> {
    evolution_cli::run(
        &mut e.store,
        v,
        eval.verifying_key().to_bytes(),
        human.verifying_key().to_bytes(),
    )
}
fn request(w: Uuid) -> Value {
    json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":w,"mode":"imitate","domain":"product_delivery","context":{"as_of":"2026-09-22T00:00:00Z","summary":"Unseen synthetic structured input","facts":[{"key":"deadline_pressure","value":0.5,"unit":"ratio","evidence_status":"explicit"},{"key":"asset_importance","value":0.5,"unit":"ratio","evidence_status":"explicit"},{"key":"loss_tolerance","value":0.5,"unit":"ratio","evidence_status":"explicit"},{"key":"customer_impact","value":0.5,"unit":"ratio","evidence_status":"explicit"},{"key":"budget_pressure","value":0.5,"unit":"ratio","evidence_status":"explicit"}],"unknown_fields":[]},"candidates":[{"id":"a","text":"Option A","attributes":{"cost":0.1,"effort":0.2,"speed":0.8,"reuse":0.3,"customer_impact":0.4,"irreversibility":0.2}},{"id":"b","text":"Option B","attributes":{"cost":0.1,"effort":0.2,"speed":0.2,"reuse":0.3,"customer_impact":0.4,"irreversibility":0.2}}],"include_evidence":true,"allow_cloud":false})
}
fn rank(e: &mut Engine, r: Value) -> Result<Value> {
    e.execute(json!({"operation":"rank","request":r}), Authority::Human)
}
fn probe(mode: &str, n: usize) -> Result<Value> {
    let dir = tempfile::tempdir()?;
    let store = Store::create(dir.path().join("vault"), &Zeroizing::new("a".repeat(64)))?;
    let eval = SigningKey::from_bytes(&[61; 32]);
    let human = SigningKey::from_bytes(&[62; 32]);
    let mut e = Engine::new(store, "unused".into(), false)
        .with_evolution_trust(eval.verifying_key(), human.verifying_key())?;
    let w = Uuid::new_v4();
    e.store.create_workspace(w)?;
    let epoch = e.store.epoch()?;
    let family = Uuid::new_v4();
    let cases = (0..201)
        .map(|i| DatasetCase {
            id: Uuid::new_v4(),
            family_id: if mode == "one_test_family" && i > 0 {
                family
            } else {
                Uuid::new_v4()
            },
            split: if i == 0 { Split::Train } else { Split::Test },
            timestamp: if i == 0 { 1 } else { 2 },
            input_hash: hash(format!("synthetic-{i}").as_bytes()),
            answer: "a".into(),
            independent_human_answer: true,
            model_exposure: false,
            domain: if mode == "one_case_domain" && i == 200 {
                "risk_reputation".into()
            } else {
                "product_delivery".into()
            },
        })
        .collect();
    let dataset = Dataset {
        id: Uuid::new_v4(),
        workspace_id: w,
        input_revision: epoch,
        deletion_epoch: epoch,
        cases,
    };
    let config = ModelConfig {
        feature_version: "1.0".into(),
        weights: (0..17).map(|i| if i == 2 { 1.0 } else { 0.0 }).collect(),
        inference: Some(InferenceSupport {
            schema_version: "1.0".into(),
            slope: 2.0,
            domains: if mode == "one_case_domain" {
                vec!["product_delivery".into(), "risk_reputation".into()]
            } else {
                vec!["product_delivery".into()]
            },
            valid_from_unix: 0,
            valid_until_unix: 4_000_000_000,
            min_margin: 0.05,
            max_margin: 2.0,
            missing_patterns: vec![0],
            feature_min: vec![-1.0; 17],
            feature_max: vec![1.0; 17],
        }),
    };
    let artifact = signing_bytes(&config).map_err(anyhow::Error::msg)?;
    let candidate = Candidate {
        id: Uuid::new_v4(),
        workspace_id: w,
        input_revision: epoch,
        deletion_epoch: epoch,
        artifact_hash: hash(&artifact),
        training_dataset_id: dataset.id,
        training_dataset_hash: content_hash(&dataset).map_err(anyhow::Error::msg)?,
        config,
    };
    let outcome = Outcome {
        choice: Some("a".into()),
        policy_violations: 0,
        latency_us: 10,
        peak_memory_bytes: 1024,
    };
    let receipt = signed(
        EvaluationReceipt {
            id: Uuid::new_v4(),
            workspace_id: w,
            input_revision: epoch,
            deletion_epoch: epoch,
            candidate_id: candidate.id,
            artifact_hash: candidate.artifact_hash.clone(),
            baseline_artifact_hash: hash(b"synthetic-baseline"),
            dataset_id: dataset.id,
            dataset_hash: content_hash(&dataset).map_err(anyhow::Error::msg)?,
            evaluator_version: "synthetic-review-not-personal-evidence".into(),
            test_previously_exposed: false,
            rows: dataset
                .cases
                .iter()
                .filter(|c| c.split == Split::Test)
                .map(|c| PairedCase {
                    case_id: c.id,
                    input_hash: c.input_hash.clone(),
                    candidate: outcome.clone(),
                    baseline: outcome.clone(),
                })
                .collect(),
            candidate_learning_seconds: 1,
            baseline_learning_seconds: 1,
        },
        &eval,
    );
    let approval = signed(
        HumanApproval {
            workspace_id: w,
            input_revision: epoch,
            deletion_epoch: epoch,
            candidate_id: candidate.id,
            artifact_hash: candidate.artifact_hash.clone(),
            receipt_hash: content_hash(&receipt).map_err(anyhow::Error::msg)?,
            human_id: Uuid::new_v4(),
        },
        &human,
    );
    op(
        &mut e,
        json!({"operation":"register","workspace_id":w,"expected_revision":0,"candidate":candidate,"artifact":artifact}),
        &eval,
        &human,
    )?;
    let evaluated = op(
        &mut e,
        json!({"operation":"evaluate","workspace_id":w,"expected_revision":1,"receipt":receipt,"dataset":dataset}),
        &eval,
        &human,
    );
    if let Err(err) = evaluated {
        return Ok(json!({"probe":mode,"accepted":false,"error":err.to_string()}));
    }
    op(
        &mut e,
        json!({"operation":"approve","workspace_id":w,"expected_revision":2,"approval":approval}),
        &eval,
        &human,
    )?;
    e.execute(
        json!({"operation":"activate_model","workspace_id":w,"id":candidate.id}),
        Authority::Human,
    )?;
    let mut r = request(w);
    if mode == "one_case_domain" {
        r["domain"] = json!("risk_reputation");
    }
    let output = rank(&mut e, r.clone())?;
    let start = Instant::now();
    black_box(rank(&mut e, r.clone())?);
    let first_us = start.elapsed().as_secs_f64() * 1e6;
    let mut samples = Vec::new();
    if mode == "independent_control" {
        for _ in 0..5 {
            black_box(rank(&mut e, r.clone())?);
        }
        for _ in 0..n {
            let start = Instant::now();
            let v = rank(&mut e, r.clone())?;
            ensure!(v["model_status"] == "approved_learned");
            black_box(serde_json::to_vec(&v)?);
            samples.push(start.elapsed().as_secs_f64() * 1e6);
        }
        samples.sort_by(f64::total_cmp);
    }
    let timing = if samples.is_empty() {
        Value::Null
    } else {
        json!({"samples":n,"unit":"microseconds","p50":samples[(n as f64*0.5).ceil() as usize-1],"p95":samples[(n as f64*0.95).ceil() as usize-1],"first_after_activation_us":first_us,"warmup":5})
    };
    let rotated = e.with_evolution_trust(
        SigningKey::from_bytes(&[63; 32]).verifying_key(),
        human.verifying_key(),
    )?;
    e = rotated;
    let wrong_key = rank(&mut e, r.clone())?;
    ensure!(wrong_key["abstained"] == true);
    e = e.with_evolution_trust(eval.verifying_key(), human.verifying_key())?;
    e.store.propose(
        w,
        Uuid::new_v4(),
        Uuid::new_v4(),
        json!({"synthetic_pending":true}),
    )?;
    let invalidated = rank(&mut e, r)?;
    ensure!(invalidated["abstained"] == true);
    Ok(
        json!({"probe":mode,"accepted":true,"inference":output,"timing":timing,"rotated_key_abstains":true,"input_epoch_change_abstains":true}),
    )
}
fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    ensure!(args.len() == 2 && args[0] == "--iterations");
    let n = args[1].parse::<usize>()?;
    ensure!((1..=1000).contains(&n));
    let probes = ["independent_control", "one_test_family", "one_case_domain"]
        .into_iter()
        .map(|m| probe(m, n))
        .collect::<Result<Vec<_>>>()?;
    println!(
        "{}",
        json!({"synthetic":true,"personal_accuracy":"not measured","scope":"one process; sequential SQLCipher signed Engine rank; no OS cache eviction; no natural language parsing; fixture signatures only","results":[],"probes":probes})
    );
    Ok(())
}
