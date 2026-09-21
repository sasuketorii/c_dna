use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, Error>;
pub const SCHEMA: &str = "1.0";
pub const MAX_REQUEST: usize = 256 * 1024;
pub const MAX_FRAME: usize = 64 * 1024 * 1024;
pub const DOMAINS: [&str; 6] = ["resource_allocation", "product_delivery", "customer_commercial", "organization_delegation", "growth_strategy", "risk_reputation"];

/// Wire errors intentionally contain no SQL, paths, credentials or raw input.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{code}")]
#[serde(deny_unknown_fields)]
pub struct Error {
    pub code: String,
    pub incident_id: String,
}
impl Error {
    pub fn new(code: &str) -> Self { Self { code: code.into(), incident_id: Uuid::new_v4().to_string() } }
}
impl From<rusqlite::Error> for Error { fn from(_: rusqlite::Error) -> Self { Self::new("STORE_ERROR") } }
impl From<serde_json::Error> for Error { fn from(_: serde_json::Error) -> Self { Self::new("INPUT_INVALID") } }
impl From<std::io::Error> for Error { fn from(_: std::io::Error) -> Self { Self::new("IO_ERROR") } }
pub fn ensure(condition: bool, code: &str) -> Result<()> { if condition { Ok(()) } else { Err(Error::new(code)) } }
pub fn now() -> String { Utc::now().to_rfc3339() }
pub fn id() -> String { Uuid::new_v4().to_string() }
pub fn valid_uuid(s: &str) -> bool { Uuid::parse_str(s).is_ok() }
pub fn valid_time(s: &str) -> bool { DateTime::parse_from_rfc3339(s).is_ok() }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub key: String,
    pub value: Value,
    pub evidence_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub as_of: String,
    pub summary: String,
    pub facts: Vec<Fact>,
    pub unknown_fields: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub id: String,
    pub text: String,
    pub attributes: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RankRequest {
    pub schema_version: String,
    pub request_id: String,
    pub workspace_id: String,
    pub mode: String,
    pub domain: String,
    pub context: Context,
    pub candidates: Vec<Candidate>,
    #[serde(default = "default_true")]
    pub include_evidence: bool,
    #[serde(default)]
    pub allow_cloud: bool,
}
pub fn default_true() -> bool { true }
fn schema_for(name: &str) -> Result<&'static jsonschema::Validator> {
    static RANK: OnceLock<jsonschema::Validator> = OnceLock::new();
    static OBS: OnceLock<jsonschema::Validator> = OnceLock::new();
    let (cell, raw) = match name {
        "rank" => (&RANK, include_str!("../../../contracts/schemas/rank-options.schema.json")),
        "observation" => (&OBS, include_str!("../../../contracts/schemas/observation-proposal.schema.json")),
        _ => return Err(Error::new("SCHEMA_UNSUPPORTED")),
    };
    if let Some(v) = cell.get() { return Ok(v); }
    let schema: Value = serde_json::from_str(raw)?;
    let validator = jsonschema::options().should_validate_formats(true).build(&schema).map_err(|_| Error::new("CONTRACT_ERROR"))?;
    let _ = cell.set(validator);
    cell.get().ok_or_else(|| Error::new("CONTRACT_ERROR"))
}
pub fn validate_schema(name: &str, value: &Value) -> Result<()> {
    finite(value, 0)?;
    ensure(schema_for(name)?.is_valid(value), "INPUT_INVALID")
}
pub fn finite(value: &Value, depth: usize) -> Result<()> {
    ensure(depth <= 24, "INPUT_TOO_DEEP")?;
    match value {
        Value::Number(n) => ensure(n.as_f64().is_some_and(f64::is_finite), "INPUT_INVALID")?,
        Value::Array(a) => for v in a { finite(v, depth + 1)?; },
        Value::Object(o) => for v in o.values() { finite(v, depth + 1)?; },
        _ => (),
    };
    Ok(())
}
impl RankRequest {
    pub fn parse(value: Value) -> Result<Self> {
        validate_schema("rank", &value)?;
        let r: Self = serde_json::from_value(value)?;
        r.validate()?;
        Ok(r)
    }
    pub fn validate(&self) -> Result<()> {
        ensure(self.schema_version == SCHEMA, "SCHEMA_UNSUPPORTED")?;
        ensure(valid_uuid(&self.request_id) && valid_uuid(&self.workspace_id), "INPUT_INVALID")?;
        ensure(["imitate", "advisor"].contains(&self.mode.as_str()), "INPUT_INVALID")?;
        ensure(DOMAINS.contains(&self.domain.as_str()), "UNSUPPORTED_DOMAIN")?;
        ensure((2..=16).contains(&self.candidates.len()), "INPUT_INVALID")?;
        self.context.validate()?;
        validate_candidates(&self.candidates)?;
        Ok(())
    }
}
impl Context {
    pub fn validate(&self) -> Result<()> {
        ensure(valid_time(&self.as_of) && !self.summary.trim().is_empty() && self.summary.len() <= 32768, "INPUT_INVALID")?;
        ensure(self.facts.len() <= 128 && self.unknown_fields.len() <= 128, "INPUT_INVALID")?;
        let mut seen = BTreeSet::new();
        let unknown: BTreeSet<_> = self.unknown_fields.iter().collect();
        ensure(unknown.len() == self.unknown_fields.len(), "INPUT_INVALID")?;
        for f in &self.facts {
            ensure(seen.insert(&f.key) && valid_key(&f.key), "INPUT_INVALID")?;
            ensure(["explicit", "inferred", "unknown"].contains(&f.evidence_status.as_str()), "INPUT_INVALID")?;
            if f.evidence_status == "unknown" || unknown.contains(&f.key) { ensure(f.value.is_null(), "INPUT_INVALID")?; }
            validate_fact_value(&f.value)?;
            if let Some(u) = &f.unit { ensure(u.len() <= 32, "INPUT_INVALID")?; }
        }
        Ok(())
    }
}
pub fn valid_key(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.as_bytes()[0].is_ascii_lowercase()
        && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}
pub fn validate_fact_value(value: &Value) -> Result<()> {
    finite(value, 0)?;
    match value {
        Value::String(s) => ensure(s.chars().count() <= 2048, "INPUT_INVALID"),
        Value::Number(n) => ensure(n.as_f64().is_some_and(|x| x.abs() <= 1e12), "INPUT_INVALID"),
        Value::Bool(_) | Value::Null => Ok(()),
        Value::Array(a) => ensure(a.len() <= 32 && a.iter().all(|x| x.as_str().is_some_and(|s| s.chars().count() <= 128)), "INPUT_INVALID"),
        Value::Object(o) => {
            ensure(o.len() == 2 && o.contains_key("amount_minor") && o.contains_key("currency"), "INPUT_INVALID")?;
            let amount = o["amount_minor"].as_i64().ok_or_else(|| Error::new("INPUT_INVALID"))?;
            ensure(amount.unsigned_abs() <= 9_007_199_254_740_991, "INPUT_INVALID")?;
            ensure(o["currency"].as_str().is_some_and(|c| ["JPY","USD","EUR","GBP","KRW","CNY"].contains(&c)), "UNSUPPORTED_CURRENCY")
        }
    }
}
pub fn validate_candidates(candidates: &[Candidate]) -> Result<()> {
    let mut ids = BTreeSet::new();
    for c in candidates {
        ensure(!c.id.is_empty() && c.id.len() <= 64 && c.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') && ids.insert(&c.id), "INPUT_INVALID")?;
        ensure(!c.text.trim().is_empty() && c.text.chars().count() <= 4096 && c.attributes.len() <= 64, "INPUT_INVALID")?;
        for (k,v) in &c.attributes { ensure(valid_key(k), "INPUT_INVALID")?; validate_fact_value(v)?; }
    }
    Ok(())
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub kind: String,
    pub selected_ids: Vec<String>,
    #[serde(default)] pub value: Option<String>,
    #[serde(default)] pub note: String,
    #[serde(default)] pub reversal_condition: String,
}
impl Response {
    pub fn validate(&self, candidates: &[Candidate], order: &[String]) -> Result<()> {
        ensure(["choose_one","choose_set","pairwise","acceptability","need_information","none_fit","skip","defer"].contains(&self.kind.as_str()), "INVALID_RESPONSE")?;
        ensure(self.note.len() <= 16384 && self.reversal_condition.len() <= 8192, "INPUT_TOO_LARGE")?;
        let ids: BTreeSet<_> = candidates.iter().map(|c| &c.id).collect();
        let chosen: BTreeSet<_> = self.selected_ids.iter().collect();
        ensure(chosen.len() == self.selected_ids.len() && chosen.is_subset(&ids), "INVALID_RESPONSE")?;
        ensure(order.len() == ids.len() && order.iter().collect::<BTreeSet<_>>() == ids, "INVALID_EXPOSURE")?;
        match self.kind.as_str() {
            "choose_one" => ensure(chosen.len() == 1 && self.value.is_none(), "INVALID_RESPONSE"),
            "choose_set" => ensure(!chosen.is_empty() && self.value.is_none(), "INVALID_RESPONSE"),
            "pairwise" => {
                ensure(candidates.len() == 2, "INVALID_RESPONSE")?;
                match self.value.as_deref() {
                    Some("left") => ensure(self.selected_ids == vec![order[0].clone()], "INVALID_RESPONSE"),
                    Some("right") => ensure(self.selected_ids == vec![order[1].clone()], "INVALID_RESPONSE"),
                    Some("tie" | "insufficient") => ensure(chosen.is_empty(), "INVALID_RESPONSE"),
                    _ => Err(Error::new("INVALID_RESPONSE")),
                }
            },
            "acceptability" => ensure(candidates.len() == 1 && chosen.is_empty() && matches!(self.value.as_deref(), Some("accept" | "reject" | "conditional" | "insufficient")), "INVALID_RESPONSE"),
            _ => ensure(chosen.is_empty() && self.value.is_none(), "INVALID_RESPONSE"),
        }
    }
    pub fn learnable(&self) -> bool { matches!(self.kind.as_str(), "choose_one" | "choose_set" | "pairwise") && !self.selected_ids.is_empty() }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub kind: String,
    pub reference: String,
    #[serde(default)] pub quote: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationProposal {
    pub schema_version: String,
    pub request_id: String,
    pub workspace_id: String,
    pub case_kind: String,
    pub family_id: String,
    pub domain: String,
    pub context: Context,
    pub candidates: Vec<Candidate>,
    #[serde(default)] pub response: Option<Response>,
    pub source: Source,
    #[serde(default)] pub model_exposure: bool,
}
impl ObservationProposal {
    pub fn parse(value: Value) -> Result<Self> {
        validate_schema("observation", &value)?;
        let proposal: Self = serde_json::from_value(value)?;
        proposal.context.validate()?;
        validate_candidates(&proposal.candidates)?;
        ensure(valid_uuid(&proposal.family_id), "INPUT_INVALID")?;
        if let Some(r) = &proposal.response {
            r.validate(&proposal.candidates, &proposal.candidates.iter().map(|c|c.id.clone()).collect::<Vec<_>>())?;
        }
        Ok(proposal)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseRecord {
    pub id: String,
    pub workspace_id: String,
    pub family_id: String,
    pub revision: i64,
    pub domain: String,
    pub context: Context,
    pub candidates: Vec<Candidate>,
    pub case_kind: String,
    pub origin: String,
    pub source: Source,
    pub verification_state: String,
    pub response: Option<Response>,
    pub model_exposure: bool,
    pub created_at: String,
    pub confirmed_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exposure {
    pub id: String,
    pub case_id: String,
    pub workspace_id: String,
    pub revision: i64,
    pub display_order: Vec<String>,
    pub shown_at: String,
    pub query_policy: String,
    pub model_exposure: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerRequest {
    pub request_id: String,
    pub workspace_id: String,
    pub case_id: String,
    pub exposure_id: String,
    pub expected_revision: i64,
    pub response: Response,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub is_demo: bool,
    pub deletion_epoch: i64,
    pub policy_revision: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedCandidate { pub candidate_id: String, pub raw_score: f64, pub tie_group: usize }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prediction {
    pub schema_version: String,
    pub request_id: String,
    pub status: String,
    pub mode: String,
    pub ranking: Vec<RankedCandidate>,
    pub score_scope: String,
    pub pairwise_probability: Option<f64>,
    pub top_choice_agreement_estimate: Option<f64>,
    pub calibration_status: String,
    pub reason_codes: Vec<String>,
    pub missing_information: Vec<Value>,
    pub evidence: Vec<Value>,
    pub excluded_candidates: Vec<Value>,
    pub explanation: String,
    pub advisor: Option<Value>,
    pub versions: Value,
    pub timing_ms: Value,
    pub human_review_required: bool,
    pub execution_authorization: String,
}
