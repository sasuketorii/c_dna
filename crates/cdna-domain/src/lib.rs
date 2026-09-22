//! Typed trust-boundary contracts. Deserialize through `from_json` to enforce semantic limits.
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

pub const MAX_REQUEST_BYTES: usize = 256 * 1024;
pub const DEFAULTS_JSON: &str = include_str!("../../../contracts/defaults.json");
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    VaultLocked,
    ScopeDenied,
    SchemaUnsupported,
    InputInvalid,
    RevisionConflict,
    Conflict,
    ModelNotReady,
    ModelInvalidated,
    ConsentRequired,
    BudgetExceeded,
    RateLimited,
    Cancelled,
    ProviderUnavailable,
    InternalError,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DomainError {
    pub code: ErrorCode,
    pub message: String,
}
impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}
impl std::error::Error for DomainError {}
pub type Result<T> = std::result::Result<T, DomainError>;
fn invalid(message: &str) -> DomainError {
    DomainError {
        code: ErrorCode::InputInvalid,
        message: message.into(),
    }
}
fn check(ok: bool, msg: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(invalid(msg)) }
}
macro_rules! enums { ($name:ident { $($v:ident),* }) => {#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize,JsonSchema)] #[serde(rename_all="snake_case")] pub enum $name {$($v),*}}; }
enums!(Mode { Imitate, Advisor });
enums!(Domain {
    ResourceAllocation,
    ProductDelivery,
    CustomerCommercial,
    OrganizationDelegation,
    GrowthStrategy,
    RiskReputation
});
enums!(EvidenceStatus {
    Explicit,
    Inferred,
    Unknown
});
enums!(CaseKind {
    Actual,
    Hypothetical
});
enums!(SourceKind {
    HumanApp,
    Imported,
    AgentProposal,
    AiGenerated
});
enums!(PairwiseOutcome {
    Left,
    Right,
    Tie,
    Insufficient
});
enums!(AcceptabilityOutcome {
    Accept,
    Reject,
    Conditional,
    Insufficient
});
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Money {
    pub amount_minor: i64,
    pub currency: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum FactValue {
    Text(String),
    Number(f64),
    Boolean(bool),
    Money(Money),
    Strings(Vec<String>),
    Null,
}
impl FactValue {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Text(s) => bounded(s, 0, 2048),
            Self::Number(n) => check(n.is_finite() && n.abs() <= 1e12, "number out of range"),
            Self::Money(m) => {
                check(
                    m.amount_minor.unsigned_abs() <= 9_007_199_254_740_991,
                    "money out of range",
                )?;
                check(
                    currency_minor_digits(&m.currency).is_some(),
                    "unsupported currency",
                )
            }
            Self::Strings(v) => {
                check(v.len() <= 32, "too many values")?;
                for s in v {
                    bounded(s, 0, 128)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
/// Explicit supported currencies; amounts are already in the currency's minor unit.
pub fn currency_minor_digits(currency: &str) -> Option<u8> {
    match currency {
        "JPY" | "KRW" => Some(0),
        "USD" | "EUR" | "GBP" | "CAD" | "AUD" | "CHF" | "CNY" => Some(2),
        "KWD" | "BHD" => Some(3),
        _ => None,
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub key: String,
    pub value: FactValue,
    pub evidence_status: EvidenceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub as_of: DateTime<Utc>,
    pub summary: String,
    pub facts: Vec<Fact>,
    pub unknown_fields: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub id: String,
    pub text: String,
    pub attributes: BTreeMap<String, FactValue>,
}
fn yes() -> bool {
    true
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RankRequest {
    pub schema_version: String,
    pub request_id: Uuid,
    pub workspace_id: Uuid,
    pub mode: Mode,
    pub domain: Domain,
    pub context: Context,
    pub candidates: Vec<Candidate>,
    #[serde(default = "yes")]
    pub include_evidence: bool,
    #[serde(default)]
    pub allow_cloud: bool,
}
fn bounded(s: &str, min: usize, max: usize) -> Result<()> {
    check(
        s.chars().count() >= min && s.chars().count() <= max && s.len() <= max,
        "text length out of range",
    )
}
fn key(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}
impl Context {
    pub fn validate(&self) -> Result<()> {
        bounded(&self.summary, 1, 32768)?;
        check(
            serde_json::to_vec(self)
                .map_err(|_| invalid("context encoding"))?
                .len()
                <= 32768,
            "context exceeds byte limit",
        )?;
        check(
            self.facts.len() <= 128 && self.unknown_fields.len() <= 128,
            "too many fields",
        )?;
        let mut unknown = HashSet::new();
        for s in &self.unknown_fields {
            check(
                key(s) && unknown.insert(s),
                "invalid or duplicate unknown field",
            )?;
        }
        let mut seen = HashSet::new();
        for f in &self.facts {
            check(
                key(&f.key) && seen.insert(&f.key),
                "invalid or duplicate fact key",
            )?;
            f.value.validate()?;
            if let Some(u) = &f.unit {
                bounded(u, 1, 32)?;
            }
            if matches!(f.value, FactValue::Number(_)) {
                check(f.unit.is_some(), "numeric fact requires unit")?;
                if matches!(
                    f.key.as_str(),
                    "deadline_pressure"
                        | "asset_importance"
                        | "loss_tolerance"
                        | "customer_impact"
                        | "budget_pressure"
                ) {
                    check(
                        f.unit.as_deref() == Some("ratio"),
                        "normalized feature fact requires ratio unit",
                    )?;
                    if let FactValue::Number(n) = f.value {
                        check(
                            (-1.0..=1.0).contains(&n),
                            "normalized feature fact out of range",
                        )?;
                    }
                }
            }
            if f.evidence_status == EvidenceStatus::Unknown {
                check(
                    matches!(f.value, FactValue::Null),
                    "unknown fact must have null value",
                )?;
            } else {
                check(
                    !unknown.contains(&f.key),
                    "known fact contradicts unknown_fields",
                )?;
            }
        }
        Ok(())
    }
}
fn candidates(v: &[Candidate], min: usize) -> Result<()> {
    check(
        v.len() >= min && v.len() <= 16,
        "candidate count out of range",
    )?;
    let mut ids = HashSet::new();
    for c in v {
        check(
            !c.id.is_empty()
                && c.id.len() <= 64
                && c.id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                && ids.insert(&c.id),
            "invalid or duplicate candidate ID",
        )?;
        bounded(&c.text, 1, 4096)?;
        check(c.attributes.len() <= 64, "too many attributes")?;
        for (k, v) in &c.attributes {
            check(key(k), "invalid attribute key")?;
            v.validate()?;
        }
    }
    Ok(())
}
fn version(v: &str) -> Result<()> {
    if v == "1.0" {
        Ok(())
    } else {
        Err(DomainError {
            code: ErrorCode::SchemaUnsupported,
            message: "unsupported schema version".into(),
        })
    }
}
/// Parse an external JSON envelope before any `Value` conversion can erase duplicate keys.
/// Enforces the request byte bound, standard finite JSON, duplicate keys at every depth,
/// and a single complete JSON value. Typed callers must still validate their contract.
pub fn parse_json(bytes: &[u8]) -> Result<serde_json::Value> {
    parse_json_with_limit(bytes, MAX_REQUEST_BYTES)
}
/// Strict parsing for trusted import boundaries with a separately selected byte limit.
pub fn parse_json_with_limit(bytes: &[u8], limit:usize) -> Result<serde_json::Value> {
    check(
        bytes.len() <= limit,
        "request exceeds byte limit",
    )?;
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let strict =
        StrictJson::deserialize(&mut de).map_err(|_| invalid("invalid or duplicate JSON keys"))?;
    de.end().map_err(|_| invalid("trailing JSON"))?;
    Ok(strict.0)
}
fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_value(parse_json(bytes)?).map_err(|_| invalid("invalid JSON contract"))
}

impl RankRequest {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let r: Self = parse(bytes)?;
        r.validate()?;
        Ok(r)
    }
    pub fn validate(&self) -> Result<()> {
        version(&self.schema_version)?;
        check(
            serde_json::to_vec(self)
                .map_err(|_| invalid("invalid serialization"))?
                .len()
                <= MAX_REQUEST_BYTES,
            "request exceeds byte limit",
        )?;
        self.context.validate()?;
        candidates(&self.candidates, 2)
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "response_type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObservationResponse {
    ChooseOne {
        candidate_id: String,
    },
    ChooseSet {
        candidate_ids: Vec<String>,
    },
    Pairwise {
        left_id: String,
        right_id: String,
        outcome: PairwiseOutcome,
    },
    Acceptability {
        candidate_id: String,
        outcome: AcceptabilityOutcome,
    },
    NeedInformation {
        fields: Vec<String>,
    },
    NoneFit,
    Skip,
    Defer,
}
impl ObservationResponse {
    pub fn validate(&self, cs: &[Candidate]) -> Result<()> {
        let has = |id: &str| cs.iter().any(|c| c.id == id);
        match self {
            Self::ChooseOne { candidate_id } | Self::Acceptability { candidate_id, .. } => {
                check(has(candidate_id), "unknown response candidate")
            }
            Self::ChooseSet { candidate_ids } => {
                let mut seen = HashSet::new();
                check(
                    !candidate_ids.is_empty()
                        && candidate_ids.len() <= 16
                        && candidate_ids.iter().all(|s| has(s) && seen.insert(s)),
                    "invalid selected set",
                )
            }
            Self::Pairwise {
                left_id, right_id, ..
            } => check(
                left_id != right_id && has(left_id) && has(right_id),
                "invalid pair",
            ),
            Self::NeedInformation { fields } => {
                let mut seen = HashSet::new();
                check(
                    !fields.is_empty()
                        && fields.len() <= 128
                        && fields.iter().all(|f| key(f) && seen.insert(f)),
                    "invalid missing fields",
                )
            }
            _ => Ok(()),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceProvenance {
    pub artifact_id: Uuid,
    pub source_kind: SourceKind,
    pub occurred_at: Option<DateTime<Utc>>,
    pub observed_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservationProposal {
    pub schema_version: String,
    pub request_id: Uuid,
    pub workspace_id: Uuid,
    pub family_id: Uuid,
    pub case_kind: CaseKind,
    pub domain: Domain,
    pub context: Context,
    pub candidates: Vec<Candidate>,
    pub response: ObservationResponse,
    pub source: SourceProvenance,
    #[serde(default)]
    pub rationale_explicit: Option<String>,
    #[serde(default)]
    pub reversal_conditions: Vec<String>,
    pub model_exposure: bool,
}
impl ObservationProposal {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        let r: Self = parse(bytes)?;
        r.validate()?;
        Ok(r)
    }
    pub fn validate(&self) -> Result<()> {
        version(&self.schema_version)?;
        check(
            serde_json::to_vec(self)
                .map_err(|_| invalid("invalid serialization"))?
                .len()
                <= MAX_REQUEST_BYTES,
            "request exceeds byte limit",
        )?;
        self.context.validate()?;
        let minimum = if matches!(
            self.response,
            ObservationResponse::ChooseOne { .. }
                | ObservationResponse::ChooseSet { .. }
                | ObservationResponse::Pairwise { .. }
        ) {
            2
        } else {
            1
        };
        candidates(&self.candidates, minimum)?;
        self.response.validate(&self.candidates)?;
        if let Some(s) = &self.rationale_explicit {
            bounded(s, 1, 8192)?;
        }
        check(
            self.reversal_conditions.len() <= 16,
            "too many reversal conditions",
        )?;
        for s in &self.reversal_conditions {
            bounded(s, 1, 2048)?;
        }
        if matches!(
            self.response,
            ObservationResponse::Acceptability {
                outcome: AcceptabilityOutcome::Conditional,
                ..
            }
        ) {
            check(
                !self.reversal_conditions.is_empty(),
                "conditional acceptability requires explicit conditions",
            )?;
        }
        Ok(())
    }
}
/// Internal trusted UI command. Authentication and owner-presence are checked by the application.
#[derive(Debug, Clone)]
pub struct Confirmation {
    pub observation_id: Uuid,
    pub expected_revision: u64,
    pub confirmed_by: Uuid,
    pub confirmed_at: DateTime<Utc>,
}
enums!(Truth {
    True,
    False,
    Unknown
});
enums!(Comparison {
    Eq,
    Lt,
    Lte,
    Gt,
    Gte
});
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum PolicyExpr {
    All {
        args: Vec<PolicyExpr>,
    },
    Any {
        args: Vec<PolicyExpr>,
    },
    Not {
        arg: Box<PolicyExpr>,
    },
    Exists {
        field: String,
    },
    Compare {
        field: String,
        comparison: Comparison,
        value: FactValue,
        unit: Option<String>,
    },
    In {
        field: String,
        values: Vec<FactValue>,
        unit: Option<String>,
    },
}
impl PolicyExpr {
    pub fn evaluate(&self, context: &Context) -> Result<Truth> {
        context.validate()?;
        let mut nodes = 0;
        self.eval(context, 0, &mut nodes)
    }
    fn eval(&self, c: &Context, depth: usize, nodes: &mut usize) -> Result<Truth> {
        *nodes += 1;
        check(depth <= 16 && *nodes <= 256, "policy complexity limit")?;
        match self {
            Self::All { args } | Self::Any { args } => {
                check(!args.is_empty() && args.len() <= 32, "policy branch limit")?;
                let all = matches!(self, Self::All { .. });
                let mut unknown = false;
                let mut decisive = false;
                for a in args {
                    match a.eval(c, depth + 1, nodes)? {
                        Truth::Unknown => unknown = true,
                        Truth::False if all => decisive = true,
                        Truth::True if !all => decisive = true,
                        _ => {}
                    }
                }
                Ok(if decisive {
                    if all { Truth::False } else { Truth::True }
                } else if unknown {
                    Truth::Unknown
                } else if all {
                    Truth::True
                } else {
                    Truth::False
                })
            }
            Self::Not { arg } => Ok(match arg.eval(c, depth + 1, nodes)? {
                Truth::True => Truth::False,
                Truth::False => Truth::True,
                Truth::Unknown => Truth::Unknown,
            }),
            Self::Exists { field } => {
                check(key(field), "invalid policy field")?;
                Ok(
                    if c.unknown_fields.contains(field)
                        || c.facts.iter().any(|f| {
                            &f.key == field
                                && (f.evidence_status != EvidenceStatus::Explicit
                                    || matches!(f.value, FactValue::Null))
                        })
                    {
                        Truth::Unknown
                    } else if c.facts.iter().any(|f| &f.key == field) {
                        Truth::True
                    } else {
                        Truth::Unknown
                    },
                )
            }
            Self::Compare {
                field,
                comparison,
                value,
                unit,
            } => compare_fact(c, field, *comparison, value, unit),
            Self::In {
                field,
                values,
                unit,
            } => {
                check(
                    !values.is_empty() && values.len() <= 32,
                    "policy value limit",
                )?;
                let mut result = Truth::False;
                for v in values {
                    match compare_fact(c, field, Comparison::Eq, v, unit)? {
                        Truth::True => result = Truth::True,
                        Truth::Unknown if result != Truth::True => result = Truth::Unknown,
                        _ => {}
                    }
                }
                Ok(result)
            }
        }
    }
}
fn compare_fact(
    c: &Context,
    field: &str,
    op: Comparison,
    value: &FactValue,
    unit: &Option<String>,
) -> Result<Truth> {
    check(key(field), "invalid policy field")?;
    value.validate()?;
    if op != Comparison::Eq {
        check(
            matches!(value, FactValue::Number(_) | FactValue::Money(_)),
            "ordered policy comparison requires numeric operand",
        )?;
    }
    if let Some(unit) = unit {
        bounded(unit, 1, 32)?;
    }
    if matches!(value, FactValue::Number(_)) {
        check(unit.is_some(), "numeric policy operand requires unit")?;
    }
    let Some(f) = c.facts.iter().find(|f| f.key == field) else {
        return Ok(Truth::Unknown);
    };
    if f.evidence_status != EvidenceStatus::Explicit || matches!(f.value, FactValue::Null) {
        return Ok(Truth::Unknown);
    }
    check(&f.unit == unit, "policy unit mismatch")?;
    check(
        std::mem::discriminant(&f.value) == std::mem::discriminant(value),
        "policy type mismatch",
    )?;
    let ordering = match (&f.value, value) {
        (FactValue::Number(a), FactValue::Number(b)) => a.partial_cmp(b),
        (FactValue::Money(a), FactValue::Money(b)) => {
            check(a.currency == b.currency, "policy currency mismatch")?;
            Some(a.amount_minor.cmp(&b.amount_minor))
        }
        _ => None,
    };
    let b = match op {
        Comparison::Eq => f.value == *value,
        other => {
            let o = ordering
                .ok_or_else(|| invalid("ordered policy comparison requires numeric values"))?;
            match other {
                Comparison::Lt => o.is_lt(),
                Comparison::Lte => o.is_le(),
                Comparison::Gt => o.is_gt(),
                Comparison::Gte => o.is_ge(),
                _ => false,
            }
        }
    };
    Ok(if b { Truth::True } else { Truth::False })
}
enums!(PredictionStatus {
    Predicted,
    NeedsInformation,
    Abstain,
    PolicyConflict
});
enums!(ExecutionAuthorization { None });
enums!(CalibrationStatus {
    InsufficientData,
    Uncalibrated,
    Calibrated
});
enums!(ScoreScope {
    AllPresentedCandidates,
    EligibleCandidates
});
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RankedCandidate {
    pub candidate_id: String,
    pub raw_score: f64,
    pub tie_group: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RankResponse {
    pub schema_version: String,
    pub request_id: Uuid,
    pub status: PredictionStatus,
    pub mode: Mode,
    pub ranking: Vec<RankedCandidate>,
    pub score_scope: ScoreScope,
    pub pairwise_probability: Option<f64>,
    pub top_choice_agreement_estimate: Option<f64>,
    pub calibration_status: CalibrationStatus,
    pub reason_codes: Vec<String>,
    pub evidence: Vec<Uuid>,
    pub explanation: String,
    pub human_review_required: bool,
    pub execution_authorization: ExecutionAuthorization,
}
impl RankResponse {
    pub fn validate(&self, request: &RankRequest) -> Result<()> {
        request.validate()?;
        version(&self.schema_version)?;
        if self.status == PredictionStatus::Predicted {
            check(
                !self.ranking.is_empty(),
                "predicted response requires ranking",
            )?;
        }
        if self.score_scope == ScoreScope::AllPresentedCandidates && !self.ranking.is_empty() {
            check(
                self.ranking.len() == request.candidates.len(),
                "all-candidate score scope omitted candidates",
            )?;
        }
        for code in &self.reason_codes {
            bounded(code, 1, 128)?;
        }
        check(
            self.evidence.iter().collect::<HashSet<_>>().len() == self.evidence.len(),
            "duplicate evidence ID",
        )?;
        check(
            self.request_id == request.request_id && self.mode == request.mode,
            "response request mismatch",
        )?;
        check(
            self.ranking.len() <= request.candidates.len(),
            "too many rankings",
        )?;
        let mut seen = HashSet::new();
        let mut last_score = f64::INFINITY;
        let mut last_group = 0;
        for r in &self.ranking {
            check(
                r.raw_score.is_finite()
                    && request.candidates.iter().any(|c| c.id == r.candidate_id)
                    && seen.insert(&r.candidate_id),
                "invalid ranking candidate or score",
            )?;
            check(
                r.raw_score <= last_score
                    && r.tie_group >= last_group
                    && r.tie_group <= last_group + 1
                    && r.tie_group > 0,
                "inconsistent ranking",
            )?;
            if last_group > 0 {
                check(
                    (r.raw_score == last_score) == (r.tie_group == last_group),
                    "inconsistent tie group",
                )?;
            }
            last_score = r.raw_score;
            last_group = r.tie_group;
        }
        for p in [
            self.pairwise_probability,
            self.top_choice_agreement_estimate,
        ]
        .into_iter()
        .flatten()
        {
            check(
                p.is_finite()
                    && (0.0..=1.0).contains(&p)
                    && self.calibration_status == CalibrationStatus::Calibrated,
                "uncalibrated or invalid probability",
            )?;
        }
        if self.pairwise_probability.is_some() {
            check(
                request.candidates.len() == 2,
                "pairwise probability requires two candidates",
            )?;
        }
        bounded(&self.explanation, 0, 8192)?;
        check(
            self.evidence.len() <= 20 && self.reason_codes.len() <= 32,
            "response limit",
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../contracts/fixtures/rank-valid.json")).unwrap()
    }
    fn accepted(v: &serde_json::Value) -> bool {
        RankRequest::from_json(&serde_json::to_vec(v).unwrap()).is_ok()
    }
    #[test]
    fn strict_public_envelope_and_ratio_units() {
        assert!(parse_json(br#"{"operation":"status","operation":"lock"}"#).is_err());
        assert!(parse_json(br#"{"a":{"b":1,"b":2}}"#).is_err());
        assert!(parse_json(b"{} {}").is_err());
        assert!(parse_json(b"{\"x\":Infinity}").is_err());
        assert!(parse_json(&vec![b' '; MAX_REQUEST_BYTES + 1]).is_err());
        assert_eq!(
            parse_json(br#"{"operation":"status"}"#).unwrap()["operation"],
            "status"
        );
        let mut v = fixture();
        v["context"]["facts"] = serde_json::json!([{"key":"deadline_pressure","value":0.5,"evidence_status":"explicit","unit":"ratio"}]);
        assert!(accepted(&v));
        v["context"]["facts"][0]["unit"] = "day".into();
        assert!(!accepted(&v));
        v["context"]["facts"][0]["unit"] = "ratio".into();
        v["context"]["facts"][0]["value"] = 2.into();
        assert!(!accepted(&v));
    }
    #[test]
    fn inferred_and_null_policy_facts_never_become_definite() {
        let mut request = RankRequest::from_json(&serde_json::to_vec(&fixture()).unwrap()).unwrap();
        let fact = request
            .context
            .facts
            .iter_mut()
            .find(|f| f.key == "deadline_days")
            .unwrap();
        fact.evidence_status = EvidenceStatus::Inferred;
        let expr = PolicyExpr::Compare {
            field: "deadline_days".into(),
            comparison: Comparison::Lt,
            value: FactValue::Number(10.),
            unit: Some("day".into()),
        };
        let exists = PolicyExpr::Exists {
            field: "deadline_days".into(),
        };
        let member = PolicyExpr::In {
            field: "deadline_days".into(),
            values: vec![FactValue::Number(7.)],
            unit: Some("day".into()),
        };
        for policy in [
            expr.clone(),
            exists.clone(),
            member,
            PolicyExpr::Not {
                arg: Box::new(expr.clone()),
            },
            PolicyExpr::All {
                args: vec![expr.clone(), exists.clone()],
            },
            PolicyExpr::Any {
                args: vec![expr.clone(), exists.clone()],
            },
        ] {
            assert_eq!(policy.evaluate(&request.context).unwrap(), Truth::Unknown);
        }
        let fact = request
            .context
            .facts
            .iter_mut()
            .find(|f| f.key == "deadline_days")
            .unwrap();
        fact.evidence_status = EvidenceStatus::Explicit;
        fact.value = FactValue::Null;
        assert_eq!(exists.evaluate(&request.context).unwrap(), Truth::Unknown);
        assert_eq!(expr.evaluate(&request.context).unwrap(), Truth::Unknown);
        let malformed = PolicyExpr::Compare {
            field: "absent".into(),
            comparison: Comparison::Eq,
            value: FactValue::Number(1.),
            unit: None,
        };
        assert!(malformed.evaluate(&request.context).is_err());
    }
    #[test]
    fn comparison_requires_two_presented_candidates() {
        let r = fixture();
        let proposal = serde_json::json!({"schema_version":"1.0","request_id":r["request_id"],"workspace_id":r["workspace_id"],"family_id":r["request_id"],"case_kind":"actual","domain":"product_delivery","context":r["context"],"candidates":[r["candidates"][0]],"response":{"response_type":"choose_one","candidate_id":"repair_first"},"source":{"artifact_id":r["request_id"],"source_kind":"human_app","occurred_at":null,"observed_at":"2026-09-21T10:00:00Z"},"model_exposure":false});
        assert!(ObservationProposal::from_json(&serde_json::to_vec(&proposal).unwrap()).is_err());
        let mut single = proposal;
        single["response"] = serde_json::json!({"response_type":"acceptability","candidate_id":"repair_first","outcome":"accept"});
        assert!(ObservationProposal::from_json(&serde_json::to_vec(&single).unwrap()).is_ok());
    }
    #[test]
    fn response_scope_and_finite_output_boundaries() {
        let request = RankRequest::from_json(&serde_json::to_vec(&fixture()).unwrap()).unwrap();
        let mut response = RankResponse {
            schema_version: "1.0".into(),
            request_id: request.request_id,
            status: PredictionStatus::Predicted,
            mode: request.mode,
            ranking: vec![],
            score_scope: ScoreScope::AllPresentedCandidates,
            pairwise_probability: None,
            top_choice_agreement_estimate: None,
            calibration_status: CalibrationStatus::Uncalibrated,
            reason_codes: vec![],
            evidence: vec![],
            explanation: String::new(),
            human_review_required: true,
            execution_authorization: ExecutionAuthorization::None,
        };
        assert!(response.validate(&request).is_err());
        response.ranking.push(RankedCandidate {
            candidate_id: request.candidates[0].id.clone(),
            raw_score: 1.,
            tie_group: 1,
        });
        assert!(response.validate(&request).is_err());
        response.ranking.push(RankedCandidate {
            candidate_id: request.candidates[1].id.clone(),
            raw_score: 0.,
            tie_group: 2,
        });
        assert!(response.validate(&request).is_ok());
        response.ranking[1].raw_score = f64::NAN;
        assert!(response.validate(&request).is_err());
        response.ranking[1].raw_score = 0.;
        response.evidence = vec![request.request_id, request.request_id];
        assert!(response.validate(&request).is_err());
        response.evidence.clear();
        response.reason_codes = vec!["a".repeat(129)];
        assert!(response.validate(&request).is_err());
    }
    #[test]
    fn ordered_policy_operand_is_checked_without_context() {
        let mut context = RankRequest::from_json(&serde_json::to_vec(&fixture()).unwrap())
            .unwrap()
            .context;
        context.facts.clear();
        context.unknown_fields.clear();
        for comparison in [
            Comparison::Lt,
            Comparison::Lte,
            Comparison::Gt,
            Comparison::Gte,
        ] {
            for value in [
                FactValue::Text("cheap".into()),
                FactValue::Boolean(false),
                FactValue::Strings(vec!["cheap".into()]),
                FactValue::Null,
            ] {
                let policy = PolicyExpr::Compare {
                    field: "cost".into(),
                    comparison,
                    value,
                    unit: None,
                };
                assert!(policy.evaluate(&context).is_err());
                assert!(
                    PolicyExpr::Not {
                        arg: Box::new(policy)
                    }
                    .evaluate(&context)
                    .is_err()
                );
            }
            for (value, unit) in [
                (FactValue::Number(1.), Some("ratio".into())),
                (
                    FactValue::Money(Money {
                        amount_minor: 1,
                        currency: "JPY".into(),
                    }),
                    None,
                ),
            ] {
                assert_eq!(
                    PolicyExpr::Compare {
                        field: "cost".into(),
                        comparison,
                        value,
                        unit
                    }
                    .evaluate(&context)
                    .unwrap(),
                    Truth::Unknown
                );
            }
        }
        assert_eq!(
            PolicyExpr::Compare {
                field: "cost".into(),
                comparison: Comparison::Eq,
                value: FactValue::Text("cheap".into()),
                unit: None
            }
            .evaluate(&context)
            .unwrap(),
            Truth::Unknown
        );
    }
    #[test]
    fn valid_fixture() {
        assert!(accepted(&fixture()));
    }
    #[test]
    fn negative_contracts() {
        for field in ["request_id", "workspace_id"] {
            let mut v = fixture();
            v[field] = "bad".into();
            assert!(!accepted(&v));
        }
        let mut v = fixture();
        v["context"]["as_of"] = "yesterday".into();
        assert!(!accepted(&v));
        let mut v = fixture();
        v["training_eligible"] = true.into();
        assert!(!accepted(&v));
        let mut v = fixture();
        v["mode"] = "policy_check".into();
        assert!(!accepted(&v));
        let mut v = fixture();
        v["candidates"][1]["id"] = v["candidates"][0]["id"].clone();
        assert!(!accepted(&v));
        let mut v = fixture();
        v["context"]["facts"][1]["key"] = v["context"]["facts"][0]["key"].clone();
        assert!(!accepted(&v));
        let mut v = fixture();
        v["context"]["unknown_fields"] = serde_json::json!(["customer_impact"]);
        assert!(!accepted(&v));
        let mut v = fixture();
        v["context"]["facts"][1]["evidence_status"] = "unknown".into();
        assert!(!accepted(&v));
        let mut v = fixture();
        v["context"]["summary"] = "あ".repeat(12000).into();
        assert!(!accepted(&v));
    }
    #[test]
    fn rejects_nonfinite_and_limit() {
        let raw = include_str!("../../../contracts/fixtures/rank-valid.json").replace(
            "\"delivery_strategy\": \"repair_first\"",
            "\"delivery_strategy\": \"repair_first\", \"delivery_strategy\": \"other\"",
        );
        assert!(RankRequest::from_json(raw.as_bytes()).is_err());
        assert!(RankRequest::from_json(b"{\"a\":NaN}").is_err());
        assert!(RankRequest::from_json(&vec![b' '; MAX_REQUEST_BYTES + 1]).is_err());
    }
    #[test]
    fn policy_unknown_and_limits() {
        let r = RankRequest::from_json(&serde_json::to_vec(&fixture()).unwrap()).unwrap();
        let p = PolicyExpr::Exists {
            field: "absent".into(),
        };
        assert_eq!(p.evaluate(&r.context).unwrap(), Truth::Unknown);
        let p = PolicyExpr::Compare {
            field: "deadline_days".into(),
            comparison: Comparison::Lt,
            value: FactValue::Number(9.),
            unit: Some("day".into()),
        };
        assert_eq!(p.evaluate(&r.context).unwrap(), Truth::True);
        let mut p = p;
        for _ in 0..18 {
            p = PolicyExpr::Not { arg: Box::new(p) };
        }
        assert!(p.evaluate(&r.context).is_err());
    }
    #[test]
    fn proposal_rejects_privilege() {
        let r = fixture();
        let mut v = serde_json::json!({"schema_version":"1.0","request_id":r["request_id"],"workspace_id":r["workspace_id"],"family_id":r["request_id"],"case_kind":"actual","domain":"product_delivery","context":r["context"],"candidates":r["candidates"],"response":{"response_type":"skip"},"source":{"artifact_id":r["request_id"],"source_kind":"agent_proposal","occurred_at":null,"observed_at":"2026-09-21T10:00:00Z"},"model_exposure":false});
        assert!(ObservationProposal::from_json(&serde_json::to_vec(&v).unwrap()).is_ok());
        v["verification_state"] = "human_confirmed".into();
        assert!(ObservationProposal::from_json(&serde_json::to_vec(&v).unwrap()).is_err());
    }
}

// Reject duplicate object keys before typed deserialization, including attribute maps.
struct StrictJson(serde_json::Value);
impl<'de> Deserialize<'de> for StrictJson {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = StrictJson;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON without duplicate keys")
            }
            fn visit_bool<E: serde::de::Error>(
                self,
                v: bool,
            ) -> std::result::Result<Self::Value, E> {
                Ok(StrictJson(v.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Self::Value, E> {
                Ok(StrictJson(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Self::Value, E> {
                Ok(StrictJson(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| StrictJson(n.into()))
                    .ok_or_else(|| E::custom("nonfinite number"))
            }
            fn visit_str<E: serde::de::Error>(
                self,
                v: &str,
            ) -> std::result::Result<Self::Value, E> {
                Ok(StrictJson(v.into()))
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(StrictJson(serde_json::Value::Null))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut v = Vec::new();
                while let Some(x) = a.next_element::<StrictJson>()? {
                    v.push(x.0)
                }
                Ok(StrictJson(v.into()))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut m = serde_json::Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if m.contains_key(&k) {
                        return Err(serde::de::Error::custom("duplicate key"));
                    }
                    m.insert(k, a.next_value::<StrictJson>()?.0);
                }
                Ok(StrictJson(m.into()))
            }
        }
        d.deserialize_any(Visitor)
    }
}
