//! Synthetic local Engine-path latency harness; never measures person-level accuracy.
use anyhow::{Result, ensure};
use cdna_app::{Authority, Engine};
use cdna_domain::RankRequest;
use cdna_inference::{DIM, DecisionScorer, LinearScorer, extract_features};
use cdna_store::Store;
use serde_json::{Value, json};
use std::{hint::black_box, time::Instant};
use uuid::Uuid;
use zeroize::Zeroizing;

fn distribution(mut samples: Vec<f64>) -> Value {
    samples.sort_by(f64::total_cmp);
    let percentile =
        |p: f64| samples[((p * samples.len() as f64).ceil() as usize).saturating_sub(1)];
    json!({"unit":"microseconds","samples":samples.len(),"p50":percentile(0.50),"p95":percentile(0.95),"p99":percentile(0.99),"min":samples[0],"max":samples[samples.len()-1],"percentile_method":"nearest_rank"})
}
fn measure<F: FnMut() -> Result<Value>>(
    name: &str,
    size: usize,
    iterations: usize,
    mut f: F,
) -> Result<Value> {
    let start = Instant::now();
    let first = f()?;
    let first_us = start.elapsed().as_secs_f64() * 1e6;
    black_box(&first);
    for _ in 0..10 {
        black_box(f()?);
    }
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        let output = f()?;
        black_box(&output);
        samples.push(start.elapsed().as_secs_f64() * 1e6);
    }
    Ok(
        json!({"scenario":name,"size":size,"first_call_us":first_us,"cold_definition":"first call for this scenario; OS page cache and process are not reset","warmup_calls":10,"warm":distribution(samples),"result_bytes":serde_json::to_vec(&first)?.len(),"first_result":first,"memory_per_scenario":"not measured","cpu_per_scenario":"not measured"}),
    )
}
fn rank_request(w: Uuid) -> Value {
    json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":w,"mode":"imitate","domain":"product_delivery","context":{"as_of":"2026-01-01T00:00:00Z","summary":"SYNTHETIC benchmark delivery decision","facts":[{"key":"deadline_pressure","value":0.8,"unit":"ratio","evidence_status":"explicit"}],"unknown_fields":[]},"candidates":[{"id":"fast","text":"Synthetic fast option","attributes":{"cost":0.1,"speed":0.9}},{"id":"slow","text":"Synthetic slow option","attributes":{"cost":0.8,"speed":0.2}}],"include_evidence":true,"allow_cloud":false})
}
fn rank_call(engine: &mut Engine, request: &Value, matching: usize) -> Result<Value> {
    let output = engine.execute(
        json!({"operation":"rank","request":request}),
        Authority::Human,
    )?;
    ensure!(
        output["execution_authorization"] == "none",
        "execution authority invariant"
    );
    if matching <= 1000 {
        ensure!(
            output["selected_candidate_id"] == "fast",
            "memory selection invariant"
        );
    } else {
        ensure!(
            output["abstained"] == true
                && output["abstention_reasons"]
                    .as_array()
                    .is_some_and(|v| v.contains(&json!("memory_search_limit_reached"))),
            "truncated matching corpus must abstain"
        );
    }
    // Include real result serialization and response-size work in the timed path.
    black_box(serde_json::to_vec(&output)?);
    Ok(output)
}
fn run(iterations: usize) -> Result<Value> {
    let mut results = Vec::new();
    // Auxiliary numeric path, not the existing Mojo-vs-Rust kernel benchmark.
    // Includes JSON input parse, shared feature extraction, model validation, scoring and output JSON.
    for batch in [1, 4, 16, 1024] {
        let request = rank_request(Uuid::new_v4());
        let input = serde_json::to_vec(
            &json!({"context":request["context"],"candidates":vec![request["candidates"][0].clone();batch]}),
        )?;
        results.push(measure(
            "numeric_json_to_features_to_scores",
            batch,
            iterations,
            || {
                let parsed: Value = serde_json::from_slice(black_box(&input))?;
                let features = parsed["candidates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| extract_features(&parsed["context"], c))
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(anyhow::Error::msg)?;
                let mut scorer =
                    LinearScorer::new("1.0", vec![0.125; DIM]).map_err(anyhow::Error::msg)?;
                let result = scorer
                    .score(black_box(&features))
                    .map_err(anyhow::Error::msg)?;
                ensure!(
                    result.scores.len() == batch && result.scores.iter().all(|n| n.is_finite()),
                    "numeric output invariant"
                );
                let output = serde_json::to_value(result)?;
                black_box(serde_json::to_vec(&output)?);
                Ok(output)
            },
        )?);
    }
    for (corpus, matching) in [(1, 1), (100, 100), (1000, 1000), (1001, 1001), (1000, 1)] {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("synthetic.db");
        let key = Zeroizing::new("a".repeat(64));
        let setup = Instant::now();
        let store = Store::create(&path, &key)?;
        let mut engine = Engine {
            store,
            learner: "unused-no-training".into(),
            demo: true,
        };
        let workspace = Uuid::new_v4();
        engine.store.create_workspace(workspace)?;
        let request = rank_request(workspace);
        RankRequest::from_json(&serde_json::to_vec(&request)?)?;
        for i in 0..corpus {
            let mut proposal = json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":workspace,"family_id":Uuid::new_v4(),"case_kind":"hypothetical","domain":request["domain"],"context":request["context"],"candidates":request["candidates"],"response":{"response_type":"choose_one","candidate_id":"fast"},"source":{"artifact_id":Uuid::new_v4(),"source_kind":"human_app","occurred_at":null,"observed_at":"2026-01-01T00:00:00Z"},"model_exposure":false});
            if i >= matching {
                proposal["context"]["summary"] = json!(format!("Unrelated synthetic decision {i}"));
            }
            let record = engine.execute(
                json!({"operation":"propose","proposal":proposal}),
                Authority::Human,
            )?;
            engine.execute(json!({"operation":"confirm","workspace_id":workspace,"id":record["id"],"expected_revision":record["revision"],"request_id":Uuid::new_v4()}),Authority::Human)?;
        }
        let setup_ms = setup.elapsed().as_secs_f64() * 1e3;
        drop(engine);
        let reopen = Instant::now();
        let store = Store::open(&path, &key)?;
        let mut engine = Engine {
            store,
            learner: "unused-no-training".into(),
            demo: true,
        };
        black_box(rank_call(&mut engine, &request, matching)?);
        let reopen_and_first_rank_ms = reopen.elapsed().as_secs_f64() * 1e3;
        let mut memory = measure("engine_rank_confirmed_memory", corpus, iterations, || {
            rank_call(&mut engine, &request, matching)
        })?;
        memory["matching_records"] = json!(matching);
        memory["setup_ms"] = json!(setup_ms);
        memory["reopen_and_first_rank_ms"] = json!(reopen_and_first_rank_ms);
        memory["reopen_definition"] =
            json!("SQLCipher Store::open plus first Engine::rank; no OS cache eviction");
        results.push(memory);
        let policy = json!({"statement":"SYNTHETIC benchmark cost constraint","when":{"op":"exists","field":"deadline_pressure"},"requires":{"op":"compare","field":"cost","comparison":"lte","value":0.5,"unit":"ratio"},"effective_from":"2020-01-01T00:00:00Z","expires_at":null});
        let draft = engine.execute(
            json!({"operation":"propose_policy","workspace_id":workspace,"policy":policy}),
            Authority::Human,
        )?;
        engine.execute(json!({"operation":"approve_policy","workspace_id":workspace,"id":draft["id"],"expected_revision":draft["revision"]}),Authority::Human)?;
        let mut policy_result = measure(
            "engine_rank_memory_and_approved_policy",
            corpus,
            iterations,
            || {
                let output = rank_call(&mut engine, &request, matching)?;
                ensure!(
                    output["ranking"][0]["policy_status"] == "compliant"
                        && output["ranking"][1]["policy_status"] == "violated",
                    "policy evaluation invariant"
                );
                Ok(output)
            },
        )?;
        policy_result["matching_records"] = json!(matching);
        policy_result["approved_policies"] = json!(1);
        results.push(policy_result);
    }
    // Full first results aid correctness but numeric batch data is redundant; retain bounded summaries.
    for result in &mut results {
        let first = result
            .as_object_mut()
            .unwrap()
            .remove("first_result")
            .unwrap();
        result["correctness"] = if let Some(scores) = first["scores"].as_array() {
            json!({"score_count":scores.len(),"score_checksum":scores.iter().map(|n|n.as_f64().unwrap()).sum::<f64>()})
        } else {
            json!({"selected_candidate_id":first["selected_candidate_id"],"abstained":first["abstained"],"abstention_reasons":first["abstention_reasons"],"evidence_count":first["evidence"].as_array().map_or(0,Vec::len),"execution_authorization":first["execution_authorization"]})
        };
    }
    Ok(
        json!({"schema_version":"1.0","synthetic":true,"personal_accuracy":"not measured","build_profile":if cfg!(debug_assertions){"debug"}else{"release"},"iterations":iterations,"timing":"Instant monotonic wall clock; single-thread sequential scenarios","limitations":["numeric batch 1024 is an internal scorer load test; public RankRequest remains 2-16 candidates","cold means first call or database reopen, not OS-cache-cold","no Python training, Mojo IPC, browser, transport, or human accuracy measurement","Only truncated exact matches above 1000 abstain; unrelated corpus does not cause search-limit abstention","CPU and peak RSS in runner cover process including synthetic setup; no per-scenario attribution"],"results":results}),
    )
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let iterations = if args.is_empty() {
        200
    } else {
        ensure!(
            args.len() == 2 && args[0] == "--iterations",
            "usage: cdna-core-bench [--iterations 1..10000]"
        );
        args[1].parse::<usize>()?
    };
    ensure!((1..=10000).contains(&iterations), "iteration bound");
    println!("{}", serde_json::to_string(&run(iterations)?)?);
    Ok(())
}
