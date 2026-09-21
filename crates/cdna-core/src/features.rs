use crate::domain::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const RAW_MANIFEST: &str = include_str!("../../../contracts/features.json");
#[derive(Debug, Clone, Deserialize)]
pub struct ContextFeature {
    pub key: String,
    pub kind: String,
    pub scale: Option<f64>,
    pub unit: Option<String>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub values: Option<BTreeMap<String, f64>>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct FeatureManifest {
    pub version: String,
    pub dtype: String,
    pub context_features: Vec<ContextFeature>,
    pub candidate_features: Vec<String>,
    pub domains: Vec<String>,
    pub tie_absolute_tolerance: f64,
}
pub fn manifest() -> &'static FeatureManifest {
    static VALUE: OnceLock<FeatureManifest> = OnceLock::new();
    VALUE.get_or_init(|| serde_json::from_str(RAW_MANIFEST).expect("checked-in feature manifest must be valid"))
}
pub fn manifest_hash() -> String { hex::encode(Sha256::digest(RAW_MANIFEST.as_bytes())) }
pub fn dimension() -> usize { let m = manifest(); m.candidate_features.len() * (2 + 2 * m.context_features.len() + m.domains.len()) }
pub fn names() -> Vec<String> {
    let m = manifest();
    let mut result = Vec::with_capacity(dimension());
    for a in &m.candidate_features { result.push(format!("candidate:{a}")); }
    for a in &m.candidate_features { result.push(format!("candidate_missing:{a}")); }
    for c in &m.context_features { for a in &m.candidate_features { result.push(format!("context:{}*{a}", c.key)); } }
    for c in &m.context_features { for a in &m.candidate_features { result.push(format!("context_missing:{}*{a}", c.key)); } }
    for d in &m.domains { for a in &m.candidate_features { result.push(format!("domain:{d}*{a}")); } }
    result
}
#[derive(Debug, Clone, Serialize)]
pub struct ContextValues { pub values: Vec<f64>, pub missing: Vec<f64>, pub out_of_range: Vec<String> }
pub fn context_values(context: &Context) -> Result<ContextValues> {
    let mut result = ContextValues { values: vec![], missing: vec![], out_of_range: vec![] };
    for cfg in &manifest().context_features {
        let fact = context.facts.iter().find(|f| f.key == cfg.key);
        let Some(fact) = fact.filter(|f| !f.value.is_null() && f.evidence_status == "explicit" && !context.unknown_fields.contains(&f.key)) else {
            result.values.push(0.0); result.missing.push(1.0); continue;
        };
        if cfg.kind == "category" {
            let value = fact.value.as_str().and_then(|s| cfg.values.as_ref()?.get(s));
            if let Some(value) = value { result.values.push(*value); result.missing.push(0.0); }
            else { result.values.push(0.0); result.missing.push(1.0); result.out_of_range.push(cfg.key.clone()); }
        } else {
            let value = fact.value.as_f64().filter(|x| x.is_finite()).ok_or_else(|| Error::new("INPUT_INVALID"))?;
            if let Some(unit) = &cfg.unit { ensure(fact.unit.as_ref() == Some(unit), "UNIT_REQUIRED")?; }
            if value < cfg.minimum.unwrap_or(f64::NEG_INFINITY) || value > cfg.maximum.unwrap_or(f64::INFINITY) { result.out_of_range.push(cfg.key.clone()); }
            result.values.push((value / cfg.scale.unwrap_or(1.0)).clamp(0.0, 1.0)); result.missing.push(0.0);
        }
    }
    Ok(result)
}
pub fn transform(domain: &str, context: &Context, candidates: &[Candidate]) -> Result<Vec<Vec<f64>>> {
    let m = manifest();
    ensure(m.domains.iter().any(|d| d == domain), "UNSUPPORTED_DOMAIN")?;
    let cx = context_values(context)?;
    let mut rows = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let mut a = Vec::with_capacity(m.candidate_features.len());
        let mut missing = Vec::with_capacity(m.candidate_features.len());
        for key in &m.candidate_features {
            match candidate.attributes.get(key) {
                None | Some(Value::Null) => { a.push(0.0); missing.push(1.0); },
                Some(value) => {
                    let v = value.as_f64().filter(|v| v.is_finite() && (0.0..=1.0).contains(v)).ok_or_else(|| Error::new("INVALID_CANDIDATE_FEATURE"))?;
                    a.push(v); missing.push(0.0);
                }
            }
        }
        let mut row = Vec::with_capacity(dimension());
        row.extend_from_slice(&a); row.extend_from_slice(&missing);
        for c in &cx.values { for v in &a { row.push(c * v); } }
        for c in &cx.missing { for v in &a { row.push(c * v); } }
        for d in &m.domains { for v in &a { row.push(if d == domain { *v } else { 0.0 }); } }
        rows.push(row);
    }
    Ok(rows)
}
pub fn score_rows(weights: &[f64], rows: &[Vec<f64>]) -> Result<Vec<f64>> {
    ensure(weights.len() == dimension() && weights.iter().all(|x| x.is_finite()), "MODEL_INVALID")?;
    let mut scores = Vec::with_capacity(rows.len());
    for row in rows {
        ensure(row.len() == weights.len() && row.iter().all(|x| x.is_finite()), "INPUT_INVALID")?;
        let value: f64 = row.iter().zip(weights).map(|(x,w)| x*w).sum();
        ensure(value.is_finite(), "MODEL_INVALID")?;
        scores.push(value);
    }
    Ok(scores)
}
pub fn rank(scores: &[f64], candidates: &[Candidate]) -> Vec<RankedCandidate> {
    let mut order: Vec<usize> = (0..scores.len()).collect();
    order.sort_by(|&a,&b| scores[b].total_cmp(&scores[a]));
    let mut result = vec![];
    let mut group = 1;
    let mut group_score = order.first().map(|&i| scores[i]).unwrap_or(0.0);
    for i in order {
        if (group_score - scores[i]).abs() > manifest().tie_absolute_tolerance { group += 1; group_score = scores[i]; }
        result.push(RankedCandidate { candidate_id: candidates[i].id.clone(), raw_score: scores[i], tie_group: group });
    }
    result
}
