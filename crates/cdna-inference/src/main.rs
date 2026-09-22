use cdna_inference::{DIM, DecisionScorer, LinearScorer};
use std::io::{self, BufRead, Read, Write};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    loop {
        let mut line = String::new();
        if (&mut input).take(2 * 1024 * 1024).read_line(&mut line)? == 0 {
            break;
        }
        if !line.ends_with('\n') {
            return Err("request too large".into());
        }
        let v: serde_json::Value = serde_json::from_str(&line)?;
        let w: Vec<f64> = serde_json::from_value(v["weights"].clone())?;
        let rows: Vec<[f64; DIM]> = serde_json::from_value(v["features"].clone())?;
        let result = LinearScorer::new("1.0", w)?.score(&rows)?;
        serde_json::to_writer(&mut stdout, &result.scores)?;
        writeln!(&mut stdout)?;
        stdout.flush()?;
    }
    Ok(())
}
