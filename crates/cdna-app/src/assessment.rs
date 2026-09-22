//! Storage contract for user-supplied results, never a questionnaire or scoring engine.
//! These records must remain outside preference learning and general profile exports.
use anyhow::{Result, ensure};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_IMPORT_BYTES: usize = 32 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssessmentResult {
    pub schema_version: String,
    pub instrument: Instrument,
    pub instrument_version: String,
    pub locale: String,
    pub confirmed_by_user: bool,
    pub assessed_on: String,
    pub scores: Vec<ReportedScore>,
    pub source: Source,
    pub consent: Consent,
    pub validity: Validity,
}
/// Names identify imported reports only; no rights-cleared questionnaire pack is enabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Instrument {
    TipiJ,
    #[serde(rename = "bfi_2_j")]
    Bfi2J,
    Ipip,
    HexacoPiR,
    #[serde(rename = "bfi_2")]
    Bfi2,
    #[serde(rename = "neo_pi_3")]
    NeoPi3,
    Mbti,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportedScore {
    pub dimension: String,
    pub value: Option<f64>,
    pub missing_reason: Option<String>,
    pub minimum: f64,
    pub maximum: f64,
    pub unit: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub kind: SourceKind,
    pub reference: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    ManualResult,
    ImportedResult,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Consent {
    pub store_locally: bool,
    pub mcp_read: bool,
    pub external_send: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Validity {
    UserReportedUnverified,
}
fn text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}
impl AssessmentResult {
    /// Pure structural validation; makes no psychometric or provenance authenticity claim.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == "1.0",
            "INPUT_INVALID: unsupported assessment schema"
        );
        ensure!(
            self.consent.store_locally,
            "CONSENT_REQUIRED: local storage consent required"
        );
        // R1 does not expose personality through MCP or external transmission. Fail closed
        // rather than storing permission flags no recipient-scoped policy can enforce.
        ensure!(
            !self.consent.mcp_read && !self.consent.external_send,
            "CONSENT_REQUIRED: personality sharing is not enabled"
        );
        ensure!(text(&self.locale, 35), "INPUT_INVALID: locale required");
        ensure!(
            self.confirmed_by_user,
            "CONSENT_REQUIRED: user confirmation required"
        );
        ensure!(
            text(&self.instrument_version, 128),
            "INPUT_INVALID: instrument version required"
        );
        ensure!(
            self.assessed_on.len() == 10,
            "INPUT_INVALID: date must be YYYY-MM-DD"
        );
        let date = NaiveDate::parse_from_str(&self.assessed_on, "%Y-%m-%d")?;
        ensure!(
            date.format("%Y-%m-%d").to_string() == self.assessed_on,
            "INPUT_INVALID: noncanonical date"
        );
        ensure!(
            text(&self.source.reference, 2048),
            "INPUT_INVALID: source reference required"
        );
        ensure!(
            !self.scores.is_empty() && self.scores.len() <= 64,
            "INPUT_INVALID: provide 1 to 64 reported scores"
        );
        let mut dimensions = HashSet::new();
        for score in &self.scores {
            ensure!(
                text(&score.dimension, 128) && text(&score.unit, 64),
                "INPUT_INVALID: dimension and original unit required"
            );
            ensure!(
                dimensions.insert(score.dimension.trim().to_lowercase()),
                "INPUT_INVALID: duplicate dimension"
            );
            ensure!(
                score.minimum.is_finite() && score.maximum.is_finite(),
                "INPUT_INVALID: finite scores required"
            );
            match score.value {
                Some(value) => ensure!(
                    value.is_finite()
                        && value >= score.minimum
                        && value <= score.maximum
                        && score.missing_reason.is_none(),
                    "INPUT_INVALID: inconsistent reported score"
                ),
                None => ensure!(
                    score
                        .missing_reason
                        .as_deref()
                        .is_some_and(|reason| text(reason, 256)),
                    "INPUT_INVALID: missing score requires reason"
                ),
            }
            ensure!(
                score.minimum < score.maximum,
                "INPUT_INVALID: score outside original scale"
            );
        }
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_IMPORT_BYTES,
            "INPUT_INVALID: assessment too large"
        );
        Ok(())
    }
}
pub fn validate_json(input: &str) -> Result<AssessmentResult> {
    ensure!(
        input.len() <= MAX_IMPORT_BYTES,
        "INPUT_INVALID: assessment too large"
    );
    let result: AssessmentResult = serde_json::from_str(input)?;
    result.validate()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        serde_json::json!({"schema_version":"1.0","instrument":"tipi_j","instrument_version":"user report edition","locale":"ja-JP","confirmed_by_user":true,"assessed_on":"2026-09-22","scores":[{"dimension":"reported trait","value":3.0,"minimum":1.0,"maximum":7.0,"unit":"original report points"}],"source":{"kind":"manual_result","reference":"My saved report"},"consent":{"store_locally":true,"mcp_read":false,"external_send":false},"validity":"user_reported_unverified"})
    }
    fn check(value: serde_json::Value) -> bool {
        validate_json(&value.to_string()).is_ok()
    }
    #[test]
    fn valid_original_bounds_are_inclusive() {
        for n in [1.0, 7.0] {
            let mut v = fixture();
            v["scores"][0]["value"] = serde_json::json!(n);
            assert!(check(v));
        }
    }
    #[test]
    fn consent_and_sharing_fail_closed() {
        for field in ["store_locally", "mcp_read", "external_send"] {
            let mut v = fixture();
            v["consent"][field] = serde_json::json!(field != "store_locally");
            assert!(!check(v));
        }
        let mut v = fixture();
        v.as_object_mut().unwrap().remove("consent");
        assert!(!check(v));
    }
    #[test]
    fn rejects_unsupported_and_inferred_or_validated_claims() {
        for (field, value) in [
            ("instrument", "unknown"),
            ("validity", "clinically_validated"),
            ("schema_version", "2.0"),
        ] {
            let mut v = fixture();
            v[field] = serde_json::json!(value);
            assert!(!check(v));
        }
        let mut v = fixture();
        v["source"]["kind"] = serde_json::json!("conversation_inference");
        assert!(!check(v));
        let mut v = fixture();
        v["questionnaire"] = serde_json::json!([]);
        assert!(!check(v));
    }
    #[test]
    fn rejects_bad_dates_scales_missing_source_and_duplicates() {
        for date in ["2025-02-29", "2026-9-22", "not-a-date"] {
            let mut v = fixture();
            v["assessed_on"] = serde_json::json!(date);
            assert!(!check(v));
        }
        for n in [0.0, 8.0] {
            let mut v = fixture();
            v["scores"][0]["value"] = serde_json::json!(n);
            assert!(!check(v));
        }
        let mut v = fixture();
        v["scores"][0]["maximum"] = serde_json::json!(1);
        assert!(!check(v));
        let mut v = fixture();
        v["source"]["reference"] = serde_json::json!(" ");
        assert!(!check(v));
        let mut v = fixture();
        let score = v["scores"][0].clone();
        v["scores"].as_array_mut().unwrap().push(score);
        assert!(!check(v));
    }
    #[test]
    fn bounded_and_finite() {
        assert!(validate_json(&" ".repeat(MAX_IMPORT_BYTES + 1)).is_err());
        let mut r = validate_json(&fixture().to_string()).unwrap();
        r.scores[0].value = Some(f64::NAN);
        assert!(r.validate().is_err());
        r.scores.clear();
        assert!(r.validate().is_err());
    }
    #[test]
    fn missing_scores_and_confirmation_are_explicit() {
        let mut v = fixture();
        v["scores"][0]["value"] = serde_json::Value::Null;
        assert!(!check(v.clone()));
        v["scores"][0]["missing_reason"] = serde_json::json!("Not reported");
        assert!(check(v.clone()));
        v["scores"][0]["value"] = serde_json::json!(3);
        assert!(!check(v));
        let mut v = fixture();
        v["confirmed_by_user"] = serde_json::json!(false);
        assert!(!check(v));
    }
    #[test]
    fn engine_crud_is_human_only_revision_bound_and_separate_from_training() {
        use crate::{Authority, Engine};
        use serde_json::json;
        use uuid::Uuid;
        let dir = tempfile::tempdir().unwrap();
        let store = cdna_store::Store::create(
            dir.path().join("test.db"),
            &zeroize::Zeroizing::new("a".repeat(64)),
        )
        .unwrap();
        let workspace = Uuid::new_v4();
        store.create_workspace(workspace).unwrap();
        let mut engine = Engine::new(store, dir.path().to_path_buf(), true);
        let id = Uuid::new_v4();
        let save = json!({"operation":"assessment_save","workspace_id":workspace,"id":id,"expected_revision":0,"result":fixture()});
        let agent = Authority::Agent {
            workspace_id: workspace,
            scopes: vec!["personality:read".into(), "judgment:infer".into()],
        };
        assert!(engine.execute(save.clone(), agent.clone()).is_err());
        let mut no_consent = save.clone();
        no_consent["result"]["consent"]["store_locally"] = json!(false);
        assert!(engine.execute(no_consent, Authority::Human).is_err());
        let mut unsupported = save.clone();
        unsupported["result"]["instrument"] = json!("unknown");
        assert!(engine.execute(unsupported, Authority::Human).is_err());
        let saved = engine.execute(save.clone(), Authority::Human).unwrap();
        assert_eq!(saved["revision"], 1);
        assert!(engine.execute(save.clone(), Authority::Human).is_err());
        let list = json!({"operation":"assessments","workspace_id":workspace});
        assert!(engine.execute(list.clone(), agent.clone()).is_err());
        assert_eq!(
            engine.execute(list.clone(), Authority::Human).unwrap()["assessments"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(engine.prepare_train(workspace).is_err());
        assert!(engine.store.list(workspace, 100, 0).unwrap().is_empty());
        let mut edit = save;
        edit["expected_revision"] = json!(1);
        edit["result"]["source"]["reference"] = json!("Corrected report reference");
        assert_eq!(
            engine.execute(edit, Authority::Human).unwrap()["revision"],
            2
        );
        let mut delete = json!({"operation":"assessment_delete","workspace_id":workspace,"id":id,"expected_revision":1});
        assert!(engine.execute(delete.clone(), Authority::Human).is_err());
        delete["expected_revision"] = json!(2);
        assert!(engine.execute(delete.clone(), agent).is_err());
        engine.execute(delete, Authority::Human).unwrap();
        assert!(
            engine.execute(list, Authority::Human).unwrap()["assessments"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(!engine.store.document_tombstones().unwrap().is_empty());
    }
}
