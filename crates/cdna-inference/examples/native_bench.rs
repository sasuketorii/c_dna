use cdna_inference::{DIM, DecisionScorer, LinearScorer};
fn main() {
    let mut scorer = LinearScorer::new("1.0", vec![0.125; DIM]).unwrap();
    let mut measurements = Vec::new();
    for n in [1, 4, 16, 1024] {
        let rows = vec![[0.5; DIM]; n];
        for run in 0..3 {
            for _ in 0..20 {
                std::hint::black_box(scorer.score(std::hint::black_box(&rows)).unwrap());
            }
            let mut samples = Vec::new();
            for _ in 0..100 {
                let start = std::time::Instant::now();
                std::hint::black_box(scorer.score(std::hint::black_box(&rows)).unwrap());
                samples.push(start.elapsed().as_secs_f64() * 1000.);
            }
            samples.sort_by(f64::total_cmp);
            measurements.push(serde_json::json!({"candidates":n,"run":run,"p50_ms":samples[50],"p95_ms":samples[95],"p99_ms":samples[99]}));
        }
    }
    println!("{}", serde_json::to_string(&measurements).unwrap());
}
