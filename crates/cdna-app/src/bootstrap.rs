//! Portable AI-generated judgement hypotheses, stored through the existing Source lifecycle.
use crate::Authority;
use anyhow::{Result, ensure};
use cdna_store::{Document, DocumentKind, Store};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const PROMPT: &str = include_str!("../../../docs/prompts/bootstrap-profile-ja.md");

pub const MAX_PROFILE_BYTES: usize = 256 * 1024;
pub const MAX_ITEMS: usize = 32;
pub const MAX_TEXT_CHARS: usize = 2048;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapProfile {
    pub schema_version: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub context: PriorContext,
    pub criteria: Vec<Criterion>,
    pub contradictions: Vec<String>,
    pub unknowns: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriorContext {
    pub availability: ContextAvailability,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextAvailability {
    Available,
    Partial,
    Unavailable,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub criterion: String,
    pub priority: Option<u8>,
    pub tradeoffs: Vec<String>,
    pub examples: Vec<String>,
    pub exceptions: Vec<String>,
    pub reversal_conditions: Vec<String>,
    pub evidence: Vec<Evidence>,
    pub confidence: Confidence,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub kind: EvidenceKind,
    pub statement_category: StatementCategory,
    pub text: String,
    pub reference: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    DirectQuote,
    RememberedSummary,
    Inference,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatementCategory {
    ActualDecision,
    ReportedPolicy,
    Ideal,
    AiInference,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confidence {
    /// Provider self-report, never measured calibration or human confirmation.
    pub self_report: Option<f64>,
    pub basis: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapSource {
    pub kind: String,
    pub schema_version: String,
    pub authority: String,
    pub training_eligible: bool,
    pub heldout_eligible: bool,
    pub human_corrected: bool,
    pub share_with_teacher: bool,
    pub profile: BootstrapProfile,
}
#[derive(Debug, Serialize)]
pub struct BootstrapImport {
    pub source_id: Uuid,
    pub source: BootstrapSource,
    pub inserted: usize,
    pub skipped: usize,
    pub preview: bool,
}

fn text(value: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.chars().count() <= MAX_TEXT_CHARS,
        "BOOTSTRAP_TEXT_LIMIT"
    );
    Ok(())
}
fn texts(values: &[String]) -> Result<()> {
    ensure!(values.len() <= MAX_ITEMS, "BOOTSTRAP_ITEM_LIMIT");
    for value in values {
        text(value)?;
    }
    Ok(())
}
/// Strict JSON first: duplicate keys cannot disappear during typed conversion.
/// Null-valued fields remain mandatory in the portable contract.
pub fn parse_profile(bytes: &[u8]) -> Result<BootstrapProfile> {
    let value = cdna_domain::parse_json_with_limit(bytes, MAX_PROFILE_BYTES)?;
    for key in ["provider", "model"] {
        ensure!(value.get(key).is_some(), "BOOTSTRAP_REQUIRED_FIELD");
    }
    if let Some(criteria) = value.get("criteria").and_then(|v| v.as_array()) {
        for c in criteria {
            ensure!(
                c.get("priority").is_some()
                    && c.get("confidence")
                        .and_then(|v| v.get("self_report"))
                        .is_some(),
                "BOOTSTRAP_REQUIRED_FIELD"
            );
            if let Some(evidence) = c.get("evidence").and_then(|v| v.as_array()) {
                for e in evidence {
                    ensure!(e.get("reference").is_some(), "BOOTSTRAP_REQUIRED_FIELD");
                }
            }
        }
    }
    let profile: BootstrapProfile = serde_json::from_value(value)?;
    validate(&profile)?;
    Ok(profile)
}
fn validate(p: &BootstrapProfile) -> Result<()> {
    ensure!(p.schema_version == "1.0", "BOOTSTRAP_VERSION");
    for v in [&p.provider, &p.model].into_iter().flatten() {
        text(v)?;
    }
    texts(&p.context.limitations)?;
    texts(&p.contradictions)?;
    texts(&p.unknowns)?;
    ensure!(p.criteria.len() <= MAX_ITEMS, "BOOTSTRAP_ITEM_LIMIT");
    ensure!(
        !p.criteria.is_empty() || !p.unknowns.is_empty(),
        "BOOTSTRAP_EMPTY_REQUIRES_UNKNOWN"
    );
    for c in &p.criteria {
        text(&c.criterion)?;
        ensure!(
            c.priority.is_none_or(|n| (1..=32).contains(&n)),
            "BOOTSTRAP_PRIORITY"
        );
        for v in [
            &c.tradeoffs,
            &c.examples,
            &c.exceptions,
            &c.reversal_conditions,
        ] {
            texts(v)?;
        }
        ensure!(c.evidence.len() <= MAX_ITEMS, "BOOTSTRAP_ITEM_LIMIT");
        // Empty evidence is permitted only with an explicit unknown declaration.
        ensure!(
            !c.evidence.is_empty() || !p.unknowns.is_empty(),
            "BOOTSTRAP_EMPTY_REQUIRES_UNKNOWN"
        );
        for e in &c.evidence {
            text(&e.text)?;
            if let Some(reference) = &e.reference {
                text(reference)?;
            }
        }
        ensure!(
            c.confidence
                .self_report
                .is_none_or(|n| n.is_finite() && (0.0..=1.0).contains(&n)),
            "BOOTSTRAP_CONFIDENCE"
        );
        text(&c.confidence.basis)?;
    }
    Ok(())
}
fn source(profile: BootstrapProfile, human_corrected: bool) -> BootstrapSource {
    BootstrapSource {
        kind: "bootstrap_profile".into(),
        schema_version: "1.0".into(),
        authority: "ai_generated_hypothesis".into(),
        training_eligible: false,
        heldout_eligible: false,
        human_corrected,
        share_with_teacher: false,
        profile,
    }
}
fn human_workspace(store: &Store, authority: &Authority, workspace: Uuid) -> Result<()> {
    ensure!(matches!(authority, Authority::Human), "HUMAN_REQUIRED");
    ensure!(store.workspaces()?.contains(&workspace), "SCOPE_DENIED");
    Ok(())
}
/// The host authenticates `Authority` and chooses workspace/namespace; neither comes
/// from profile JSON. Canonical typed JSON ignores formatting/key-order differences.
pub fn import_profile(
    store: &mut Store,
    authority: &Authority,
    workspace_id: Uuid,
    namespace: Uuid,
    bytes: &[u8],
    preview: bool,
) -> Result<BootstrapImport> {
    human_workspace(store, authority, workspace_id)?;
    let profile = parse_profile(bytes)?;
    let digest = Sha256::digest(serde_json::to_vec(&(
        "cdna-bootstrap-1",
        workspace_id,
        namespace,
        &profile,
    ))?);
    let mut id = [0u8; 16];
    id.copy_from_slice(&digest[..16]);
    id[6] = (id[6] & 0x0f) | 0x80;
    id[8] = (id[8] & 0x3f) | 0x80;
    let source_id = Uuid::from_bytes(id);
    let source = source(profile, false);
    let (inserted, skipped) = if preview {
        (0, 0)
    } else {
        let result = store.insert_documents_batch(
            DocumentKind::Source,
            workspace_id,
            vec![(source_id, serde_json::to_value(&source)?)],
        )?;
        (result.inserted, result.skipped)
    };
    Ok(BootstrapImport {
        source_id,
        source,
        inserted,
        skipped,
        preview,
    })
}
/// A human correction is an optimistic Source revision, never observation promotion.
/// Retry the original import to retain this corrected revision, not overwrite it.
pub fn revise_profile(
    store: &mut Store,
    authority: &Authority,
    workspace_id: Uuid,
    id: Uuid,
    expected_revision: u64,
    bytes: &[u8],
) -> Result<Document> {
    human_workspace(store, authority, workspace_id)?;
    let profile = parse_profile(bytes)?;
    let current = store.get_document(DocumentKind::Source, workspace_id, id)?;
    let previous: BootstrapSource = serde_json::from_value(current.payload)?;
    ensure!(
        previous.kind == "bootstrap_profile"
            && previous.schema_version == "1.0"
            && previous.authority == "ai_generated_hypothesis"
            && !previous.training_eligible
            && !previous.heldout_eligible,
        "BOOTSTRAP_SOURCE_INVALID"
    );
    Ok(store.put_document(
        DocumentKind::Source,
        workspace_id,
        id,
        expected_revision,
        serde_json::to_value(source(profile, true))?,
    )?)
}

/// Explicit human grant/revoke for the current Source revision. Import and correction
/// default to local-only; correction resets consent because the shared text changed.
pub fn set_teacher_sharing(
    store: &mut Store,
    authority: &Authority,
    workspace_id: Uuid,
    id: Uuid,
    expected_revision: u64,
    enabled: bool,
) -> Result<Document> {
    human_workspace(store, authority, workspace_id)?;
    let current = store.get_document(DocumentKind::Source, workspace_id, id)?;
    let mut payload = checked_source(current.payload)?;
    payload.share_with_teacher = enabled;
    Ok(store.put_document(
        DocumentKind::Source,
        workspace_id,
        id,
        expected_revision,
        serde_json::to_value(payload)?,
    )?)
}
fn checked_source(value: serde_json::Value) -> Result<BootstrapSource> {
    let source: BootstrapSource = serde_json::from_value(value)?;
    ensure!(
        source.kind == "bootstrap_profile"
            && source.schema_version == "1.0"
            && source.authority == "ai_generated_hypothesis"
            && !source.training_eligible
            && !source.heldout_eligible,
        "BOOTSTRAP_SOURCE_INVALID"
    );
    validate(&source.profile)?;
    Ok(source)
}
/// Authenticated callers must separately enforce profile:read and learning:read.
/// Only explicitly shared bootstrap hypotheses, never unrelated source artifacts.
/// Fail closed on scan/output bounds rather than silently omit consent revocations.
pub fn teacher_context(store: &Store, workspace: Uuid) -> Result<serde_json::Value> {
    ensure!(store.workspaces()?.contains(&workspace), "SCOPE_DENIED");
    let started = std::time::Instant::now();
    let mut result = Vec::new();
    let mut total_bytes = 0;
    let mut scanned_bytes = 0;
    let mut offset = 0;
    loop {
        ensure!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "BOOTSTRAP_SCAN_DEADLINE"
        );
        let documents = store.list_documents(DocumentKind::Source, workspace, 100, offset)?;
        let count = documents.len();
        for document in documents {
            scanned_bytes += serde_json::to_vec(&document.payload)?.len();
            ensure!(scanned_bytes <= 64 * 1024 * 1024, "BOOTSTRAP_SCAN_BYTES");
            if document.payload["kind"] != "bootstrap_profile" {
                continue;
            }
            let source = checked_source(document.payload)?;
            if !source.share_with_teacher {
                continue;
            }
            let value = serde_json::json!({"source_id":document.id,"revision":document.revision,"family_id":document.id,"authority":"ai_generated_hypothesis","training_eligible":false,"heldout_eligible":false,"human_corrected":source.human_corrected,"profile":source.profile});
            total_bytes += serde_json::to_vec(&value)?.len();
            ensure!(
                result.len() < 32 && total_bytes <= MAX_PROFILE_BYTES,
                "BOOTSTRAP_CONTEXT_LIMIT"
            );
            result.push(value);
        }
        if count < 100 {
            break;
        }
        offset += 100;
        ensure!(offset < 100_000, "BOOTSTRAP_SCAN_LIMIT");
    }
    Ok(serde_json::Value::Array(result))
}
