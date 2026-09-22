//! Bounded synthetic review probes. All production operations go through existing APIs.
use anyhow::{Result, ensure};
use cdna_app::{Authority, Engine};
use cdna_domain::RankRequest;
use cdna_inference::{DIM, DecisionScorer, LinearScorer, extract_features};
use cdna_store::{DocumentKind, Store, matching_signature};
use serde_json::{Value, json};
use std::{
    hint::black_box,
    sync::{Arc, Barrier},
    time::Instant,
};
use uuid::Uuid;
use zeroize::Zeroizing;
fn timed<F: FnMut() -> Result<()>>(name: &str, n: usize, mut f: F) -> Result<Value> {
    let start = Instant::now();
    f()?;
    let first = start.elapsed().as_secs_f64() * 1e6;
    for _ in 0..10 {
        f()?;
    }
    let mut samples = Vec::new();
    for _ in 0..n {
        let start = Instant::now();
        f()?;
        samples.push(start.elapsed().as_secs_f64() * 1e6);
    }
    samples.sort_by(f64::total_cmp);
    Ok(
        json!({"scenario":name,"unit":"microseconds","samples":n,"warmup":10,"first_call_us":first,"p50":samples[(n as f64*0.5).ceil() as usize-1],"p95":samples[(n as f64*0.95).ceil() as usize-1],"max":samples[n-1],"percentile":"nearest_rank"}),
    )
}
fn request(w: Uuid) -> Value {
    json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":w,"mode":"imitate","domain":"product_delivery","context":{"as_of":"2026-01-01T00:00:00Z","summary":"SYNTHETIC delivery","facts":[{"key":"deadline_pressure","value":0.8,"unit":"ratio","evidence_status":"explicit"}],"unknown_fields":[]},"candidates":[{"id":"fast","text":"Fast","attributes":{"cost":0.1,"speed":0.9}},{"id":"slow","text":"Slow","attributes":{"cost":0.8,"speed":0.2}}],"include_evidence":true,"allow_cloud":false})
}
fn proposal(r: &Value) -> Value {
    json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":r["workspace_id"],"family_id":Uuid::new_v4(),"case_kind":"hypothetical","domain":r["domain"],"context":r["context"],"candidates":r["candidates"],"response":{"response_type":"choose_one","candidate_id":"fast"},"source":{"artifact_id":Uuid::new_v4(),"source_kind":"human_app","occurred_at":null,"observed_at":"2026-01-01T00:00:00Z"},"model_exposure":false})
}
fn execute(e: &mut Engine, v: Value) -> Result<Value> {
    e.execute(v, Authority::Human)
}
fn add(e: &mut Engine, p: Value) -> Result<Value> {
    let w = p["workspace_id"].clone();
    let r = execute(e, json!({"operation":"propose","proposal":p}))?;
    execute(
        e,
        json!({"operation":"confirm","workspace_id":w,"id":r["id"],"expected_revision":r["revision"],"request_id":Uuid::new_v4()}),
    )
}
fn rank(e: &mut Engine, r: &Value) -> Result<Value> {
    execute(e, json!({"operation":"rank","request":r}))
}
fn rank_timed(e: &mut Engine, r: &Value, selected: bool) -> Result<()> {
    let out = rank(e, r)?;
    ensure!(
        out["abstained"] == !selected,
        "unexpected abstention: {out}"
    );
    ensure!(out["execution_authorization"] == "none");
    if selected {
        ensure!(out["selected_candidate_id"] == "fast");
    }
    black_box(serde_json::to_vec(&out)?);
    Ok(())
}
fn run(n: usize) -> Result<Value> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("vault.db");
    let key = Zeroizing::new("a".repeat(64));
    let mut e = Engine::new(Store::create(&path, &key)?, "unused".into(), true);
    let w = Uuid::new_v4();
    e.store.create_workspace(w)?;
    let r = request(w);
    let typed = RankRequest::from_json(&serde_json::to_vec(&r)?)?;
    let setup = Instant::now();
    for i in 0..10_000 {
        let mut p = proposal(&r);
        if i >= 20 {
            p["context"]["summary"] = json!("SYNTHETIC dense");
            p["model_exposure"] = json!(true);
        }
        add(&mut e, p)?;
    }
    let setup_s = setup.elapsed().as_secs_f64();
    let mut results = vec![timed("engine_rank_10k_20_matches_no_policy", n, || {
        rank_timed(&mut e, &r, true)
    })?];
    let sig = matching_signature(&r)?;
    results.push(timed(
        "encrypted_index_retrieval_10k_20_matches",
        n,
        || {
            let found = e.store.matching_records(w, &sig, 1000)?;
            ensure!(found.total == 20 && !found.truncated);
            black_box(found);
            Ok(())
        },
    )?);
    let draft = execute(
        &mut e,
        json!({"operation":"propose_policy","workspace_id":w,"policy":{"statement":"SYNTHETIC cost limit","when":{"op":"exists","field":"deadline_pressure"},"requires":{"op":"compare","field":"cost","comparison":"lte","value":0.5,"unit":"ratio"},"effective_from":"2020-01-01T00:00:00Z","expires_at":null}}),
    )?;
    execute(
        &mut e,
        json!({"operation":"approve_policy","workspace_id":w,"id":draft["id"],"expected_revision":draft["revision"]}),
    )?;
    let policies: Vec<_> = e
        .store
        .list_documents(DocumentKind::Policy, w, 100, 0)?
        .into_iter()
        .map(|d| (d.id, d.payload))
        .collect();
    results.push(timed("policy_evaluation_preloaded_one_policy", n, || {
        let out = cdna_app::policy::check(&policies, &typed.context, &typed.candidates)?;
        ensure!(out["candidates"][1]["status"] == "violated");
        black_box(out);
        Ok(())
    })?);
    results.push(timed(
        "encrypted_policy_retrieval_and_evaluation",
        n,
        || {
            black_box(execute(
                &mut e,
                json!({"operation":"check_policy","request":r}),
            )?);
            Ok(())
        },
    )?);
    let context = serde_json::to_value(&typed.context)?;
    let mut scorer = LinearScorer::new("1.0", vec![0.125; DIM]).map_err(anyhow::Error::msg)?;
    results.push(timed(
        "structured_features_and_linear_scoring_not_active_rank",
        n,
        || {
            let features = typed
                .candidates
                .iter()
                .map(|c| extract_features(&context, &serde_json::to_value(c).unwrap()))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(anyhow::Error::msg)?;
            black_box(scorer.score(&features).map_err(anyhow::Error::msg)?);
            Ok(())
        },
    )?);
    let out = rank(&mut e, &r)?;
    results.push(timed(
        "explanation_evidence_serialization_precomputed_not_generation",
        n,
        || {
            black_box(serde_json::to_vec(&out["evidence"])?);
            Ok(())
        },
    )?);
    results.push(timed(
        "engine_rank_10k_20_matches_with_policy_and_response_json",
        n,
        || rank_timed(&mut e, &r, true),
    )?);
    let mut dense = r.clone();
    dense["context"]["summary"] = json!("SYNTHETIC dense");
    results.push(timed(
        "engine_rank_10k_9980_matches_abstention",
        n.min(100),
        || {
            let out = rank(&mut e, &dense)?;
            ensure!(
                out["abstention_reasons"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("memory_search_limit_reached"))
            );
            black_box(serde_json::to_vec(&out)?);
            Ok(())
        },
    )?);
    let train_at_10k = match e.prepare_train(w) {
        Ok(t) => json!({"ok":true,"records":t.input["records"].as_array().map(Vec::len)}),
        Err(err) => json!({"ok":false,"error":err.to_string()}),
    };
    drop(e);
    let start = Instant::now();
    let mut e = Engine::new(Store::open(&path, &key)?, "unused".into(), true);
    rank_timed(&mut e, &r, true)?;
    let reopen_us = start.elapsed().as_secs_f64() * 1e6;
    drop(e);
    // Connections open sequentially; synchronized independent readers exercise one encrypted DB.
    let engines = (0..2)
        .map(|_| {
            Ok(Engine::new(
                Store::open(&path, &key)?,
                "unused".into(),
                true,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let barrier = Arc::new(Barrier::new(2));
    let concurrent = std::thread::scope(|scope| -> Result<Vec<Value>> {
        let handles = engines
            .into_iter()
            .enumerate()
            .map(|(id, mut engine)| {
                let b = barrier.clone();
                let query = r.clone();
                scope.spawn(move || {
                    b.wait();
                    timed(&format!("concurrent_reader_{id}_10k_20_matches"), n, || {
                        rank_timed(&mut engine, &query, true)
                    })
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|h| h.join().map_err(|_| anyhow::anyhow!("reader panic"))?)
            .collect()
    })?;
    results.extend(concurrent);
    let mut e = Engine::new(Store::open(&path, &key)?, "unused".into(), true);
    let mut probes = vec![json!({"probe":"training_at_10000_records","result":train_at_10k})];
    let mut changed = r.clone();
    changed["context"]["summary"] = json!("SYNTHETIC delivery ");
    probes.push(json!({"probe":"trailing_space_exact_memory","result":rank(&mut e,&changed)?}));
    let mut conflict = proposal(&r);
    conflict["response"]["candidate_id"] = json!("slow");
    let conflicting = add(&mut e, conflict)?;
    let conflict_result = rank(&mut e, &r)?;
    ensure!(
        conflict_result["abstention_reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("conflicting_evidence"))
    );
    probes.push(json!({"probe":"conflicting_confirmed_answer","result":conflict_result}));
    execute(
        &mut e,
        json!({"operation":"delete","workspace_id":w,"id":conflicting["id"],"expected_revision":conflicting["revision"]}),
    )?;
    rank_timed(&mut e, &r, true)?;
    probes.push(json!({"probe":"delete_conflicting_answer_restores_match","passed":true}));
    let export=json!([{"uuid":"synthetic-conversation","chat_messages":(0..1001).map(|i|json!({"uuid":format!("message-{i}"),"sender":"human","text":"SYNTHETIC preference","created_at":"2026-01-01T00:00:00Z"})).collect::<Vec<_>>()}]).to_string();
    let preview = execute(
        &mut e,
        json!({"operation":"import_sources","workspace_id":w,"provider":"claude","export_json":export,"preview":true}),
    )?;
    ensure!(
        preview["total_sources"] == 1001
            && preview["page_count"] == 1000
            && preview["has_more"] == true
    );
    ensure!(
        e.store
            .list_documents(DocumentKind::Source, w, 1000, 0)?
            .is_empty()
    );
    let first = execute(
        &mut e,
        json!({"operation":"import_sources","workspace_id":w,"provider":"claude","export_json":export,"preview":false}),
    )?;
    let second = execute(
        &mut e,
        json!({"operation":"import_sources","workspace_id":w,"provider":"claude","export_json":export,"preview":false,"cursor":first["next_cursor"]}),
    )?;
    let retry = execute(
        &mut e,
        json!({"operation":"import_sources","workspace_id":w,"provider":"claude","export_json":export,"preview":false}),
    )?;
    ensure!(
        first["inserted"] == 1000
            && second["inserted"] == 1
            && second["has_more"] == false
            && retry["inserted"] == 0
            && retry["skipped"] == 1000
    );
    let stored = e
        .store
        .list_documents(DocumentKind::Source, w, 1000, 0)?
        .len()
        + e.store
            .list_documents(DocumentKind::Source, w, 1000, 1000)?
            .len();
    ensure!(stored == 1001);
    probes.push(json!({"probe":"preview_and_commit_export_1001_messages","passed":true,"preview_sources":1000,"stored":stored,"commit_pages":2,"replay_inserted":0}));
    Ok(
        json!({"schema_version":"1.0","synthetic":true,"personal_accuracy":"not measured","corpus":10000,"matching_sparse":20,"matching_dense":9980,"setup_seconds":setup_s,"reopen_and_first_rank_us":reopen_us,"database_bytes":std::fs::metadata(&path)?.len(),"results":results,"probes":probes,"scope":{"test_processes":1,"reader_threads":2,"database":"real SQLCipher temporary file; public propose/confirm ingestion","build_profile":if cfg!(debug_assertions){"debug"}else{"release"},"cold":"first scenario call or connection reopen; no OS page-cache eviction","scoring":"separate structured numeric microbenchmark, not active rank and not arbitrary NL","explanation":"precomputed evidence serialization only; generation remains in full rank timing","timings":"overlapping independent component measurements; do not add or subtract as stage profiling"}}),
    )
}
fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    ensure!(
        args.len() == 2 && args[0] == "--iterations",
        "usage: --iterations 1..1000"
    );
    let n = args[1].parse::<usize>()?;
    ensure!((1..=1000).contains(&n));
    println!("{}", serde_json::to_string(&run(n)?)?);
    Ok(())
}
