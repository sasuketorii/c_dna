# Inference v1

`LinearScorer::new(feature_version: &str, weights: Vec<f64>) -> Result<LinearScorer, String>` validates exactly 17 finite bounded weights. `extract_features(context: &serde_json::Value, candidate: &serde_json::Value) -> Result<[f64; 17], String>` follows learner v1. `DecisionScorer::score(&mut self, features: &[[f64;17]]) -> Result<ScoreBatch,String>` accepts at most 4096 rows. `ScoreBatch` has `scores: Vec<f64>`, `engine: String`, `fallback_reason: Option<String>`.

`MojoScorerAdapter::new(reference: LinearScorer, executable: &Path, timeout: Duration)` attempts a persistent numerical-only worker; errors/timeouts permanently fall back to reference and expose the reason. CPU Rust is default; callers must explicitly opt into Mojo. No candidate IDs/text/context cross worker boundary. Scores are raw preferences, never probabilities. Feature version is `1.0`, float64 and stable input-order tie handling.

Feature extraction accepts broker-validated context/candidate objects. Every interaction left operand is an explicit context fact, including customer_impact; every right operand is a candidate attribute. Missing numeric values are zero with missing indicators. Inferred/unknown context facts never enter interactions. Invalid/nonfinite/out-of-normalized-range numeric features are rejected; model weights are bounded to magnitude 1e6.

Explicit non-null context feature facts must declare `unit: "ratio"`; missing or other units are rejected before scoring.
