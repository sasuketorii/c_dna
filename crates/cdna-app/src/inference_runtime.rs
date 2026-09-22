//! Learned inference only from freshly verified, approved executable configuration.
use crate::evolution_bridge::EvolutionBridge;
use cdna_domain::{EvidenceStatus, FactValue, RankRequest};
use cdna_inference::{DecisionScorer, LinearScorer, extract_features};
use cdna_store::Store;
use serde_json::{Value, json};

const ATTRIBUTES: [&str; 6] = ["cost", "effort", "speed", "reuse", "customer_impact", "irreversibility"];
const FACTS: [&str; 5] = ["deadline_pressure", "asset_importance", "loss_tolerance", "customer_impact", "budget_pressure"];

pub(crate) fn rank(store: &Store, bridge: &EvolutionBridge, request: &RankRequest, policy: &Value, epoch: u64) -> Result<Value, &'static str> {
    let snapshot = bridge.load(store, request.workspace_id, epoch).map_err(|_| "model_evidence_invalid")?;
    let candidate = snapshot.active_candidate().ok_or("no_active_approved_model")?;
    let scope = candidate.config.inference.as_ref().ok_or("model_inference_support_missing")?;
    let domain = serde_json::to_value(request.domain).map_err(|_| "unsupported_domain")?;
    let domain = domain.as_str().ok_or("unsupported_domain")?;
    if !snapshot.active_domains().contains(&domain) || !scope.domains.iter().any(|d| d == domain) { return Err("unsupported_domain"); }
    let now = chrono::Utc::now().timestamp();
    let as_of = request.context.as_of.timestamp();
    if [now, as_of].iter().any(|t| *t < scope.valid_from_unix || *t >= scope.valid_until_unix) { return Err("calibration_period_expired_or_unsupported"); }
    if request.candidates.len() != 2 { return Err("calibrated_two_choice_scope_required"); }
    if !request.context.unknown_fields.is_empty() { return Err("missing_information"); }
    let mut context_mask = 0u16;
    for fact in &request.context.facts {
        if !FACTS.contains(&fact.key.as_str()) { return Err("unsupported_context_feature"); }
        if fact.evidence_status != EvidenceStatus::Explicit { return Err("missing_information"); }
        if !matches!(fact.value, FactValue::Number(v) if v.is_finite() && v.abs() <= 1.0) || fact.unit.as_deref() != Some("ratio") { return Err("unsupported_context_feature"); }
        if request.context.facts.iter().filter(|f| f.key == fact.key).count() != 1 { return Err("conflicting_context_features"); }
    }
    for (i, key) in FACTS.iter().enumerate() {
        if !request.context.facts.iter().any(|f| &f.key == key) { context_mask |= 1 << (i + 6); }
    }
    let context = serde_json::to_value(&request.context).map_err(|_| "invalid_features")?;
    let mut features = Vec::with_capacity(2);
    for option in &request.candidates {
        if option.attributes.keys().any(|k| !ATTRIBUTES.contains(&k.as_str())) { return Err("unsupported_candidate_feature"); }
        let mut mask = context_mask;
        for (i, key) in ATTRIBUTES.iter().enumerate() {
            match option.attributes.get(*key) {
                None | Some(FactValue::Null) => mask |= 1 << i,
                Some(FactValue::Number(v)) if v.is_finite() && v.abs() <= 1.0 => (),
                _ => return Err("unsupported_candidate_feature"),
            }
        }
        if !scope.missing_patterns.contains(&mask) { return Err("unsupported_missing_pattern"); }
        let value = serde_json::to_value(option).map_err(|_| "invalid_features")?;
        let row = extract_features(&context, &value).map_err(|_| "invalid_features")?;
        if row.iter().zip(&scope.feature_min).zip(&scope.feature_max).any(|((v, lo), hi)| v < lo || v > hi) { return Err("out_of_distribution"); }
        features.push(row);
    }
    let mut scorer = LinearScorer::new(&candidate.config.feature_version, candidate.config.weights.clone()).map_err(|_| "invalid_model")?;
    let scores = scorer.score(&features).map_err(|_| "invalid_model")?;
    let delta = scores.scores[0] - scores.scores[1];
    if !delta.is_finite() || delta.abs() < scope.min_margin || delta == 0.0 { return Err("unsupported_margin_or_tie"); }
    if delta.abs() > scope.max_margin { return Err("out_of_distribution_margin"); }
    let winner = usize::from(delta < 0.0);
    let status = |id: &str| policy["candidates"].as_array().and_then(|rows| rows.iter().find(|r| r["candidate_id"] == id)).and_then(|row| row["status"].as_str()).unwrap_or("unknown");
    // Never replace the model winner with the second-best option after a policy rejection.
    if !matches!(status(&request.candidates[winner].id), "compliant" | "no_applicable_policy") { return Err("policy_violation_or_unknown"); }
    let p0 = 1.0 / (1.0 + (-scope.slope * delta).exp());
    let mut ranking: Vec<_> = request.candidates.iter().enumerate().map(|(i, c)| json!({"candidate_id": c.id, "raw_score": scores.scores[i], "probability": null, "memory_match":false, "policy_status":status(&c.id)})).collect();
    ranking.sort_by(|a,b| b["raw_score"].as_f64().unwrap().total_cmp(&a["raw_score"].as_f64().unwrap()));
    // Revalidate the state revision as well as the input epoch after scoring.
    let current = bridge.load(store, request.workspace_id, epoch).map_err(|_| "model_evidence_invalid")?;
    if store.epoch().map_err(|_| "model_evidence_invalid")? != epoch || current.revision != snapshot.revision || current.active_candidate().map(|c| c.id) != Some(candidate.id) { return Err("model_changed_during_inference"); }
    Ok(json!({"mode":request.mode,"ranking":ranking,"selected_candidate_id":request.candidates[winner].id,"abstained":false,"abstention_reasons":[],"execution_authorization":"none","model_status":"approved_learned","model_id":candidate.id,"artifact_hash":candidate.artifact_hash,"evolution_revision":snapshot.revision,"input_epoch":epoch,"engine":scores.engine,"probability":null,"top_choice_agreement_estimate":null,"pairwise_probability":{"left_candidate_id":request.candidates[0].id,"right_candidate_id":request.candidates[1].id,"probability_left":p0},"probability_scope":"calibrated_two_choice","evidence":if request.include_evidence {vec![json!({"kind":"signed_evolution","candidate_id":candidate.id,"artifact_hash":candidate.artifact_hash,"revision":snapshot.revision})]} else {vec![]}}))
}
