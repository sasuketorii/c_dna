//! Thin browser facade over the same domain contract as the native application.
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn validate_proposal(json: &str) -> Result<String, JsValue> {
    let value = cdna_domain::ObservationProposal::from_json(json.as_bytes())
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    serde_json::to_string(&value).map_err(|error| JsValue::from_str(&error.to_string()))
}

#[wasm_bindgen]
pub fn validate_rank(json: &str) -> Result<String, JsValue> {
    let value = cdna_domain::RankRequest::from_json(json.as_bytes())
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    serde_json::to_string(&value).map_err(|error| JsValue::from_str(&error.to_string()))
}

/// The same bounded float64 scorer as native inference. No Python or network calls.
#[wasm_bindgen]
pub fn score_candidates(json: &str) -> Result<String, JsValue> {
    score(json).map_err(|error|JsValue::from_str(&error))
}
fn score(json:&str)->Result<String,String> {
    use cdna_inference::{DecisionScorer,LinearScorer,extract_features};
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {feature_version:String,weights:Vec<f64>,request:cdna_domain::RankRequest}
    let value=cdna_domain::parse_json(json.as_bytes()).map_err(|e|e.to_string())?;
    let input:Input=serde_json::from_value(value).map_err(|_|"invalid score input")?;
    input.request.validate().map_err(|e|e.to_string())?;
    if input.request.allow_cloud {return Err("cloud unavailable".into());}
    let mut scorer=LinearScorer::new(&input.feature_version,input.weights.clone())?;
    let context=serde_json::to_value(&input.request.context).map_err(|e|e.to_string())?;
    let features=input.request.candidates.iter().map(|c| {
        let c=serde_json::to_value(c).map_err(|e|e.to_string())?;
        extract_features(&context,&c)
    }).collect::<Result<Vec<_>,_>>()?;
    let scores=scorer.score(&features)?;
    let contributions:Vec<Vec<f64>>=features.iter().map(|row|row.iter().zip(&input.weights).map(|(x,w)|x*w).collect()).collect();
    serde_json::to_string(&serde_json::json!({"scores":scores.scores,"features":features,"contributions":contributions,"engine":"rust_wasm"})).map_err(|e|e.to_string())
}
