use cdna_inference::{DIM, DecisionScorer, LinearScorer, MojoScorerAdapter};
use std::{path::PathBuf, time::Duration};
fn main() {
    let executable = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("compiled Mojo executable required"),
    );
    let mut adapter = MojoScorerAdapter::new(
        LinearScorer::new("1.0", vec![0.5; DIM]).unwrap(),
        &executable,
        Duration::from_secs(5),
    );
    for n in [1, 4, 16, 1024] {
        let result = adapter.score(&vec![[0.25; DIM]; n]).unwrap();
        assert_eq!(result.engine, "mojo", "{:?}", result.fallback_reason);
        assert_eq!(result.scores, vec![2.125; n]);
    }
    println!("persistent Rust to Mojo adapter: four batches equivalent, no fallback");
}
