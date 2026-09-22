//! Hash-bound, signed, fail-closed model promotion. Host owns keys and transactional persistence.
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
pub type Result<T> = std::result::Result<T, String>;
fn require(ok: bool, msg: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
pub fn signing_bytes<T: Serialize>(v: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(v).map_err(|_| "invalid encoding".into())
}
pub fn hash(bytes: &[u8]) -> String {
    {
        use std::fmt::Write;
        let mut encoded = String::with_capacity(64);
        for byte in Sha256::digest(bytes) {
            write!(&mut encoded, "{byte:02x}").expect("writing String is infallible");
        }
        encoded
    }
}
pub fn content_hash<T: Serialize>(v: &T) -> Result<String> {
    Ok(hash(&signing_bytes(v)?))
}
fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signed<T> {
    pub payload: T,
    pub signature: Vec<u8>,
}
impl<T: Serialize> Signed<T> {
    pub fn verify(&self, key: &VerifyingKey) -> Result<()> {
        let sig = Signature::from_slice(&self.signature).map_err(|_| "invalid signature")?;
        key.verify_strict(&signing_bytes(&self.payload)?, &sig)
            .map_err(|_| "untrusted signature".into())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Candidate,
    Evaluated,
    Approved,
    Active,
    Retired,
    Quarantined,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Split {
    Train,
    Validation,
    Calibration,
    Test,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetCase {
    pub id: Uuid,
    pub family_id: Uuid,
    pub split: Split,
    pub timestamp: i64,
    pub input_hash: String,
    pub answer: String,
    pub independent_human_answer: bool,
    pub model_exposure: bool,
    pub domain: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dataset {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub input_revision: u64,
    pub deletion_epoch: u64,
    pub cases: Vec<DatasetCase>,
}
impl Dataset {
    pub fn validate(&self) -> Result<()> {
        require(
            !self.cases.is_empty() && self.cases.len() <= 20_000,
            "dataset size",
        )?;
        let mut ids = BTreeSet::new();
        let mut families = BTreeMap::new();
        let mut test_families = BTreeSet::new();
        let mut prior_max = [i64::MIN; 3];
        let mut split_min = [i64::MAX; 4];
        let mut split_max = [i64::MIN; 4];
        let mut test_min = i64::MAX;
        for c in &self.cases {
            require(
                ids.insert(c.id)
                    && digest(&c.input_hash)
                    && !c.answer.is_empty()
                    && c.answer.len() <= 64
                    && !c.domain.is_empty()
                    && c.domain.len() <= 64,
                "invalid dataset case",
            )?;
            if let Some(s) = families.insert(c.family_id, c.split) {
                require(s == c.split, "family leakage")?;
            }
            let index = match c.split {
                Split::Train => 0,
                Split::Validation => 1,
                Split::Calibration => 2,
                Split::Test => 3,
            };
            split_min[index] = split_min[index].min(c.timestamp);
            split_max[index] = split_max[index].max(c.timestamp);
            match c.split {
                Split::Train => prior_max[0] = prior_max[0].max(c.timestamp),
                Split::Validation => prior_max[1] = prior_max[1].max(c.timestamp),
                Split::Calibration => prior_max[2] = prior_max[2].max(c.timestamp),
                Split::Test => {
                    require(test_families.insert(c.family_id), "duplicate independent test family")?;
                    require(
                        c.independent_human_answer && !c.model_exposure,
                        "test answer is not independent",
                    )?;
                    test_min = test_min.min(c.timestamp);
                }
            }
        }
        let mut previous = i64::MIN;
        for i in 0..4 {
            if split_max[i] != i64::MIN {
                require(previous < split_min[i], "split chronology leakage")?;
                previous = split_max[i];
            }
        }
        require(
            split_max[0] != i64::MIN && split_min[3] != i64::MAX,
            "train and test splits required",
        )?;
        require(
            prior_max.iter().all(|t| *t < test_min),
            "future information in evaluation",
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    pub feature_version: String,
    pub weights: Vec<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inference: Option<InferenceSupport>,
}
/// Evaluator-attested numerical scope, hashed with the executable weights.
/// Absence preserves legacy artifacts but never authorizes learned inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceSupport {
    pub schema_version: String,
    pub slope: f64,
    pub domains: Vec<String>,
    pub valid_from_unix: i64,
    pub valid_until_unix: i64,
    pub min_margin: f64,
    pub max_margin: f64,
    pub missing_patterns: Vec<u16>,
    pub feature_min: Vec<f64>,
    pub feature_max: Vec<f64>,
}
impl InferenceSupport {
    fn validate(&self) -> Result<()> {
        require(self.schema_version == "1.0"
            && self.slope.is_finite() && self.slope >= 1e-6 && self.slope <= 1e6
            && !self.domains.is_empty() && self.domains.len() <= 6
            && self.domains.iter().all(|d| matches!(d.as_str(), "resource_allocation"|"product_delivery"|"customer_commercial"|"organization_delegation"|"growth_strategy"|"risk_reputation"))
            && self.domains.iter().collect::<BTreeSet<_>>().len() == self.domains.len()
            && self.valid_from_unix >= 0 && self.valid_until_unix > self.valid_from_unix
            && self.min_margin.is_finite() && self.min_margin > 0.0
            && self.max_margin.is_finite() && self.max_margin >= self.min_margin && self.max_margin <= 34e6
            && !self.missing_patterns.is_empty() && self.missing_patterns.len() <= 2048
            && self.missing_patterns.iter().all(|m| *m < 2048)
            && self.missing_patterns.iter().collect::<BTreeSet<_>>().len() == self.missing_patterns.len()
            && self.feature_min.len() == 17 && self.feature_max.len() == 17
            && self.feature_min.iter().zip(&self.feature_max).all(|(lo, hi)| lo.is_finite() && hi.is_finite() && *lo >= -1.0 && *hi <= 1.0 && lo <= hi),
            "invalid inference support")
    }
}
impl ModelConfig {
    fn validate(&self) -> Result<()> {
        if let Some(scope) = &self.inference { scope.validate()?; }
        require(
            self.feature_version == "1.0"
                && self.weights.len() == 17
                && self.weights.iter().all(|x| x.is_finite() && x.abs() <= 1e6),
            "unsupported model configuration",
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub input_revision: u64,
    pub deletion_epoch: u64,
    pub artifact_hash: String,
    pub training_dataset_id: Uuid,
    pub training_dataset_hash: String,
    pub config: ModelConfig,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub choice: Option<String>,
    pub policy_violations: u32,
    pub latency_us: u64,
    pub peak_memory_bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairedCase {
    pub case_id: Uuid,
    pub input_hash: String,
    pub candidate: Outcome,
    pub baseline: Outcome,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationReceipt {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub input_revision: u64,
    pub deletion_epoch: u64,
    pub candidate_id: Uuid,
    pub artifact_hash: String,
    pub baseline_artifact_hash: String,
    pub dataset_id: Uuid,
    pub dataset_hash: String,
    pub evaluator_version: String,
    pub test_previously_exposed: bool,
    pub rows: Vec<PairedCase>,
    pub candidate_learning_seconds: u64,
    pub baseline_learning_seconds: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanApproval {
    pub workspace_id: Uuid,
    pub input_revision: u64,
    pub deletion_epoch: u64,
    pub candidate_id: Uuid,
    pub artifact_hash: String,
    pub receipt_hash: String,
    pub human_id: Uuid,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metrics {
    pub cases: usize,
    pub answered: usize,
    pub correct: usize,
    pub coverage: f64,
    pub agreement: f64,
    pub wilson_lower_95: f64,
    pub p95_latency_us: u64,
    pub peak_memory_bytes: u64,
    pub policy_violations: u64,
}
fn metrics(rows: &[(&PairedCase, &DatasetCase)], candidate: bool) -> Metrics {
    let mut answered = 0;
    let mut correct = 0;
    let mut times = Vec::new();
    let mut memory = 0;
    let mut violations = 0;
    for (r, c) in rows {
        let o = if candidate { &r.candidate } else { &r.baseline };
        if let Some(choice) = &o.choice {
            answered += 1;
            if choice == &c.answer {
                correct += 1;
            }
        }
        times.push(o.latency_us);
        memory = memory.max(o.peak_memory_bytes);
        violations += u64::from(o.policy_violations);
    }
    times.sort_unstable();
    let n = answered as f64;
    let p = if n > 0. { correct as f64 / n } else { 0. };
    let z = 1.959963984540054;
    let wilson = if n > 0. {
        (p + z * z / (2. * n) - z * (p * (1. - p) / n + z * z / (4. * n * n)).sqrt())
            / (1. + z * z / n)
    } else {
        0.
    };
    Metrics {
        cases: rows.len(),
        answered,
        correct,
        coverage: answered as f64 / rows.len().max(1) as f64,
        agreement: p,
        wilson_lower_95: wilson,
        p95_latency_us: times
            .get((times.len() * 95).div_ceil(100).saturating_sub(1))
            .copied()
            .unwrap_or(0),
        peak_memory_bytes: memory,
        policy_violations: violations,
    }
}
fn compare(rows: &[(&PairedCase, &DatasetCase)], absolute: bool) -> Result<Metrics> {
    let c = metrics(rows, true);
    let b = metrics(rows, false);
    if absolute {
        require(
            c.cases >= 200
                && c.answered >= 100
                && c.coverage >= 0.6
                && c.agreement >= 0.9
                && c.wilson_lower_95 >= 0.85,
            "insufficient independent quality",
        )?;
    }
    if absolute {
        let differences: Vec<f64> = rows
            .iter()
            .map(|(r, d)| {
                f64::from(r.candidate.choice.as_ref() == Some(&d.answer))
                    - f64::from(r.baseline.choice.as_ref() == Some(&d.answer))
            })
            .collect();
        let n = differences.len() as f64;
        let mean = differences.iter().sum::<f64>() / n;
        let variance = differences.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
        require(
            mean - 1.959963984540054 * (variance / n).sqrt() >= -0.02,
            "paired comparison uncertainty exceeds margin",
        )?;
    }
    require(
        c.policy_violations == 0
            && c.coverage >= b.coverage
            && c.agreement >= b.agreement
            && c.p95_latency_us <= b.p95_latency_us
            && c.peak_memory_bytes <= b.peak_memory_bytes,
        "baseline regression",
    )?;
    Ok(c)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    candidate: Candidate,
    state: State,
    evaluation: Option<Signed<EvaluationReceipt>>,
    dataset: Option<Dataset>,
    approval: Option<Signed<HumanApproval>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evolution {
    schema_version: String,
    workspace_id: Uuid,
    input_revision: u64,
    deletion_epoch: u64,
    entries: BTreeMap<Uuid, Entry>,
    active: Option<Uuid>,
    #[serde(skip)]
    trusted: bool,
    #[serde(skip)]
    evaluator_key: Option<[u8; 32]>,
}
impl Evolution {
    pub fn new(workspace_id: Uuid, input_revision: u64, deletion_epoch: u64) -> Self {
        Self {
            schema_version: "1.0".into(),
            workspace_id,
            input_revision,
            deletion_epoch,
            entries: BTreeMap::new(),
            active: None,
            trusted: true,
            evaluator_key: None,
        }
    }
    pub fn active(&self) -> Option<Uuid> {
        if self.trusted { self.active } else { None }
    }
    pub fn active_candidate(&self) -> Option<&Candidate> {
        self.active()
            .and_then(|id| self.entries.get(&id))
            .map(|e| &e.candidate)
    }
    /// Only these measured domains belong to the active artifact's evaluated scope.
    pub fn active_domains(&self) -> Vec<&str> {
        let Some(id) = self.active() else {
            return Vec::new();
        };
        self.entries
            .get(&id)
            .and_then(|e| e.dataset.as_ref())
            .map(|d| {
                d.cases
                    .iter()
                    .filter(|c| c.split == Split::Test)
                    .map(|c| c.domain.as_str())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn state(&self, id: Uuid) -> Option<State> {
        self.entries.get(&id).map(|e| e.state)
    }
    fn binding(&self, w: Uuid, r: u64, e: u64) -> Result<()> {
        require(
            w == self.workspace_id && r == self.input_revision && e == self.deletion_epoch,
            "stale or foreign binding",
        )
    }
    pub fn register(&mut self, c: Candidate, artifact: &[u8]) -> Result<()> {
        require(self.trusted, "validate restored state first")?;
        self.binding(c.workspace_id, c.input_revision, c.deletion_epoch)?;
        require(
            self.entries.len() < 128 && !self.entries.contains_key(&c.id),
            "duplicate or excessive candidate",
        )?;
        c.config.validate()?;
        require(
            artifact.len() <= 1_048_576
                && digest(&c.training_dataset_hash)
                && hash(artifact) == c.artifact_hash,
            "artifact mismatch",
        )?;
        let parsed: ModelConfig =
            serde_json::from_slice(artifact).map_err(|_| "unsupported artifact")?;
        require(
            signing_bytes(&parsed)? == artifact
                && signing_bytes(&parsed)? == signing_bytes(&c.config)?,
            "artifact configuration mismatch",
        )?;
        self.entries.insert(
            c.id,
            Entry {
                candidate: c,
                state: State::Candidate,
                evaluation: None,
                dataset: None,
                approval: None,
            },
        );
        Ok(())
    }
    pub fn evaluate(
        &mut self,
        id: Uuid,
        receipt: Signed<EvaluationReceipt>,
        dataset: Dataset,
        key: &VerifyingKey,
    ) -> Result<Metrics> {
        require(self.trusted, "validate restored state first")?;
        if let Some(k) = self.evaluator_key {
            require(k == key.to_bytes(), "evaluator key changed")?;
        }
        if let Some(active) = self.active {
            require(
                self.entries
                    .get(&active)
                    .ok_or("invalid active pointer")?
                    .candidate
                    .artifact_hash
                    == receipt.payload.baseline_artifact_hash,
                "baseline is not current artifact",
            )?;
        }
        let entry = self.entries.get(&id).ok_or("unknown candidate")?;
        require(entry.state == State::Candidate, "immutable evaluation")?;
        let m = self.verify_evaluation(&entry.candidate, &receipt, &dataset, key)?;
        let entry = self.entries.get_mut(&id).unwrap();
        entry.evaluation = Some(receipt);
        entry.dataset = Some(dataset);
        entry.state = State::Evaluated;
        self.evaluator_key = Some(key.to_bytes());
        Ok(m)
    }
    fn verify_evaluation(
        &self,
        c: &Candidate,
        s: &Signed<EvaluationReceipt>,
        d: &Dataset,
        key: &VerifyingKey,
    ) -> Result<Metrics> {
        s.verify(key)?;
        let r = &s.payload;
        self.binding(c.workspace_id, c.input_revision, c.deletion_epoch)?;
        self.binding(r.workspace_id, r.input_revision, r.deletion_epoch)?;
        self.binding(d.workspace_id, d.input_revision, d.deletion_epoch)?;
        d.validate()?;
        require(
            r.candidate_id == c.id
                && r.artifact_hash == c.artifact_hash
                && r.dataset_id == d.id
                && r.dataset_hash == content_hash(d)?
                && c.training_dataset_id == d.id
                && c.training_dataset_hash == r.dataset_hash,
            "evidence binding mismatch",
        )?;
        require(
            digest(&r.baseline_artifact_hash)
                && !r.test_previously_exposed
                && !r.evaluator_version.is_empty()
                && r.evaluator_version.len() <= 128,
            "invalid evaluator provenance",
        )?;
        let tests: BTreeMap<_, _> = d
            .cases
            .iter()
            .filter(|x| x.split == Split::Test)
            .map(|c| (c.id, c))
            .collect();
        require(
            r.rows.len() == tests.len() && r.rows.len() <= 20_000,
            "paired dataset mismatch",
        )?;
        let mut seen = BTreeSet::new();
        let mut rows = Vec::new();
        for p in &r.rows {
            let d = tests.get(&p.case_id).ok_or("unknown evaluation case")?;
            require(
                seen.insert(p.case_id) && p.input_hash == d.input_hash,
                "duplicate or unequal paired input",
            )?;
            for o in [&p.candidate, &p.baseline] {
                require(
                    o.choice
                        .as_ref()
                        .is_none_or(|s| !s.is_empty() && s.len() <= 64)
                        && o.latency_us > 0
                        && o.peak_memory_bytes > 0,
                    "unmeasured outcome",
                )?;
            }
            rows.push((p, *d));
        }
        let m = compare(&rows, true)?;
        let domains: BTreeSet<_> = rows.iter().map(|(_, d)| &d.domain).collect();
        for domain in domains {
            compare(
                &rows
                    .iter()
                    .copied()
                    .filter(|(_, d)| &d.domain == domain)
                    .collect::<Vec<_>>(),
                true,
            )?;
        }
        require(
            r.candidate_learning_seconds > 0
                && r.baseline_learning_seconds > 0
                && r.candidate_learning_seconds <= r.baseline_learning_seconds,
            "unmeasured or regressed learning burden",
        )?;
        Ok(m)
    }
    pub fn approve(
        &mut self,
        id: Uuid,
        approval: Signed<HumanApproval>,
        key: &VerifyingKey,
    ) -> Result<()> {
        require(
            self.trusted && self.evaluator_key.is_some_and(|k| k != key.to_bytes()),
            "independent human key required",
        )?;
        approval.verify(key)?;
        let p = &approval.payload;
        self.binding(p.workspace_id, p.input_revision, p.deletion_epoch)?;
        let entry = self.entries.get(&id).ok_or("unknown candidate")?;
        require(
            entry.state == State::Evaluated
                && p.candidate_id == id
                && p.artifact_hash == entry.candidate.artifact_hash
                && p.receipt_hash
                    == content_hash(entry.evaluation.as_ref().ok_or("missing evaluation")?)?
                && !p.human_id.is_nil(),
            "human approval mismatch",
        )?;
        let entry = self.entries.get_mut(&id).unwrap();
        entry.approval = Some(approval);
        entry.state = State::Approved;
        Ok(())
    }
    pub fn activate(&mut self, id: Uuid) -> Result<()> {
        require(self.trusted, "validate restored state first")?;
        let e = self.entries.get(&id).ok_or("unknown candidate")?;
        self.binding(
            e.candidate.workspace_id,
            e.candidate.input_revision,
            e.candidate.deletion_epoch,
        )?;
        require(
            e.state == State::Approved && e.approval.is_some() && e.evaluation.is_some(),
            "activation not approved",
        )?;
        if let Some(old) = self.active {
            require(
                self.entries
                    .get(&old)
                    .ok_or("invalid active pointer")?
                    .candidate
                    .artifact_hash
                    == e.evaluation
                        .as_ref()
                        .ok_or("missing evaluation")?
                        .payload
                        .baseline_artifact_hash,
                "baseline changed since evaluation",
            )?;
        }
        self.switch_active(id)
    }
    fn switch_active(&mut self, id: Uuid) -> Result<()> {
        if let Some(old) = self.active {
            self.entries
                .get_mut(&old)
                .ok_or("invalid active pointer")?
                .state = State::Retired;
        }
        self.entries.get_mut(&id).unwrap().state = State::Active;
        self.active = Some(id);
        Ok(())
    }
    pub fn rollback(&mut self, id: Uuid) -> Result<()> {
        require(self.trusted, "validate restored state first")?;
        let e = self.entries.get(&id).ok_or("unknown rollback candidate")?;
        self.binding(
            e.candidate.workspace_id,
            e.candidate.input_revision,
            e.candidate.deletion_epoch,
        )?;
        require(
            e.state == State::Retired && e.approval.is_some() && e.evaluation.is_some(),
            "rollback artifact invalid",
        )?;
        self.switch_active(id)
    }
    pub fn invalidate(&mut self, revision: u64, epoch: u64) -> Result<()> {
        require(
            revision >= self.input_revision
                && epoch >= self.deletion_epoch
                && (revision > self.input_revision || epoch > self.deletion_epoch),
            "invalidation must advance",
        )?;
        self.input_revision = revision;
        self.deletion_epoch = epoch;
        for e in self.entries.values_mut() {
            e.state = State::Quarantined;
        }
        self.active = None;
        Ok(())
    }
    pub fn validate_restored(
        &mut self,
        workspace: Uuid,
        revision: u64,
        epoch: u64,
        evaluator: &VerifyingKey,
        human: &VerifyingKey,
    ) -> Result<()> {
        self.trusted = false;
        require(self.schema_version == "1.0", "unsupported evolution schema")?;
        self.binding(workspace, revision, epoch)?;
        require(evaluator != human, "evaluator and human keys must differ")?;
        require(self.entries.len() <= 128, "state size")?;
        let mut active = 0;
        for (id, e) in &self.entries {
            require(*id == e.candidate.id, "candidate identity mismatch")?;
            e.candidate.config.validate()?;
            require(
                content_hash(&e.candidate.config)? == e.candidate.artifact_hash,
                "restored artifact mismatch",
            )?;
            if e.state == State::Quarantined {
                continue;
            }
            self.binding(
                e.candidate.workspace_id,
                e.candidate.input_revision,
                e.candidate.deletion_epoch,
            )?;
            if e.state != State::Candidate {
                self.verify_evaluation(
                    &e.candidate,
                    e.evaluation.as_ref().ok_or("missing receipt")?,
                    e.dataset.as_ref().ok_or("missing dataset")?,
                    evaluator,
                )?;
            }
            if matches!(e.state, State::Approved | State::Active | State::Retired) {
                let a = e.approval.as_ref().ok_or("missing approval")?;
                a.verify(human)?;
                let p = &a.payload;
                require(
                    p.workspace_id == workspace
                        && p.input_revision == revision
                        && p.deletion_epoch == epoch
                        && p.candidate_id == *id
                        && !p.human_id.is_nil()
                        && p.artifact_hash == e.candidate.artifact_hash
                        && p.receipt_hash == content_hash(e.evaluation.as_ref().unwrap())?,
                    "restored approval mismatch",
                )?;
            }
            if e.state == State::Active {
                active += 1;
                require(self.active == Some(*id), "active pointer mismatch")?;
            }
        }
        require(
            active == usize::from(self.active.is_some()),
            "active pointer mismatch",
        )?;
        self.evaluator_key = Some(evaluator.to_bytes());
        self.trusted = true;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    fn signed<T: Serialize>(payload: T, k: &SigningKey) -> Signed<T> {
        let signature = k
            .sign(&signing_bytes(&payload).unwrap())
            .to_bytes()
            .to_vec();
        Signed { payload, signature }
    }
    fn fixture() -> (
        Evolution,
        Candidate,
        Vec<u8>,
        Dataset,
        EvaluationReceipt,
        SigningKey,
        SigningKey,
    ) {
        let w = Uuid::new_v4();
        let config = ModelConfig {
            inference: None,
            feature_version: "1.0".into(),
            weights: vec![0.; 17],
        };
        let bytes = signing_bytes(&config).unwrap();
        let mut cases = vec![DatasetCase {
            id: Uuid::new_v4(),
            family_id: Uuid::new_v4(),
            split: Split::Train,
            timestamp: 1,
            input_hash: hash(b"train"),
            answer: "a".into(),
            independent_human_answer: true,
            model_exposure: false,
            domain: "test".into(),
        }];
        for _ in 0..200 {
            cases.push(DatasetCase {
                id: Uuid::new_v4(),
                family_id: Uuid::new_v4(),
                split: Split::Test,
                timestamp: 2,
                input_hash: hash(b"input"),
                answer: "a".into(),
                independent_human_answer: true,
                model_exposure: false,
                domain: "test".into(),
            });
        }
        let d = Dataset {
            id: Uuid::new_v4(),
            workspace_id: w,
            input_revision: 1,
            deletion_epoch: 0,
            cases,
        };
        let c = Candidate {
            id: Uuid::new_v4(),
            workspace_id: w,
            input_revision: 1,
            deletion_epoch: 0,
            artifact_hash: hash(&bytes),
            training_dataset_id: d.id,
            training_dataset_hash: content_hash(&d).unwrap(),
            config,
        };
        let rows = d
            .cases
            .iter()
            .filter(|x| x.split == Split::Test)
            .map(|x| {
                let o = Outcome {
                    choice: Some("a".into()),
                    policy_violations: 0,
                    latency_us: 1,
                    peak_memory_bytes: 1,
                };
                PairedCase {
                    case_id: x.id,
                    input_hash: x.input_hash.clone(),
                    candidate: o.clone(),
                    baseline: o,
                }
            })
            .collect();
        let r = EvaluationReceipt {
            id: Uuid::new_v4(),
            workspace_id: w,
            input_revision: 1,
            deletion_epoch: 0,
            candidate_id: c.id,
            artifact_hash: c.artifact_hash.clone(),
            baseline_artifact_hash: hash(b"baseline"),
            dataset_id: d.id,
            dataset_hash: content_hash(&d).unwrap(),
            evaluator_version: "test-1".into(),
            test_previously_exposed: false,
            rows,
            candidate_learning_seconds: 1,
            baseline_learning_seconds: 1,
        };
        (
            Evolution::new(w, 1, 0),
            c,
            bytes,
            d,
            r,
            SigningKey::from_bytes(&[7; 32]),
            SigningKey::from_bytes(&[8; 32]),
        )
    }
    #[test]
    fn repeated_test_family_and_underpowered_domain_cannot_promote() {
        for repeated_family in [true, false] {
            let (mut e, mut c, bytes, mut d, mut r, evaluator, _) = fixture();
            if repeated_family {
                let family=d.cases[1].family_id;
                for case in d.cases.iter_mut().filter(|c| c.split==Split::Test) { case.family_id=family; }
            } else {
                d.cases[1].domain="underpowered".into();
            }
            c.training_dataset_hash=content_hash(&d).unwrap();
            r.dataset_hash=c.training_dataset_hash.clone();
            e.register(c.clone(),&bytes).unwrap();
            assert!(e.evaluate(c.id,signed(r,&evaluator),d,&evaluator.verifying_key()).is_err());
            assert!(e.active_candidate().is_none());
        }
    }
    fn approval(e: &Evolution, id: Uuid, k: &SigningKey) -> Signed<HumanApproval> {
        let entry = &e.entries[&id];
        signed(
            HumanApproval {
                workspace_id: e.workspace_id,
                input_revision: e.input_revision,
                deletion_epoch: e.deletion_epoch,
                candidate_id: id,
                artifact_hash: entry.candidate.artifact_hash.clone(),
                receipt_hash: content_hash(entry.evaluation.as_ref().unwrap()).unwrap(),
                human_id: Uuid::new_v4(),
            },
            k,
        )
    }
    #[test]
    fn promotion_restore_and_epoch() {
        let (mut e, c, b, d, r, k, h) = fixture();
        let id = c.id;
        e.register(c, &b).unwrap();
        e.evaluate(id, signed(r, &k), d, &k.verifying_key())
            .unwrap();
        e.approve(id, approval(&e, id, &h), &h.verifying_key())
            .unwrap();
        e.activate(id).unwrap();
        let mut restored: Evolution = serde_json::from_slice(&signing_bytes(&e).unwrap()).unwrap();
        assert!(restored.activate(id).is_err());
        restored
            .validate_restored(e.workspace_id, 1, 0, &k.verifying_key(), &h.verifying_key())
            .unwrap();
        assert_eq!(restored.active(), Some(id));
        restored.invalidate(2, 1).unwrap();
        assert_eq!(restored.state(id), Some(State::Quarantined));
        assert!(restored.rollback(id).is_err());
        assert!(restored.activate(id).is_err());
    }
    #[test]
    fn restored_signed_nil_human_identity_is_rejected() {
        let (mut evolution, candidate, artifact, dataset, receipt, evaluator, human) = fixture();
        let id = candidate.id;
        evolution.register(candidate, &artifact).unwrap();
        evolution
            .evaluate(
                id,
                signed(receipt, &evaluator),
                dataset,
                &evaluator.verifying_key(),
            )
            .unwrap();
        let mut payload = approval(&evolution, id, &human).payload;
        payload.human_id = Uuid::nil();
        let invalid_approval = signed(payload, &human);
        assert!(
            evolution
                .approve(id, invalid_approval.clone(), &human.verifying_key())
                .is_err()
        );
        // A valid host-key signature does not make a missing owner identity
        // valid. Exercise historical state injection, not signature tampering.
        let entry = evolution.entries.get_mut(&id).unwrap();
        entry.approval = Some(invalid_approval);
        entry.state = State::Active;
        evolution.active = Some(id);
        let mut restored: Evolution =
            serde_json::from_slice(&signing_bytes(&evolution).unwrap()).unwrap();
        assert!(
            restored
                .validate_restored(
                    evolution.workspace_id,
                    1,
                    0,
                    &evaluator.verifying_key(),
                    &human.verifying_key()
                )
                .is_err()
        );
        assert!(restored.active_candidate().is_none());
        assert!(restored.activate(id).is_err());
    }
    #[test]
    fn forged_stale_and_unsupported() {
        let (mut e, mut c, b, d, r, k, h) = fixture();
        assert!(e.register(c.clone(), b"altered").is_err());
        c.config.feature_version = "shell".into();
        assert!(e.register(c.clone(), &b).is_err());
        c.config.feature_version = "1.0".into();
        let id = c.id;
        e.register(c, &b).unwrap();
        let mut forged = signed(r.clone(), &k);
        forged.payload.rows[0].candidate.choice = Some("b".into());
        assert!(
            e.evaluate(id, forged, d.clone(), &k.verifying_key())
                .is_err()
        );
        let mut stale = r.clone();
        stale.deletion_epoch = 1;
        assert!(
            e.evaluate(id, signed(stale, &k), d.clone(), &k.verifying_key())
                .is_err()
        );
        e.evaluate(id, signed(r, &k), d, &k.verifying_key())
            .unwrap();
        assert!(
            e.approve(id, approval(&e, id, &k), &k.verifying_key())
                .is_err()
        );
        let mut a = approval(&e, id, &h).payload;
        a.workspace_id = Uuid::new_v4();
        assert!(e.approve(id, signed(a, &h), &h.verifying_key()).is_err());
        assert!(e.activate(id).is_err());
    }
    #[test]
    fn leakage_and_regression() {
        let (mut e, c, b, d, mut r, k, _) = fixture();
        let id = c.id;
        let mut leaked = d.clone();
        leaked.cases[1].family_id = leaked.cases[0].family_id;
        assert_eq!(leaked.validate().unwrap_err(), "family leakage");
        e.register(c, &b).unwrap();
        for row in &mut r.rows {
            row.candidate.latency_us = 100;
        }
        assert_eq!(
            e.evaluate(id, signed(r, &k), d, &k.verifying_key())
                .unwrap_err(),
            "baseline regression"
        );
    }
    #[test]
    fn unsigned_config_and_evidence_content_rejected() {
        assert!(
            serde_json::from_str::<ModelConfig>(
                r#"{"feature_version":"1.0","weights":[],"shell":"run"}"#
            )
            .is_err()
        );
        let (mut e, c, b, d, r, k, h) = fixture();
        let id = c.id;
        e.register(c, &b).unwrap();
        assert!(
            e.evaluate(
                id,
                Signed {
                    payload: r.clone(),
                    signature: vec![]
                },
                d.clone(),
                &k.verifying_key()
            )
            .is_err()
        );
        let mut tampered = d.clone();
        tampered.cases[1].answer = "b".into();
        assert!(
            e.evaluate(id, signed(r.clone(), &k), tampered, &k.verifying_key())
                .is_err()
        );
        let mut bad = r.clone();
        for row in &mut bad.rows {
            row.candidate.choice = None;
        }
        assert!(
            e.evaluate(id, signed(bad, &k), d.clone(), &k.verifying_key())
                .is_err()
        );
        e.evaluate(id, signed(r, &k), d, &k.verifying_key())
            .unwrap();
        let mut stale = approval(&e, id, &h).payload;
        stale.artifact_hash = hash(b"wrong");
        assert!(
            e.approve(id, signed(stale, &h), &h.verifying_key())
                .is_err()
        );
    }
    #[test]
    fn rollback_keeps_approved_binding() {
        let (mut e, c, b, d, r, k, h) = fixture();
        let first = c.id;
        e.register(c.clone(), &b).unwrap();
        e.evaluate(first, signed(r.clone(), &k), d.clone(), &k.verifying_key())
            .unwrap();
        e.approve(first, approval(&e, first, &h), &h.verifying_key())
            .unwrap();
        e.activate(first).unwrap();
        let mut second = c;
        second.id = Uuid::new_v4();
        let sid = second.id;
        let mut receipt = r;
        receipt.candidate_id = sid;
        receipt.baseline_artifact_hash = second.artifact_hash.clone();
        e.register(second, &b).unwrap();
        e.evaluate(sid, signed(receipt, &k), d, &k.verifying_key())
            .unwrap();
        e.approve(sid, approval(&e, sid, &h), &h.verifying_key())
            .unwrap();
        e.activate(sid).unwrap();
        e.rollback(first).unwrap();
        assert_eq!(e.active(), Some(first));
        assert_eq!(e.state(sid), Some(State::Retired));
    }
}
