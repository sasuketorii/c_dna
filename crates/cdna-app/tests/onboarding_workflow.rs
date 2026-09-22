use cdna_app::{Authority, Engine};
use cdna_store::{DocumentKind, Store};
use serde_json::{Value, json};
use uuid::Uuid;
use zeroize::Zeroizing;
fn engine() -> (tempfile::TempDir, Engine, Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(dir.path().join("vault"), &Zeroizing::new("a".repeat(64))).unwrap();
    let w = Uuid::new_v4();
    store.create_workspace(w).unwrap();
    (
        dir,
        Engine::new(store, "unused".into(), true),
        w,
    )
}
fn run(e: &mut Engine, v: Value) -> Value {
    e.execute(v, Authority::Human).unwrap()
}
fn export() -> String {
    json!([{"uuid":"conversation","chat_messages":[{"uuid":"human","sender":"human","text":"My decision"},{"uuid":"ai","sender":"assistant","text":"Suggested decision"}]}]).to_string()
}
fn import(w: Uuid, preview: bool) -> Value {
    json!({"operation":"import_sources","workspace_id":w,"provider":"claude","export_json":export(),"preview":preview})
}
fn assessment() -> Value {
    json!({"schema_version":"1.0","instrument":"tipi_j","instrument_version":"original report","locale":"ja-JP","confirmed_by_user":true,"assessed_on":"2026-09-22","scores":[{"dimension":"reported trait","value":3.0,"minimum":1.0,"maximum":7.0,"unit":"original points"}],"source":{"kind":"manual_result","reference":"local report"},"consent":{"store_locally":true,"mcp_read":false,"external_send":false},"validity":"user_reported_unverified"})
}
fn rank(w: Uuid) -> Value {
    json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":w,"mode":"imitate","domain":"product_delivery","context":{"as_of":"2026-09-22T00:00:00Z","summary":"Decision","facts":[],"unknown_fields":[]},"candidates":[{"id":"a","text":"First","attributes":{}},{"id":"b","text":"Second","attributes":{}}]})
}
fn selection(w: Uuid) -> Value {
    json!({"workspace_id":w,"pool_id":"test","candidates":[{"id":Uuid::new_v4(),"family_id":Uuid::new_v4(),"question":rank(w),"synthetic_scenario":true,"source":"test fixture","category":"gap","estimated_answer_seconds":30}],"exposures":[],"limit":1,"seed":42,"session_offset":0})
}
fn agent(w: Uuid) -> Authority {
    Authority::Agent {
        workspace_id: w,
        scopes: vec![
            "decision:read".into(),
            "decision:propose".into(),
            "learning:read".into(),
            "profile:read".into(),
        ],
    }
}
#[test]
fn import_preview_save_get_delete_preserves_source_only_boundary() {
    let (_dir, mut e, w) = engine();
    let before = e.store.epoch().unwrap();
    let preview = run(&mut e, import(w, true));
    assert_eq!(preview["sources"].as_array().unwrap().len(), 2);
    assert_eq!(e.store.epoch().unwrap(), before);
    assert!(
        e.store
            .list_documents(DocumentKind::Source, w, 100, 0)
            .unwrap()
            .is_empty()
    );
    run(&mut e, import(w, false));
    let after = e.store.epoch().unwrap();
    assert!(after > before);
    let rows = run(&mut e, json!({"operation":"sources","workspace_id":w}));
    for row in rows["sources"].as_array().unwrap() {
        let saved = run(
            &mut e,
            json!({"operation":"source_get","workspace_id":w,"id":row["id"]}),
        );
        assert_eq!(&saved, row);
        assert_eq!(row["payload"]["training_eligible"], false);
        assert_eq!(row["payload"]["review_pending"], true);
    }
    assert!(e.store.list(w, 100, 0).unwrap().is_empty());
    assert!(e.prepare_train(w).is_err());
    run(&mut e, import(w, false));
    assert_eq!(e.store.epoch().unwrap(), after);
    let row = &rows["sources"][0];
    assert!(e.execute(json!({"operation":"source_delete","workspace_id":w,"id":row["id"],"expected_revision":2}),Authority::Human).is_err());
    run(
        &mut e,
        json!({"operation":"source_delete","workspace_id":w,"id":row["id"],"expected_revision":1}),
    );
    run(&mut e, import(w, false));
    assert!(
        e.execute(
            json!({"operation":"source_get","workspace_id":w,"id":row["id"]}),
            Authority::Human
        )
        .is_err()
    );
    assert_eq!(
        run(&mut e, json!({"operation":"sources","workspace_id":w}))["sources"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn agent_cannot_access_onboarding_actions_and_workspace_get_is_scoped() {
    let (_dir, mut e, w) = engine();
    let saved = run(&mut e, import(w, false));
    let id = saved["sources"][0]["source_id"].clone();
    let before = e.store.epoch().unwrap();
    for cmd in [
        import(w, true),
        import(w, false),
        json!({"operation":"sources","workspace_id":w}),
        json!({"operation":"source_get","workspace_id":w,"id":id}),
        json!({"operation":"source_delete","workspace_id":w,"id":id,"expected_revision":1}),
        json!({"operation":"assessments","workspace_id":w}),
        json!({"operation":"assessment_save","workspace_id":w,"id":Uuid::new_v4(),"expected_revision":0,"result":assessment()}),
        json!({"operation":"assessment_delete","workspace_id":w,"id":Uuid::new_v4(),"expected_revision":1}),
        json!({"operation":"select_questions","request":selection(w)}),
    ] {
        assert!(e.execute(cmd.clone(), agent(w)).is_err(), "{cmd}");
    }
    assert_eq!(e.store.epoch().unwrap(), before);
    let other = Uuid::new_v4();
    e.store.create_workspace(other).unwrap();
    assert!(
        e.execute(
            json!({"operation":"source_get","workspace_id":other,"id":id}),
            Authority::Human
        )
        .is_err()
    );
}
#[test]
fn assessments_require_consent_and_never_become_observations() {
    let (_dir, mut e, w) = engine();
    let id = Uuid::new_v4();
    let before = e.store.epoch().unwrap();
    for field in ["store_locally", "mcp_read", "external_send"] {
        let mut value = assessment();
        value["consent"][field] = json!(field != "store_locally");
        assert!(e.execute(json!({"operation":"assessment_save","workspace_id":w,"id":id,"expected_revision":0,"result":value}),Authority::Human).is_err());
    }
    let mut value = assessment();
    value["confirmed_by_user"] = json!(false);
    assert!(e.execute(json!({"operation":"assessment_save","workspace_id":w,"id":id,"expected_revision":0,"result":value}),Authority::Human).is_err());
    assert_eq!(e.store.epoch().unwrap(), before);
    let saved = run(
        &mut e,
        json!({"operation":"assessment_save","workspace_id":w,"id":id,"expected_revision":0,"result":assessment()}),
    );
    assert_eq!(saved["revision"], 1);
    assert_eq!(
        run(&mut e, json!({"operation":"assessments","workspace_id":w}))["assessments"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(e.store.list(w, 100, 0).unwrap().is_empty());
    assert!(e.prepare_train(w).is_err());
    assert_eq!(
        run(&mut e, json!({"operation":"export","workspace_id":w}))["jsonl"],
        ""
    );
    run(
        &mut e,
        json!({"operation":"assessment_delete","workspace_id":w,"id":id,"expected_revision":1}),
    );
    assert!(
        run(&mut e, json!({"operation":"assessments","workspace_id":w}))["assessments"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn selection_enforces_bounds_scope_and_exposure_without_writes() {
    let (_dir, mut e, w) = engine();
    let request = selection(w);
    let before = e.store.epoch().unwrap();
    let selected = run(
        &mut e,
        json!({"operation":"select_questions","request":request}),
    );
    assert_eq!(selected["selected"].as_array().unwrap().len(), 1);
    assert_eq!(
        selected,
        run(
            &mut e,
            json!({"operation":"select_questions","request":request})
        )
    );
    for limit in [0, 11] {
        let mut bad = request.clone();
        bad["limit"] = json!(limit);
        assert!(
            e.execute(
                json!({"operation":"select_questions","request":bad}),
                Authority::Human
            )
            .is_err()
        );
    }
    let mut bad = request.clone();
    bad["candidates"] = json!(vec![request["candidates"][0].clone(); 129]);
    assert!(
        e.execute(
            json!({"operation":"select_questions","request":bad}),
            Authority::Human
        )
        .is_err()
    );
    let mut bad = request.clone();
    bad["candidates"][0]["question"]["workspace_id"] = json!(Uuid::new_v4());
    assert!(
        e.execute(
            json!({"operation":"select_questions","request":bad}),
            Authority::Human
        )
        .is_err()
    );
    let mut seen = request.clone();
    seen["exposures"] = json!([{"question_id":request["candidates"][0]["id"],"family_id":request["candidates"][0]["family_id"],"domain":"product_delivery"}]);
    assert!(
        run(
            &mut e,
            json!({"operation":"select_questions","request":seen})
        )["selected"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(e.store.epoch().unwrap(), before);
}

#[test]
fn deleting_imported_source_cannot_leave_its_confirmed_label_trainable() {
    let (_dir, mut e, w) = engine();
    let batch = run(&mut e, import(w, false));
    let source = batch["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["speaker"] == "human")
        .unwrap();
    let q = rank(w);
    let p = json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":w,"family_id":Uuid::new_v4(),"case_kind":"hypothetical","domain":q["domain"],"context":q["context"],"candidates":q["candidates"],"response":{"response_type":"choose_one","candidate_id":"a"},"source":{"artifact_id":source["source_id"],"source_kind":"imported","occurred_at":null,"observed_at":"2026-09-22T00:00:00Z"},"model_exposure":false});
    let r = run(&mut e, json!({"operation":"propose","proposal":p}));
    run(
        &mut e,
        json!({"operation":"confirm","workspace_id":w,"id":r["id"],"expected_revision":r["revision"],"request_id":Uuid::new_v4()}),
    );
    assert!(e.prepare_train(w).is_ok());
    let before = e.prepare_train(w).unwrap().input;
    run(
        &mut e,
        json!({"operation":"assessment_save","workspace_id":w,"id":Uuid::new_v4(),"expected_revision":0,"result":assessment()}),
    );
    assert_eq!(
        e.prepare_train(w).unwrap().input,
        before,
        "assessment changed observation training input"
    );
    if e.execute(json!({"operation":"source_delete","workspace_id":w,"id":source["source_id"],"expected_revision":1}),Authority::Human).is_ok() {
        assert!(e.prepare_train(w).is_err(),"deleted imported source still supplies confirmed training label");
    }
}
