use cdna_app::{Authority, Engine};
use cdna_store::{DocumentKind, Store};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;
#[path = "../src/mcp_bridge.rs"]
#[allow(dead_code)]
mod mcp_bridge;
fn setup() -> (tempfile::TempDir, Engine, Uuid, String) {
    let dir = tempfile::tempdir().unwrap();
    let mut store =
        Store::create(dir.path().join("vault.db"), &Zeroizing::new("z".repeat(64))).unwrap();
    let w = Uuid::new_v4();
    store.create_workspace(w).unwrap();
    let hash = format!("{:x}", Sha256::digest(Uuid::new_v4().as_bytes()));
    store
        .create_grant(cdna_store::Grant {
            id: Uuid::new_v4(),
            workspace_id: w,
            scopes: vec![
                "profile:read".into(),
                "learning:read".into(),
                "question:propose".into(),
            ],
            expires_at: chrono::Utc::now().timestamp() + 300,
            revoked: false,
            token_hash: hash.clone(),
        })
        .unwrap();
    (
        dir,
        Engine::new(
            store,
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../learner"),
            true,
        ),
        w,
        hash,
    )
}
fn question(w: Uuid) -> Value {
    json!({"id":Uuid::new_v4(),"family_id":Uuid::new_v4(),"question":{"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":w,"mode":"imitate","domain":"product_delivery","context":{"as_of":"2026-09-22T00:00:00Z","summary":"Synthetic deadline tradeoff","facts":[{"key":"deadline_pressure","value":1.,"unit":"ratio","evidence_status":"explicit"}],"unknown_fields":[]},"candidates":[{"id":"a","text":"Ship","attributes":{"speed":1.}},{"id":"b","text":"Wait","attributes":{"speed":-1.}}],"allow_cloud":false},"synthetic_scenario":true,"source":"llm_proposal","category":"gap","owner_declared_importance":null,"observed_frequency":null,"estimated_answer_seconds":30})
}
fn batch(w: Uuid, epoch: u64) -> Value {
    let q = question(w);
    json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":w,"epoch":epoch,"provider":"codex","evidence_refs":[],"observed_issue":"No confirmed train evidence yet","hypothesis":"Deadline pressure may change choices","proposed_change":"Ask a neutral synthetic scenario","evaluation_plan":"Collect independent family heldout and evaluate","risks":"Leading questions and contamination","rollback":"Discard proposal, retain prior engine","measured_effect":null,"question_origins":[{"question_id":q["id"],"derived_from":null}],"questions":[q]})
}
fn enqueue(e: &mut Engine, hash: &str, b: &Value) -> anyhow::Result<Value> {
    mcp_bridge::execute(e, hash, json!({"operation":"enqueue","request":b}))
}
fn profile(e: &mut Engine, hash: &str, w: Uuid) -> anyhow::Result<Value> {
    mcp_bridge::execute(
        e,
        hash,
        json!({"operation":"profile","schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":w}),
    )
}
#[test]
fn real_bridge_draft_human_answer_confirm_train_and_retry() {
    let (_dir, mut e, w, hash) = setup();
    let b = batch(w, e.store.epoch().unwrap());
    let model = Uuid::new_v4();
    e.store.save_model(w,model,0,json!({"created_at":"2026-09-22T00:00:00Z","lineage":[],"split_family_ids":{"train":[]}})).unwrap();
    e.store
        .put_evolution_document(
            w,
            Store::EVOLUTION_DOCUMENT_ID,
            0,
            0,
            json!({"active_model":model}),
        )
        .unwrap();
    let result = enqueue(&mut e, &hash, &b).unwrap();
    assert_eq!(enqueue(&mut e, &hash, &b).unwrap(), result);
    assert_eq!(e.store.epoch().unwrap(), 0);
    assert!(e.store.model(w, model).is_ok());
    assert_eq!(
        e.store
            .get_document(DocumentKind::Improvement, w, Store::EVOLUTION_DOCUMENT_ID)
            .unwrap()
            .payload["active_model"],
        model.to_string()
    );
    assert!(e.store.list(w, 100, 0).unwrap().is_empty());
    let mut different = b.clone();
    different["hypothesis"] = json!("different");
    assert!(enqueue(&mut e, &hash, &different).is_err());
    let answer = json!({"operation":"with_ai_respond","workspace_id":w,"id":b["request_id"],"question_id":b["questions"][0]["id"],"request_id":Uuid::new_v4(),"response":{"response_type":"choose_one","candidate_id":"a"}});
    assert!(e.execute(answer.clone(), Authority::Human).is_err());
    e.execute(
        json!({"operation":"with_ai_adopt","workspace_id":w,"id":b["request_id"],"epoch":0}),
        Authority::Human,
    )
    .unwrap();
    let r = e.execute(answer.clone(), Authority::Human).unwrap();
    assert_eq!(r["status"], "pending");
    assert_eq!(r["payload"]["source"]["source_kind"], "human_app");
    assert_eq!(r["payload"]["case_kind"], "hypothetical");
    assert_eq!(r["payload"]["family_id"], b["questions"][0]["family_id"]);
    assert_eq!(e.execute(answer, Authority::Human).unwrap(), r);
    assert!(
        e.store
            .list_documents(DocumentKind::Coaching, w, 32, 0)
            .unwrap()
            .is_empty()
    );
    e.execute(json!({"operation":"confirm","workspace_id":w,"id":r["id"],"expected_revision":1,"request_id":Uuid::new_v4()}),Authority::Human).unwrap();
    let input = e.prepare_train(w).unwrap();
    assert_eq!(input.input["records"].as_array().unwrap().len(), 1);
    let trained = e
        .execute(
            json!({"operation":"with_ai_bootstrap","workspace_id":w}),
            Authority::Human,
        )
        .unwrap();
    assert_eq!(trained["state"], "provisional");
}
#[test]
fn scopes_expiry_no_ai_labels_and_atomic_batch() {
    let (_dir, mut e, w, hash) = setup();
    let b = batch(w, 0);
    let agent = Authority::Agent {
        workspace_id: w,
        scopes: vec!["profile:read".into(), "learning:read".into()],
    };
    assert!(
        e.execute(
            json!({"operation":"with_ai_enqueue","request":b}),
            agent.clone()
        )
        .is_err()
    );
    assert!(
        e.execute(
            json!({"operation":"with_ai_adopt","workspace_id":w,"id":b["request_id"],"epoch":0}),
            agent
        )
        .is_err()
    );
    assert!(profile(&mut e, &hash, Uuid::new_v4()).is_err());
    assert!(mcp_bridge::execute(&mut e,&hash,json!({"operation":"find","schema_version":"1.0","workspace_id":w,"request_id":Uuid::new_v4(),"query":"x"})).is_err());
    for key in ["response", "confirmed_by", "source_kind", "execute"] {
        let mut bad = b.clone();
        bad["questions"][0][key] = json!("human_app");
        assert!(enqueue(&mut e, &hash, &bad).is_err());
    }
    let mut bad = b.clone();
    bad["questions"]
        .as_array_mut()
        .unwrap()
        .push(question(Uuid::new_v4()));
    assert!(enqueue(&mut e, &hash, &bad).is_err());
    assert!(
        e.store
            .list_documents(DocumentKind::Coaching, w, 32, 0)
            .unwrap()
            .is_empty()
    );
    let mut bad = b.clone();
    bad["measured_effect"] = json!(0.9);
    assert!(enqueue(&mut e, &hash, &bad).is_err());
    let grant = e.store.authorize_grant(&hash).unwrap();
    e.store.revoke_grant(grant.id).unwrap();
    assert!(enqueue(&mut e, &hash, &b).is_err());
}
fn save(e: &mut Engine, w: Uuid, label: &str) -> Value {
    let q = question(w);
    let p = json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":w,"family_id":q["family_id"],"case_kind":"hypothetical","domain":"product_delivery","context":q["question"]["context"],"candidates":q["question"]["candidates"],"response":{"response_type":"choose_one","candidate_id":"a"},"source":{"artifact_id":Uuid::new_v4(),"source_kind":"human_app","occurred_at":null,"observed_at":"2026-09-22T00:00:00Z"},"rationale_explicit":label,"model_exposure":false});
    let r = e
        .execute(
            json!({"operation":"propose","proposal":p}),
            Authority::Human,
        )
        .unwrap();
    e.execute(json!({"operation":"confirm","workspace_id":w,"id":r["id"],"expected_revision":1,"request_id":Uuid::new_v4()}),Authority::Human).unwrap()
}
#[test]
fn train_only_profile_and_input_mutation_purges_copied_drafts() {
    let (dir, mut e, w, hash) = setup();
    let train = save(&mut e, w, "TRAIN");
    let held = save(&mut e, w, "HELD_OUT_SECRET");
    let epoch = e.store.epoch().unwrap();
    let model = Uuid::new_v4();
    e.store.save_model(w,model,epoch,json!({"created_at":"2026-09-22T00:00:00Z","lineage":[{"id":train["id"],"revision":2},{"id":held["id"],"revision":2}],"split_family_ids":{"train":[train["payload"]["family_id"]],"test":[held["payload"]["family_id"]]}})).unwrap();
    let p = profile(&mut e, &hash, w).unwrap();
    assert_eq!(p["evidence"].as_array().unwrap().len(), 1);
    assert_eq!(p["evidence"][0]["record_id"], train["id"]);
    assert!(!p.to_string().contains(held["id"].as_str().unwrap()));
    assert!(!p.to_string().contains("HELD_OUT_SECRET"));
    let mut b = batch(w, epoch);
    b["questions"][0]["question"]["context"]["summary"] = json!("Independent new scenario");
    b["evidence_refs"] = json!([{"record_id":held["id"],"revision":2}]);
    assert!(enqueue(&mut e, &hash, &b).is_err());
    b["evidence_refs"] = json!([{"record_id":train["id"],"revision":2}]);
    enqueue(&mut e, &hash, &b).unwrap();
    let backup = dir.path().join("backup.db");
    let key = Zeroizing::new("r".repeat(64));
    e.store.backup(&backup, &key).unwrap();
    let restored = Store::restore(
        &backup,
        &key,
        dir.path().join("restored.db"),
        &Zeroizing::new("n".repeat(64)),
        &[],
        &[],
    )
    .unwrap();
    assert!(
        restored
            .list_documents(DocumentKind::Coaching, w, 32, 0)
            .unwrap()
            .is_empty()
    );
    e.execute(
        json!({"operation":"delete","workspace_id":w,"id":train["id"],"expected_revision":2}),
        Authority::Human,
    )
    .unwrap();
    assert!(
        e.store
            .list_documents(DocumentKind::Coaching, w, 32, 0)
            .unwrap()
            .is_empty()
    );
    assert!(enqueue(&mut e, &hash, &b).is_err());
    assert!(e.execute(json!({"operation":"with_ai_adopt","workspace_id":w,"id":b["request_id"],"epoch":epoch}),Authority::Human).is_err());
}
#[tokio::test]
async fn official_sdk_routes_profile_and_batch_to_real_private_bridge() {
    use rmcp::{ServiceExt, model::CallToolRequestParams};
    use std::sync::{Arc, Mutex};
    let (_dir, e, w, hash) = setup();
    let engine = Arc::new(Mutex::new(e));
    let dispatch_engine = engine.clone();
    let server = cdna_mcp::McpServer::new(Arc::new(move |command| {
        mcp_bridge::execute(&mut dispatch_engine.lock().unwrap(), &hash, command)
            .map_err(|e| e.to_string())
    }));
    let (client_io, server_io) = tokio::io::duplex(256 * 1024);
    let serving = tokio::spawn(async move { server.serve(server_io).await.unwrap() });
    let client = ().serve(client_io).await.unwrap();
    let server = serving.await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    let tool = tools
        .iter()
        .find(|t| t.name == "cdna_enqueue_questions")
        .unwrap();
    assert_eq!(tool.input_schema["properties"]["questions"]["maxItems"], 8);
    assert_eq!(
        tool.input_schema["properties"]["questions"]["items"]["additionalProperties"],
        false
    );
    let result = client
        .call_tool(
            CallToolRequestParams::new("cdna_get_profile").with_arguments(
                json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":w})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    let b = batch(w, 0);
    let result = client
        .call_tool(
            CallToolRequestParams::new("cdna_enqueue_questions")
                .with_arguments(b.as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    assert_eq!(
        engine
            .lock()
            .unwrap()
            .store
            .list_documents(DocumentKind::Coaching, w, 32, 0)
            .unwrap()
            .len(),
        1
    );
    assert!(
        engine
            .lock()
            .unwrap()
            .store
            .list(w, 100, 0)
            .unwrap()
            .is_empty()
    );
    client.cancel().await.unwrap();
    server.cancel().await.unwrap();
}
#[test]
fn source_erasure_and_revision_purge_drafts_and_expired_grant_is_denied() {
    let (_dir, mut e, w, hash) = setup();
    let source = Uuid::new_v4();
    e.store
        .put_document(
            DocumentKind::Source,
            w,
            source,
            0,
            json!({"raw":"PRIVATE SOURCE"}),
        )
        .unwrap();
    let b = batch(w, e.store.epoch().unwrap());
    enqueue(&mut e, &hash, &b).unwrap();
    assert!(
        !profile(&mut e, &hash, w)
            .unwrap()
            .to_string()
            .contains("PRIVATE SOURCE")
    );
    e.store.delete_source_cascade(w, source, 1).unwrap();
    assert!(
        e.store
            .list_documents(DocumentKind::Coaching, w, 32, 0)
            .unwrap()
            .is_empty()
    );
    let r = save(&mut e, w, "before correction");
    let mut b = batch(w, e.store.epoch().unwrap());
    b["questions"][0]["question"]["context"]["summary"] = json!("New revision scenario");
    enqueue(&mut e, &hash, &b).unwrap();
    let id: Uuid = serde_json::from_value(r["id"].clone()).unwrap();
    let mut payload = r["payload"].clone();
    payload["rationale_explicit"] = json!("corrected");
    e.store.revise(w, id, 2, Uuid::new_v4(), payload).unwrap();
    assert!(
        e.store
            .list_documents(DocumentKind::Coaching, w, 32, 0)
            .unwrap()
            .is_empty()
    );
    assert!(
        e.store
            .create_grant(cdna_store::Grant {
                id: Uuid::new_v4(),
                workspace_id: w,
                scopes: vec!["question:propose".into()],
                expires_at: chrono::Utc::now().timestamp() - 1,
                revoked: false,
                token_hash: "f".repeat(64)
            })
            .is_err()
    );
}
#[test]
fn derivative_must_keep_family_and_source_and_human_reflection() {
    let (_dir, mut e, w, hash) = setup();
    let r = save(&mut e, w, "origin");
    let epoch = e.store.epoch().unwrap();
    e.store.save_model(w,Uuid::new_v4(),epoch,json!({"created_at":"2026-09-22T00:00:00Z","lineage":[{"id":r["id"],"revision":2}],"split_family_ids":{"train":[r["payload"]["family_id"]]}})).unwrap();
    let mut b = batch(w, epoch);
    b["question_origins"][0]["derived_from"] = json!({"record_id":r["id"],"revision":2});
    assert!(enqueue(&mut e, &hash, &b).is_err());
    b["questions"][0]["family_id"] = r["payload"]["family_id"].clone();
    enqueue(&mut e, &hash, &b).unwrap();
    e.execute(
        json!({"operation":"with_ai_adopt","workspace_id":w,"id":b["request_id"],"epoch":epoch}),
        Authority::Human,
    )
    .unwrap();
    let answer = json!({"operation":"with_ai_respond","workspace_id":w,"id":b["request_id"],"question_id":b["questions"][0]["id"],"request_id":Uuid::new_v4(),"response":{"response_type":"choose_one","candidate_id":"b"},"rationale_explicit":"Safety overrides speed","reversal_conditions":["After verification"],"model_exposure":true});
    let response = e.execute(answer.clone(), Authority::Human).unwrap();
    assert_eq!(response["payload"]["family_id"], r["payload"]["family_id"]);
    assert_eq!(
        response["payload"]["source"]["artifact_id"],
        r["payload"]["source"]["artifact_id"]
    );
    assert_eq!(response["payload"]["model_exposure"], true);
    assert_eq!(
        response["payload"]["rationale_explicit"],
        "Safety overrides speed"
    );
    assert_eq!(
        e.execute(answer.clone(), Authority::Human).unwrap(),
        response
    );
    let mut wrong = answer;
    wrong["question_id"] = json!(Uuid::new_v4());
    assert!(e.execute(wrong, Authority::Human).is_err());
}

#[test]
fn private_bridge_rate_and_bound_checks_are_enforced_on_retries() {
    let (_dir, mut e, w, hash) = setup();
    let b = batch(w, 0);
    for _ in 0..10 {
        enqueue(&mut e, &hash, &b).unwrap();
    }
    assert!(
        enqueue(&mut e, &hash, &b)
            .unwrap_err()
            .to_string()
            .contains("RATE_LIMITED")
    );
    assert_eq!(
        e.store
            .list_documents(DocumentKind::Coaching, w, 32, 0)
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn bootstrap_hypotheses_need_sharing_and_descendants_keep_source_family() {
    use cdna_app::bootstrap;
    let (_dir, mut e, w, hash) = setup();
    let exported = json!({"schema_version":"1.0","provider":"claude","model":null,"context":{"availability":"partial","limitations":["Limited prior conversations"]},"criteria":[{"criterion":"Prefer verified delivery","priority":null,"tradeoffs":["Speed versus verification"],"examples":[],"exceptions":["A reversible experiment"],"reversal_conditions":[],"evidence":[{"kind":"remembered_summary","statement_category":"reported_policy","text":"Unverified remembered preference","reference":null}],"confidence":{"self_report":null,"basis":"Needs owner review"}}],"contradictions":[],"unknowns":["Current priorities"]});
    let imported = bootstrap::import_profile(
        &mut e.store,
        &Authority::Human,
        w,
        Uuid::new_v4(),
        &serde_json::to_vec(&exported).unwrap(),
        false,
    )
    .unwrap();
    let p = profile(&mut e, &hash, w).unwrap();
    assert_eq!(p["bootstrap_hypotheses"], json!([]));
    assert_eq!(p["evidence"], json!([]));
    let shared = bootstrap::set_teacher_sharing(
        &mut e.store,
        &Authority::Human,
        w,
        imported.source_id,
        1,
        true,
    )
    .unwrap();
    let p = profile(&mut e, &hash, w).unwrap();
    assert_eq!(
        p["bootstrap_hypotheses"][0]["authority"],
        "ai_generated_hypothesis"
    );
    assert_eq!(p["bootstrap_hypotheses"][0]["training_eligible"], false);
    assert_eq!(p["evidence"], json!([]));
    let mut b = batch(w, e.store.epoch().unwrap());
    b["question_origins"][0]["derived_source"] =
        json!({"source_id":imported.source_id,"revision":shared.revision});
    assert!(enqueue(&mut e, &hash, &b).is_err());
    b["questions"][0]["family_id"] = json!(imported.source_id);
    enqueue(&mut e, &hash, &b).unwrap();
    e.execute(json!({"operation":"with_ai_adopt","workspace_id":w,"id":b["request_id"],"epoch":b["epoch"]}),Authority::Human).unwrap();
    let response=e.execute(json!({"operation":"with_ai_respond","workspace_id":w,"id":b["request_id"],"question_id":b["questions"][0]["id"],"request_id":Uuid::new_v4(),"response":{"response_type":"choose_one","candidate_id":"b"}}),Authority::Human).unwrap();
    assert_eq!(
        response["payload"]["source"]["artifact_id"],
        imported.source_id.to_string()
    );
    assert_eq!(
        response["payload"]["family_id"],
        imported.source_id.to_string()
    );
    assert_eq!(response["status"], "pending");
    let record_id: Uuid = serde_json::from_value(response["id"].clone()).unwrap();
    e.store
        .delete_source_cascade(w, imported.source_id, shared.revision)
        .unwrap();
    assert!(e.store.get(w, record_id).is_err());
    assert_eq!(
        profile(&mut e, &hash, w).unwrap()["bootstrap_hypotheses"],
        json!([])
    );
}
