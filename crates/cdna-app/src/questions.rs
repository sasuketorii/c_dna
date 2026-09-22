//! Deterministic, local question scheduling, policy version 1.0.
//!
//! Caller supplies validated scenarios and complete exposure history (including unanswered
//! presentations). No text is interpreted as a fact. Selection is a heuristic, not a
//! calibrated confidence or an optimal information-gain claim. Evaluation pools must be
//! kept separate by the caller. Semantic leading-question review belongs to the generator.
use anyhow::{Result, ensure};
use cdna_domain::{
    AcceptabilityOutcome, Domain, EvidenceStatus, ObservationProposal, ObservationResponse,
    PairwiseOutcome, RankRequest, SourceKind,
};
use cdna_inference::{DecisionScorer, LinearScorer, extract_features};
use cdna_store::{Record, Status};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use uuid::Uuid;

pub const MAX_POOL: usize = 128;
pub const MAX_HISTORY: usize = 1000;
pub const POLICY_VERSION: &str = "1.0";
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionCategory {
    Gap,
    Representative,
    Exploration,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionCandidate {
    pub id: Uuid,
    pub family_id: Uuid,
    pub question: RankRequest,
    pub synthetic_scenario: bool,
    pub source: String,
    pub category: QuestionCategory,
    pub owner_declared_importance: Option<f64>,
    pub observed_frequency: Option<f64>,
    pub estimated_answer_seconds: u16,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionExposure {
    pub question_id: Uuid,
    pub family_id: Uuid,
    pub domain: Domain,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionSelectionRequest {
    pub workspace_id: Uuid,
    pub pool_id: String,
    pub candidates: Vec<QuestionCandidate>,
    pub exposures: Vec<QuestionExposure>,
    pub limit: usize,
    pub seed: u64,
    pub session_offset: u64,
    pub fixed_domain: Option<Domain>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerSemantics {
    Preference,
    Tie,
    Insufficient,
    NoneFit,
    Skip,
    Defer,
}
/// These outcomes never manufacture a winner. NoneFit concerns the offered set;
/// Skip is nonresponse; Defer is a recorded decision to postpone, not missing data.
pub fn answer_semantics(response: &ObservationResponse) -> AnswerSemantics {
    match response {
        ObservationResponse::NeedInformation { .. }
        | ObservationResponse::Pairwise {
            outcome: PairwiseOutcome::Insufficient,
            ..
        }
        | ObservationResponse::Acceptability {
            outcome: AcceptabilityOutcome::Insufficient,
            ..
        } => AnswerSemantics::Insufficient,
        ObservationResponse::Pairwise {
            outcome: PairwiseOutcome::Tie,
            ..
        } => AnswerSemantics::Tie,
        ObservationResponse::NoneFit => AnswerSemantics::NoneFit,
        ObservationResponse::Skip => AnswerSemantics::Skip,
        ObservationResponse::Defer => AnswerSemantics::Defer,
        _ => AnswerSemantics::Preference,
    }
}
#[derive(Debug, Serialize)]
pub struct QuestionSignals {
    pub independent_domain_cases: usize,
    pub independent_context_cases: usize,
    pub raw_top_two_margin: Option<f64>,
    pub owner_declared_importance: Option<f64>,
    pub observed_frequency: Option<f64>,
    pub estimated_answer_seconds: u16,
    pub gap_reasons: Vec<&'static str>,
    pub prior_outcomes: Vec<AnswerSemantics>,
}
#[derive(Debug, Serialize)]
pub struct SelectedQuestion {
    pub candidate: QuestionCandidate,
    pub priority: f64,
    pub signals: QuestionSignals,
}
#[derive(Debug, Serialize)]
pub struct QuestionSelection {
    pub policy_version: &'static str,
    pub pool_id: String,
    pub pool_question_ids: Vec<Uuid>,
    pub seed: u64,
    /// No random sampling or post-hoc propensity estimate is performed.
    pub selection_probability: Option<f64>,
    pub selected: Vec<SelectedQuestion>,
    pub status: &'static str,
    pub excluded_count: usize,
}
fn region(context: &cdna_domain::Context) -> Result<String> {
    // Exact structured evidence only: prose, timestamps and inferred facts cannot
    // create known context coverage. Canonical sorting makes fact order irrelevant.
    let mut facts: Vec<_> = context
        .facts
        .iter()
        .filter(|f| f.evidence_status == EvidenceStatus::Explicit)
        .map(serde_json::to_string)
        .collect::<std::result::Result<_, _>>()?;
    facts.sort();
    Ok(serde_json::to_string(&facts)?)
}
fn tie_key(id: Uuid, seed: u64) -> u64 {
    // Specified stable FNV-1a rather than process-random HashMap ordering.
    id.as_bytes()
        .iter()
        .chain(seed.to_le_bytes().iter())
        .fold(0xcbf29ce484222325, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
        })
}
/// Bounded scheduling over a complete supplied snapshot. Oversize inputs fail rather
/// than silently truncating exposure history. Model is local and cloned once; all
/// candidate features are scored in a single batch using the shared inference crate.
pub fn select_questions(
    request: &QuestionSelectionRequest,
    records: &[Record],
    model: Option<&LinearScorer>,
) -> Result<QuestionSelection> {
    ensure!(
        request.candidates.len() <= MAX_POOL
            && records.len() <= MAX_HISTORY
            && request.exposures.len() <= MAX_HISTORY
            && (1..=10).contains(&request.limit),
        "INPUT_INVALID: question bounds"
    );
    ensure!(
        !request.pool_id.trim().is_empty() && request.pool_id.len() <= 128,
        "INPUT_INVALID: pool identity"
    );
    let mut bytes = 0usize;
    let mut ids = HashSet::new();
    for c in &request.candidates {
        ensure!(
            ids.insert(c.id)
                && c.question.workspace_id == request.workspace_id
                && !c.question.allow_cloud,
            "INPUT_INVALID: question identity/scope"
        );
        ensure!(
            !c.source.trim().is_empty()
                && c.source.len() <= 128
                && (1..=600).contains(&c.estimated_answer_seconds),
            "INPUT_INVALID: source/burden"
        );
        ensure!(
            [c.owner_declared_importance, c.observed_frequency]
                .into_iter()
                .flatten()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(&v)),
            "INPUT_INVALID: question signal"
        );
        c.question.validate()?;
        bytes += serde_json::to_vec(c)?.len();
        ensure!(
            bytes <= 8 * 1024 * 1024,
            "INPUT_INVALID: question snapshot bytes"
        );
    }
    let mut excluded_ids: HashSet<Uuid> = request.exposures.iter().map(|e| e.question_id).collect();
    let mut excluded_families: HashSet<Uuid> =
        request.exposures.iter().map(|e| e.family_id).collect();
    let mut confirmed = Vec::new();
    let mut seen_records = HashSet::new();
    for r in records {
        ensure!(
            r.workspace_id == request.workspace_id && seen_records.insert(r.id),
            "INPUT_INVALID: history scope/duplicate"
        );
        bytes += serde_json::to_vec(&r.payload)?.len();
        ensure!(
            bytes <= 8 * 1024 * 1024,
            "INPUT_INVALID: question snapshot bytes"
        );
        if r.status != Status::Confirmed {
            continue;
        }
        let p: ObservationProposal = serde_json::from_value(r.payload.clone())?;
        p.validate()?;
        ensure!(
            p.workspace_id == request.workspace_id,
            "INPUT_INVALID: history payload scope"
        );
        excluded_ids.insert(p.request_id);
        excluded_families.insert(p.family_id);
        // Contaminated labels cannot supply coverage, but still count as exposure.
        if p.source.source_kind != SourceKind::AiGenerated && !p.model_exposure {
            let context_region = region(&p.context)?;
            confirmed.push((p, context_region));
        }
    }
    let mut rows = Vec::new();
    let mut offsets = Vec::new();
    if model.is_some() {
        for c in &request.candidates {
            let start = rows.len();
            let context = serde_json::to_value(&c.question.context)?;
            let features = c
                .question
                .candidates
                .iter()
                .map(|choice| {
                    let value = serde_json::to_value(choice).map_err(|e| e.to_string())?;
                    extract_features(&context, &value)
                })
                .collect::<std::result::Result<Vec<_>, _>>();
            match features {
                Ok(features) => {
                    rows.extend(features);
                    offsets.push(Some(start..rows.len()));
                }
                Err(_) => offsets.push(None),
            }
        }
    }
    let scores = if !rows.is_empty() {
        Some(
            model
                .expect("rows require model")
                .clone()
                .score(&rows)
                .map_err(anyhow::Error::msg)?
                .scores,
        )
    } else {
        None
    };
    let mut ranked = Vec::new();
    for (index, c) in request.candidates.iter().enumerate() {
        if excluded_ids.contains(&c.id)
            || excluded_families.contains(&c.family_id)
            || request.fixed_domain.is_some_and(|d| d != c.question.domain)
        {
            continue;
        }
        let context_region = region(&c.question.context)?;
        let domain_cases: BTreeSet<_> = confirmed
            .iter()
            .filter(|(p, _)| {
                p.domain == c.question.domain
                    && !matches!(
                        answer_semantics(&p.response),
                        AnswerSemantics::Skip
                            | AnswerSemantics::Insufficient
                            | AnswerSemantics::NoneFit
                    )
            })
            .map(|(p, _)| p.family_id)
            .collect();
        let matches: Vec<_> = confirmed
            .iter()
            .filter(|(p, r)| p.domain == c.question.domain && *r == context_region)
            .collect();
        let context_cases: BTreeSet<_> = matches
            .iter()
            .filter(|(p, _)| domain_cases.contains(&p.family_id))
            .map(|(p, _)| p.family_id)
            .collect();
        let outcomes = matches
            .iter()
            .map(|(p, _)| answer_semantics(&p.response))
            .collect::<Vec<_>>();
        // Conflict/drift only compare the exact same candidate set (IDs included),
        // so unrelated questions in the same context cannot fabricate contradiction.
        let comparable: Vec<_> = matches
            .iter()
            .filter(|(p, _)| {
                p.candidates == c.question.candidates
                    && matches!(
                        answer_semantics(&p.response),
                        AnswerSemantics::Preference | AnswerSemantics::Tie | AnswerSemantics::Defer
                    )
            })
            .collect();
        let responses: BTreeSet<_> = comparable
            .iter()
            .map(|(p, _)| serde_json::to_string(&p.response))
            .collect::<std::result::Result<_, _>>()?;
        let conflict = responses.len() > 1;
        let drift = conflict
            && comparable
                .iter()
                .map(|(p, _)| p.source.observed_at)
                .collect::<BTreeSet<_>>()
                .len()
                > 1;
        let mut reasons = Vec::new();
        if domain_cases.is_empty() {
            reasons.push("domain_count_gap");
        }
        if context_cases.is_empty() {
            reasons.push("unknown_context");
        }
        if conflict {
            reasons.push("conflicting_choices");
        }
        if drift {
            reasons.push("temporal_response_change");
        }
        if !c.question.context.unknown_fields.is_empty()
            || c.question
                .context
                .facts
                .iter()
                .any(|f| f.evidence_status != EvidenceStatus::Explicit)
            || outcomes.contains(&AnswerSemantics::Insufficient)
        {
            reasons.push("missing_information");
        }
        if outcomes.contains(&AnswerSemantics::NoneFit) {
            reasons.push("candidate_set_repair");
        }
        let margin = scores.as_ref().and_then(|s| {
            offsets[index].as_ref().map(|range| {
                let mut values = s[range.clone()].to_vec();
                values.sort_by(|a, b| b.total_cmp(a));
                values[0] - values[1]
            })
        });
        if model.is_some() && margin.is_none() {
            reasons.push("model_features_unavailable");
        }
        let priority = 4.0 / (1.0 + domain_cases.len() as f64)
            + 3.0 / (1.0 + context_cases.len() as f64)
            + margin.map_or(0.0, |m| 2.0 / (1.0 + m))
            + 2.0 * c.owner_declared_importance.unwrap_or(0.0)
            + c.observed_frequency.unwrap_or(0.0)
            + if conflict { 2.0 } else { 0.0 }
            + if drift { 1.0 } else { 0.0 }
            + if reasons.contains(&"missing_information") {
                1.0
            } else {
                0.0
            }
            + if reasons.contains(&"candidate_set_repair") {
                1.0
            } else {
                0.0
            }
            - f64::from(c.estimated_answer_seconds) / 120.0;
        ranked.push(SelectedQuestion {
            candidate: c.clone(),
            priority,
            signals: QuestionSignals {
                independent_domain_cases: domain_cases.len(),
                independent_context_cases: context_cases.len(),
                raw_top_two_margin: margin,
                owner_declared_importance: c.owner_declared_importance,
                observed_frequency: c.observed_frequency,
                estimated_answer_seconds: c.estimated_answer_seconds,
                gap_reasons: reasons,
                prior_outcomes: outcomes,
            },
        });
    }
    let excluded_count = request.candidates.len() - ranked.len();
    let mut selected = Vec::new();
    let schedule = [
        QuestionCategory::Gap,
        QuestionCategory::Representative,
        QuestionCategory::Gap,
        QuestionCategory::Exploration,
        QuestionCategory::Gap,
        QuestionCategory::Representative,
        QuestionCategory::Gap,
        QuestionCategory::Exploration,
        QuestionCategory::Gap,
        QuestionCategory::Representative,
    ];
    while selected.len() < request.limit && !ranked.is_empty() {
        let category = schedule[((request.session_offset % 10) as usize + selected.len()) % 10];
        let domains = [
            Domain::ResourceAllocation,
            Domain::ProductDelivery,
            Domain::CustomerCommercial,
            Domain::OrganizationDelegation,
            Domain::GrowthStrategy,
            Domain::RiskReputation,
        ];
        let counts = domains.map(|d| {
            request.exposures.iter().filter(|e| e.domain == d).count()
                + selected
                    .iter()
                    .filter(|q: &&SelectedQuestion| q.candidate.question.domain == d)
                    .count()
        });
        let exposure_count = |d| {
            counts[domains
                .iter()
                .position(|domain| *domain == d)
                .expect("all domains")]
        };
        ranked.sort_by(|a, b| {
            let diversity = if request.fixed_domain.is_none() {
                exposure_count(a.candidate.question.domain)
                    .cmp(&exposure_count(b.candidate.question.domain))
            } else {
                std::cmp::Ordering::Equal
            };
            diversity
                .then_with(|| {
                    (b.candidate.category == category).cmp(&(a.candidate.category == category))
                })
                .then_with(|| b.priority.total_cmp(&a.priority))
                .then_with(|| {
                    tie_key(a.candidate.id, request.seed)
                        .cmp(&tie_key(b.candidate.id, request.seed))
                })
                .then_with(|| a.candidate.id.cmp(&b.candidate.id))
        });
        let next = ranked.remove(0);
        ranked.retain(|q| q.candidate.family_id != next.candidate.family_id);
        selected.push(next);
    }
    Ok(QuestionSelection {
        policy_version: POLICY_VERSION,
        pool_id: request.pool_id.clone(),
        pool_question_ids: request.candidates.iter().map(|c| c.id).collect(),
        seed: request.seed,
        selection_probability: None,
        status: if selected.is_empty() {
            "no_eligible_questions"
        } else {
            "selected"
        },
        selected,
        excluded_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn candidate(n: u128) -> QuestionCandidate {
        let mut question: RankRequest =
            serde_json::from_str(include_str!("../../../contracts/fixtures/rank-valid.json"))
                .unwrap();
        question.allow_cloud = false;
        QuestionCandidate {
            id: Uuid::from_u128(n),
            family_id: Uuid::from_u128(n),
            question,
            synthetic_scenario: true,
            source: "test_fixture".into(),
            category: QuestionCategory::Gap,
            owner_declared_importance: None,
            observed_frequency: None,
            estimated_answer_seconds: 30,
        }
    }
    fn request(candidates: Vec<QuestionCandidate>) -> QuestionSelectionRequest {
        QuestionSelectionRequest {
            workspace_id: candidate(1).question.workspace_id,
            pool_id: "fixture_v1".into(),
            candidates,
            exposures: vec![],
            limit: 1,
            seed: 42,
            session_offset: 0,
            fixed_domain: None,
        }
    }
    fn record(c: &QuestionCandidate, response: ObservationResponse) -> Record {
        let p = ObservationProposal {
            schema_version: "1.0".into(),
            request_id: c.id,
            workspace_id: c.question.workspace_id,
            family_id: c.family_id,
            case_kind: cdna_domain::CaseKind::Hypothetical,
            domain: c.question.domain,
            context: c.question.context.clone(),
            candidates: c.question.candidates.clone(),
            response,
            source: cdna_domain::SourceProvenance {
                artifact_id: c.id,
                source_kind: SourceKind::HumanApp,
                occurred_at: None,
                observed_at: c.question.context.as_of,
            },
            rationale_explicit: None,
            reversal_conditions: vec![],
            model_exposure: false,
        };
        Record {
            workspace_id: p.workspace_id,
            id: c.id,
            revision: 1,
            status: Status::Confirmed,
            payload: serde_json::to_value(p).unwrap(),
        }
    }
    #[test]
    fn coldstart_empty_and_reproducible_order() {
        let empty = select_questions(&request(vec![]), &[], None).unwrap();
        assert_eq!(empty.status, "no_eligible_questions");
        let mut r = request(vec![candidate(1), candidate(2)]);
        let a = select_questions(&r, &[], None).unwrap();
        assert!(
            a.selected[0]
                .signals
                .gap_reasons
                .contains(&"domain_count_gap")
        );
        assert_eq!(a.selected[0].signals.raw_top_two_margin, None);
        r.candidates.reverse();
        let b = select_questions(&r, &[], None).unwrap();
        assert_eq!(a.selected[0].candidate.id, b.selected[0].candidate.id);
    }
    #[test]
    fn exposed_ids_families_and_batch_families_never_repeat() {
        let a = candidate(1);
        let mut b = candidate(2);
        b.family_id = a.family_id;
        let mut r = request(vec![a.clone(), b, candidate(3)]);
        r.limit = 3;
        assert_eq!(select_questions(&r, &[], None).unwrap().selected.len(), 2);
        r.exposures.push(QuestionExposure {
            question_id: a.id,
            family_id: a.family_id,
            domain: a.question.domain,
        });
        let result = select_questions(&r, &[], None).unwrap();
        assert_eq!(result.selected.len(), 1);
        assert_eq!(result.selected[0].candidate.id, Uuid::from_u128(3));
        let skipped = record(&a, ObservationResponse::Skip);
        let result = select_questions(&r, &[skipped], None).unwrap();
        assert_eq!(result.selected.len(), 1);
    }
    #[test]
    fn coverage_importance_burden_and_session_diversity() {
        let history = record(&candidate(99), ObservationResponse::Defer);
        let a = candidate(1);
        let mut b = candidate(2);
        b.question.domain = Domain::RiskReputation;
        let r = request(vec![a.clone(), b.clone()]);
        assert_eq!(
            select_questions(&r, &[history], None).unwrap().selected[0]
                .candidate
                .id,
            b.id
        );
        b.question.domain = a.question.domain;
        b.owner_declared_importance = Some(1.0);
        let r = request(vec![a.clone(), b.clone()]);
        assert_eq!(
            select_questions(&r, &[], None).unwrap().selected[0]
                .candidate
                .id,
            b.id
        );
        b.owner_declared_importance = None;
        b.estimated_answer_seconds = 600;
        assert_eq!(
            select_questions(&request(vec![a.clone(), b]), &[], None)
                .unwrap()
                .selected[0]
                .candidate
                .id,
            a.id
        );
        let mut r = request(vec![a.clone(), candidate(2)]);
        r.candidates[1].question.domain = Domain::RiskReputation;
        r.limit = 2;
        let result = select_questions(&r, &[], None).unwrap();
        assert_ne!(
            result.selected[0].candidate.question.domain,
            result.selected[1].candidate.question.domain
        );
        r.fixed_domain = Some(a.question.domain);
        assert_eq!(select_questions(&r, &[], None).unwrap().selected.len(), 1);
    }
    #[test]
    fn outcomes_are_distinct_and_not_preference_labels() {
        for (response, expected) in [
            (ObservationResponse::Skip, AnswerSemantics::Skip),
            (ObservationResponse::Defer, AnswerSemantics::Defer),
            (ObservationResponse::NoneFit, AnswerSemantics::NoneFit),
            (
                ObservationResponse::NeedInformation {
                    fields: vec!["cost".into()],
                },
                AnswerSemantics::Insufficient,
            ),
            (
                ObservationResponse::Pairwise {
                    left_id: "a".into(),
                    right_id: "b".into(),
                    outcome: PairwiseOutcome::Tie,
                },
                AnswerSemantics::Tie,
            ),
        ] {
            assert_eq!(answer_semantics(&response), expected);
        }
        let r = request(vec![candidate(1)]);
        for response in [
            ObservationResponse::Skip,
            ObservationResponse::NoneFit,
            ObservationResponse::NeedInformation {
                fields: vec!["cost".into()],
            },
        ] {
            let result = select_questions(&r, &[record(&candidate(99), response)], None).unwrap();
            assert_eq!(result.selected[0].signals.independent_domain_cases, 0);
        }
    }
    #[test]
    fn local_margin_reuses_inference_and_missing_is_not_confidence() {
        let mut a = candidate(1);
        let mut b = candidate(2);
        for c in [&mut a, &mut b] {
            c.question.context.facts.clear();
            for v in &mut c.question.candidates {
                v.attributes.clear();
            }
        }
        a.question.candidates[0]
            .attributes
            .insert("cost".into(), cdna_domain::FactValue::Number(0.0));
        a.question.candidates[1]
            .attributes
            .insert("cost".into(), cdna_domain::FactValue::Number(1.0));
        b.question.candidates[0]
            .attributes
            .insert("cost".into(), cdna_domain::FactValue::Number(0.0));
        b.question.candidates[1]
            .attributes
            .insert("cost".into(), cdna_domain::FactValue::Number(0.0));
        let mut weights = vec![0.; cdna_inference::DIM];
        weights[0] = 1.;
        let model = LinearScorer::new("1.0", weights).unwrap();
        let result = select_questions(&request(vec![a, b.clone()]), &[], Some(&model)).unwrap();
        assert_eq!(result.selected[0].candidate.id, b.id);
        assert_eq!(result.selected[0].signals.raw_top_two_margin, Some(0.0));
        assert_eq!(result.selection_probability, None);
    }
    #[test]
    fn rejects_bounds_invalid_signals_and_cross_workspace() {
        let mut r = request(vec![candidate(1); MAX_POOL + 1]);
        assert!(select_questions(&r, &[], None).is_err());
        r = request(vec![candidate(1)]);
        r.candidates[0].owner_declared_importance = Some(f64::NAN);
        assert!(select_questions(&r, &[], None).is_err());
        r = request(vec![candidate(1)]);
        r.candidates[0].question.workspace_id = Uuid::nil();
        assert!(select_questions(&r, &[], None).is_err());
        r = request(vec![candidate(1)]);
        r.limit = 0;
        assert!(select_questions(&r, &[], None).is_err());
        r.limit = 1;
        let mut other = record(&candidate(99), ObservationResponse::Skip);
        other.workspace_id = Uuid::nil();
        assert!(select_questions(&r, &[other], None).is_err());
        let exposures = QuestionExposure {
            question_id: Uuid::nil(),
            family_id: Uuid::nil(),
            domain: Domain::RiskReputation,
        };
        r.exposures = vec![exposures; MAX_HISTORY + 1];
        assert!(select_questions(&r, &[], None).is_err());
    }
    #[test]
    fn pending_and_ai_labels_do_not_supply_coverage() {
        let r = request(vec![candidate(1)]);
        let mut pending = record(&candidate(98), ObservationResponse::Defer);
        pending.status = Status::Pending;
        let mut ai = record(&candidate(99), ObservationResponse::Defer);
        ai.payload["source"]["source_kind"] = serde_json::json!("ai_generated");
        let result = select_questions(&r, &[pending, ai], None).unwrap();
        assert_eq!(result.selected[0].signals.independent_domain_cases, 0);
    }
    #[test]
    fn category_allocation_and_model_feature_fallback() {
        let mut pool = Vec::new();
        for n in 1..=30 {
            let mut c = candidate(n);
            c.category = match n % 3 {
                0 => QuestionCategory::Gap,
                1 => QuestionCategory::Representative,
                _ => QuestionCategory::Exploration,
            };
            pool.push(c);
        }
        let mut r = request(pool);
        r.limit = 10;
        let model = LinearScorer::new("1.0", vec![0.; cdna_inference::DIM]).unwrap();
        let result = select_questions(&r, &[], Some(&model)).unwrap();
        for (category, count) in [
            (QuestionCategory::Gap, 5),
            (QuestionCategory::Representative, 3),
            (QuestionCategory::Exploration, 2),
        ] {
            assert_eq!(
                result
                    .selected
                    .iter()
                    .filter(|s| s.candidate.category == category)
                    .count(),
                count
            );
        }
        // Fixture contains explicit text, not normalized numeric model features.
        assert!(
            result.selected[0]
                .signals
                .gap_reasons
                .contains(&"model_features_unavailable")
        );
        assert_eq!(result.selected[0].signals.raw_top_two_margin, None);
    }
    #[test]
    fn conflict_missing_and_temporal_change_are_separate_signals() {
        let a = candidate(90);
        let b = candidate(91);
        let left = record(
            &a,
            ObservationResponse::ChooseOne {
                candidate_id: a.question.candidates[0].id.clone(),
            },
        );
        let mut right = record(
            &b,
            ObservationResponse::ChooseOne {
                candidate_id: b.question.candidates[1].id.clone(),
            },
        );
        right.payload["source"]["observed_at"] = serde_json::json!("2026-09-22T12:00:00Z");
        let result = select_questions(&request(vec![candidate(1)]), &[left, right], None).unwrap();
        let reasons = &result.selected[0].signals.gap_reasons;
        assert!(reasons.contains(&"conflicting_choices"));
        assert!(reasons.contains(&"temporal_response_change"));
        assert!(!reasons.contains(&"domain_count_gap"));
        let missing = record(
            &a,
            ObservationResponse::NeedInformation {
                fields: vec!["cost".into()],
            },
        );
        let result = select_questions(&request(vec![candidate(1)]), &[missing], None).unwrap();
        assert!(
            result.selected[0]
                .signals
                .gap_reasons
                .contains(&"missing_information")
        );
        assert!(
            !result.selected[0]
                .signals
                .gap_reasons
                .contains(&"conflicting_choices")
        );
    }
    #[test]
    fn text_changes_cannot_invent_context_coverage_and_contaminated_exposure_is_excluded() {
        let a = candidate(99);
        let history = record(&a, ObservationResponse::Defer);
        let mut b = candidate(1);
        b.question.context.summary = "Entirely different wording".into();
        b.question.context.facts.reverse();
        let result = select_questions(&request(vec![b]), &[history], None).unwrap();
        assert_eq!(result.selected[0].signals.independent_context_cases, 1);
        let mut history = record(&a, ObservationResponse::Defer);
        history.payload["model_exposure"] = serde_json::json!(true);
        let result = select_questions(&request(vec![a]), &[history], None).unwrap();
        assert!(result.selected.is_empty());
    }
}
