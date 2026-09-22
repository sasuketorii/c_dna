use cdna_app::{Authority, Engine};
use cdna_store::Store;
use serde_json::{Value, json};
use uuid::Uuid;
use zeroize::Zeroizing;
fn engine() -> (tempfile::TempDir, Engine, Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(dir.path().join("test.db"), &Zeroizing::new("a".repeat(64))).unwrap();
    let mut engine = Engine::new(store, std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../learner"), true);
    let w = engine
        .execute(
            json!({"operation":"workspace_create","name":"test"}),
            Authority::Human,
        )
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    (dir, engine, w)
}
fn proposal(w: Uuid) -> Value {
    json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":w,"family_id":Uuid::new_v4(),"case_kind":"hypothetical","domain":"product_delivery","context":{"as_of":"2026-09-01T00:00:00Z","summary":"納期が短い開発案件","facts":[{"key":"deadline_pressure","value":1.0,"unit":"ratio","evidence_status":"explicit"}],"unknown_fields":[]},"candidates":[{"id":"fast","text":"外注する","attributes":{"speed":1.0,"cost":0.5}},{"id":"slow","text":"内製する","attributes":{"speed":-1.0,"cost":-0.5}}],"response":{"response_type":"choose_one","candidate_id":"fast"},"source":{"artifact_id":Uuid::new_v4(),"source_kind":"human_app","occurred_at":null,"observed_at":"2026-09-01T01:00:00Z"},"model_exposure":false,"rationale_explicit":"納期を優先する","reversal_conditions":["余裕があれば内製する"]})
}
fn request(p: &Value) -> Value {
    json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":p["workspace_id"],"mode":"imitate","domain":p["domain"],"context":p["context"],"candidates":p["candidates"]})
}
fn save(engine: &mut Engine, p: &Value) -> Value {
    let r = engine
        .execute(
            json!({"operation":"propose","proposal":p}),
            Authority::Human,
        )
        .unwrap();
    engine.execute(json!({"operation":"confirm","workspace_id":p["workspace_id"],"id":r["id"],"expected_revision":r["revision"],"request_id":Uuid::new_v4()}),Authority::Human).unwrap()
}

fn revise(e: &mut Engine, w: Uuid, r: &Value, p: &Value) -> anyhow::Result<Value> {
    e.execute(json!({"operation":"revise","workspace_id":w,"id":r["id"],"expected_revision":r["revision"],"request_id":Uuid::new_v4(),"proposal":p}),Authority::Human)
}
fn confirm(e: &mut Engine, w: Uuid, r: &Value) -> anyhow::Result<Value> {
    e.execute(json!({"operation":"confirm","workspace_id":w,"id":r["id"],"expected_revision":r["revision"],"request_id":Uuid::new_v4()}),Authority::Human)
}

#[test]
fn revision_cannot_launder_ai_source_into_training() {
    let (_dir, mut e, w) = engine();
    let mut p = proposal(w);
    p["source"]["source_kind"] = json!("ai_generated");
    let r = e
        .execute(
            json!({"operation":"propose","proposal":p}),
            Authority::Human,
        )
        .unwrap();
    assert!(confirm(&mut e, w, &r).is_err());
    p["source"]["source_kind"] = json!("human_app");
    if let Ok(revised) = revise(&mut e, w, &r, &p) {
        if confirm(&mut e, w, &revised).is_ok() {
            assert!(
                e.prepare_train(w).is_err(),
                "AI source was relabeled human_app and entered training"
            );
        }
    }
}

#[test]
fn revision_cannot_erase_model_exposure() {
    let (_dir, mut e, w) = engine();
    let mut p = proposal(w);
    p["model_exposure"] = json!(true);
    let r = save(&mut e, &p);
    assert!(e.prepare_train(w).is_err());
    p["model_exposure"] = json!(false);
    if let Ok(revised) = revise(&mut e, w, &r, &p) {
        if confirm(&mut e, w, &revised).is_ok() {
            assert!(
                e.prepare_train(w).is_err(),
                "previously exposed answer entered clean training snapshot"
            );
        }
    }
}

#[test]
fn stale_training_completion_and_activation_are_rejected() {
    let (_dir, mut e, w) = engine();
    let p = proposal(w);
    let r = save(&mut e, &p);
    let task = e.prepare_train(w).unwrap();
    let epoch = task.epoch;
    let model_id = Uuid::new_v4();
    e.store
        .save_model(
            w,
            model_id,
            epoch,
            json!({"evaluation":{"promotion_eligible":true}}),
        )
        .unwrap();
    e.execute(json!({"operation":"delete","workspace_id":w,"id":r["id"],"expected_revision":r["revision"]}),Authority::Human).unwrap();
    assert!(
        e.finish_train(task, json!({"ok":true,"model":{},"evaluation":{}}))
            .is_err()
    );
    assert!(
        e.store
            .save_model(w, Uuid::new_v4(), epoch, json!({}))
            .is_err()
    );
    assert!(
        e.execute(
            json!({"operation":"activate_model","workspace_id":w,"id":model_id}),
            Authority::Human
        )
        .is_err()
    );
    assert!(e.store.list_models(w, 100).unwrap().is_empty());
}

#[test]
fn agents_and_record_ids_cannot_cross_workspaces() {
    let (_dir, mut e, w) = engine();
    let other = Uuid::new_v4();
    e.store.create_workspace(other).unwrap();
    let p = proposal(w);
    let r = save(&mut e, &p);
    let id = r["id"].as_str().unwrap().parse().unwrap();
    let agent = Authority::Agent {
        workspace_id: other,
        scopes: vec![
            "decision:read".into(),
            "decision:propose".into(),
            "judgment:infer".into(),
        ],
    };
    for command in [
        json!({"operation":"list","workspace_id":w}),
        json!({"operation":"propose","proposal":proposal(w)}),
        json!({"operation":"rank","request":request(&p)}),
    ] {
        assert!(e.execute(command, agent.clone()).is_err());
    }
    assert!(e.store.get(other, id).is_err());
    let mut replacement = p.clone();
    replacement["workspace_id"] = json!(other);
    assert!(revise(&mut e, other, &r, &replacement).is_err());
    assert_eq!(e.store.get(w, id).unwrap().payload, p);
    let status = e.execute(json!({"operation":"status"}), agent).unwrap();
    assert_eq!(status["workspaces"], json!([other]));
}

#[test]
fn invalid_engine_inputs_leave_epoch_unchanged() {
    let (_dir, mut e, w) = engine();
    let before = e.store.epoch().unwrap();
    let mut bad_type = proposal(w);
    bad_type["model_exposure"] = json!("false");
    let mut oversized = proposal(w);
    oversized["context"]["summary"] = json!("x".repeat(cdna_domain::MAX_REQUEST_BYTES));
    for command in [
        json!({"operation":"propose","proposal":bad_type}),
        json!({"operation":"propose","proposal":oversized}),
        json!({"operation":"list","workspace_id":w,"offset":-1}),
        json!({"operation":"list","workspace_id":w,"authority":"human"}),
        json!({"operation":"status","authority":"human"}),
        json!({"operation":"lock","authority":"human"}),
    ] {
        let result = e.execute(command.clone(), Authority::Human);
        assert!(
            result.is_err(),
            "unexpected success for {command}: {result:?}"
        );
        assert_eq!(e.store.epoch().unwrap(), before);
    }
    assert!(e.store.list(w, 100, 0).unwrap().is_empty());
}

#[test]
fn importer_rejects_deep_json_and_quarantines_oversized_messages() {
    use cdna_app::importer::{MAX_MESSAGE_BYTES, Provider, parse_export};
    let nested = format!("{}0{}", "[".repeat(256), "]".repeat(256));
    assert!(parse_export(Provider::Claude, nested.as_bytes(), &[]).is_err());
    let bytes=serde_json::to_vec(&json!([{"uuid":"c","chat_messages":[{"uuid":"m","sender":"human","text":"x".repeat(MAX_MESSAGE_BYTES+1)}]}])).unwrap();
    let batch = parse_export(Provider::Claude, &bytes, &[]).unwrap();
    assert!(!batch.complete);
    assert!(batch.sources.is_empty());
    assert!(batch.issues.iter().any(|i| i.code == "message_size_limit"));
}

#[test]
fn deleted_imports_and_observations_do_not_resurrect_after_restore() {
    use cdna_app::importer::{Provider, parse_export};
    use cdna_store::DocumentKind;
    let (dir, mut e, w) = engine();
    let p = proposal(w);
    let r = save(&mut e, &p);
    let bytes=br#"[{"uuid":"c","chat_messages":[{"uuid":"m","sender":"assistant","text":"suggestion","training_eligible":true,"review_pending":false}]}]"#;
    let batch = parse_export(Provider::Claude, bytes, &[]).unwrap();
    let source = &batch.sources[0];
    assert!(source.review_pending);
    assert!(!source.training_eligible);
    let document = (source.source_id, serde_json::to_value(source).unwrap());
    e.store
        .insert_documents_batch(DocumentKind::Source, w, vec![document.clone()])
        .unwrap();
    let recovery = Zeroizing::new("b".repeat(64));
    let backup = dir.path().join("backup");
    e.store.backup(&backup, &recovery).unwrap();
    e.store
        .delete_document(DocumentKind::Source, w, source.source_id, 1)
        .unwrap();
    e.execute(json!({"operation":"delete","workspace_id":w,"id":r["id"],"expected_revision":r["revision"]}),Authority::Human).unwrap();
    let mut restored = Store::restore(
        &backup,
        &recovery,
        dir.path().join("restored"),
        &Zeroizing::new("c".repeat(64)),
        &e.store.tombstones().unwrap(),
        &e.store.document_tombstones().unwrap(),
    )
    .unwrap();
    assert!(restored.list(w, 100, 0).unwrap().is_empty());
    assert!(
        restored
            .get_document(DocumentKind::Source, w, source.source_id)
            .is_err()
    );
    assert_eq!(
        restored
            .insert_documents_batch(DocumentKind::Source, w, vec![document])
            .unwrap()
            .inserted,
        0
    );
    assert!(
        restored
            .propose(
                w,
                p["request_id"].as_str().unwrap().parse().unwrap(),
                Uuid::new_v4(),
                p
            )
            .is_err()
    );
}

#[test]
fn revision_cannot_reassign_family_to_leak_evaluation_splits() {
    let (_dir, mut e, w) = engine();
    let mut p = proposal(w);
    let r = save(&mut e, &p);
    p["family_id"] = json!(Uuid::new_v4());
    assert!(revise(&mut e, w, &r, &p).is_err());
}

#[test]
fn store_never_commits_json_it_cannot_read_back() {
    use cdna_store::DocumentKind;
    let (_dir, mut e, w) = engine();
    let mut payload = Value::Null;
    for _ in 0..150 {
        payload = json!([payload]);
    }
    let id = Uuid::new_v4();
    let epoch = e.store.epoch().unwrap();
    let result = e
        .store
        .insert_documents_batch(DocumentKind::Source, w, vec![(id, payload)]);
    if result.is_ok() {
        assert!(
            e.store.get_document(DocumentKind::Source, w, id).is_ok(),
            "Store committed a 304-byte JSON value that its read boundary rejects"
        );
        assert!(
            e.store
                .list_documents(DocumentKind::Source, w, 100, 0)
                .is_ok()
        );
    } else {
        assert_eq!(e.store.epoch().unwrap(), epoch);
    }
}

#[test]
fn importer_does_not_silently_choose_between_conflicting_speakers() {
    use cdna_app::importer::{Provider, parse_export};
    let bytes=br#"[{"uuid":"c","chat_messages":[{"uuid":"m","sender":"assistant","sender":"human","text":"AI answer"}]}]"#;
    if let Ok(batch) = parse_export(Provider::Claude, bytes, &[]) {
        assert!(
            batch.sources.is_empty() && !batch.complete,
            "duplicate speaker key silently became {:?}",
            batch.sources
        );
    }
}

#[test]
fn single_document_write_never_commits_unreadable_json() {
    use cdna_store::DocumentKind;
    let (_dir, mut e, w) = engine();
    let id = Uuid::new_v4();
    let mut payload = Value::Null;
    for _ in 0..150 {
        payload = json!([payload]);
    }
    if e.store
        .put_document(DocumentKind::Source, w, id, 0, payload)
        .is_ok()
    {
        assert!(e.store.get_document(DocumentKind::Source, w, id).is_ok());
    }
}

#[test]
fn record_write_never_commits_unreadable_json() {
    let (_dir, mut e, w) = engine();
    let id = Uuid::new_v4();
    let mut payload = Value::Null;
    for _ in 0..150 {
        payload = json!([payload]);
    }
    let before = e.store.epoch().unwrap();
    if e.store.propose(w, id, Uuid::new_v4(), payload).is_ok() {
        assert!(e.store.get(w, id).is_ok());
    } else {
        assert_eq!(
            e.store.epoch().unwrap(),
            before,
            "rejected write committed a mutation"
        );
        assert!(e.store.list(w, 100, 0).unwrap().is_empty());
    }
}

#[test]
fn model_write_never_commits_unreadable_json() {
    let (_dir, mut e, w) = engine();
    let id = Uuid::new_v4();
    let mut payload = Value::Null;
    for _ in 0..150 {
        payload = json!([payload]);
    }
    if e.store
        .save_model(w, id, e.store.epoch().unwrap(), payload)
        .is_ok()
    {
        assert!(e.store.model(w, id).is_ok());
    }
}

#[test]
fn revision_retains_original_source_identity() {
    let (_dir, mut e, w) = engine();
    let mut p = proposal(w);
    p["source"]["source_kind"] = json!("imported");
    let r = save(&mut e, &p);
    let original = p["source"]["artifact_id"].clone();
    p["source"]["artifact_id"] = json!(Uuid::new_v4());
    p["source"]["source_kind"] = json!("human_app");
    if let Ok(revised) = revise(&mut e, w, &r, &p) {
        assert_eq!(
            revised["payload"]["source"]["artifact_id"], original,
            "revision replaced original source identity without a source lineage link"
        );
        assert_eq!(revised["payload"]["source"]["source_kind"], "imported");
    }
}
