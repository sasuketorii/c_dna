use cdna_app::{Authority, bootstrap::*};
use cdna_store::{DocumentKind, Store};
use serde_json::{Value, json};
use uuid::Uuid;
use zeroize::Zeroizing;
fn profile() -> Value {
    json!({"schema_version":"1.0","provider":"Claude","model":null,"context":{"availability":"partial","limitations":["Only this conversation"]},"criteria":[{"criterion":"Prefer reversible decisions","priority":1,"tradeoffs":["Speed over completeness"],"examples":["Trial before rollout"],"exceptions":[],"reversal_conditions":["Irreversible safety impact"],"evidence":[{"kind":"remembered_summary","statement_category":"reported_policy","text":"Prioritize a bounded trial","reference":null}],"confidence":{"self_report":0.6,"basis":"Uncalibrated remembered summary"}}],"contradictions":[],"unknowns":[]})
}
fn bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap()
}
fn fixture() -> (tempfile::TempDir, Store, Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(dir.path().join("vault"), &Zeroizing::new("a".repeat(64))).unwrap();
    let workspace = Uuid::new_v4();
    store.create_workspace(workspace).unwrap();
    (dir, store, workspace)
}
#[test]
fn strict_parser_rejects_authority_injection_and_malformed_inputs() {
    assert!(parse_profile(&bytes(&profile())).is_ok());
    for field in [
        "workspace_id",
        "namespace",
        "authority",
        "training_eligible",
        "heldout_eligible",
        "confirmed",
    ] {
        let mut v = profile();
        v[field] = json!(true);
        assert!(parse_profile(&bytes(&v)).is_err(), "{field}");
    }
    for input in [
        b"{}".as_slice(),
        b"null",
        b"{\"schema_version\":\"1.0\",\"schema_version\":\"1.0\"}",
        b"{} {}",
    ] {
        assert!(parse_profile(input).is_err());
    }
    let duplicate = String::from_utf8(bytes(&profile())).unwrap().replace(
        "\"self_report\":0.6",
        "\"self_report\":0.6,\"self_report\":0.8",
    );
    assert!(parse_profile(duplicate.as_bytes()).is_err());
    assert!(parse_profile(format!("{}0{}", "[".repeat(256), "]".repeat(256)).as_bytes()).is_err());
    assert!(parse_profile(&vec![b' '; MAX_PROFILE_BYTES + 1]).is_err());
    let mut v = profile();
    v["criteria"][0]["evidence"][0]["confirmed"] = json!(true);
    assert!(parse_profile(&bytes(&v)).is_err());
}
#[test]
fn explicit_unknown_nullable_and_bounds_contract() {
    let mut v = profile();
    v["criteria"][0]["evidence"] = json!([]);
    assert!(parse_profile(&bytes(&v)).is_err());
    v["unknowns"] = json!(["Evidence unavailable"]);
    assert!(parse_profile(&bytes(&v)).is_ok());
    v["criteria"] = json!([]);
    assert!(parse_profile(&bytes(&v)).is_ok());
    for pointer in [
        "/provider",
        "/model",
        "/criteria/0/priority",
        "/criteria/0/confidence/self_report",
        "/criteria/0/evidence/0/reference",
    ] {
        let mut v = profile();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        v.pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert!(parse_profile(&bytes(&v)).is_err(), "{pointer}");
    }
    for (pointer, value) in [
        ("/schema_version", json!("2.0")),
        ("/criteria/0/priority", json!(0)),
        ("/criteria/0/confidence/self_report", json!(1.1)),
        ("/criteria/0/criterion", json!("x".repeat(2049))),
        ("/context/limitations", json!(vec!["x"; 33])),
    ] {
        let mut v = profile();
        *v.pointer_mut(pointer).unwrap() = value;
        assert!(parse_profile(&bytes(&v)).is_err(), "{pointer}");
    }
}
#[test]
fn preview_import_revision_consent_deletion_and_workspace_isolation() {
    let (_dir, mut store, w) = fixture();
    let namespace = Uuid::new_v4();
    let input = bytes(&profile());
    let agent = Authority::Agent {
        workspace_id: w,
        scopes: vec!["*".into()],
    };
    assert!(import_profile(&mut store, &agent, w, namespace, &input, false).is_err());
    assert!(
        import_profile(
            &mut store,
            &Authority::Human,
            Uuid::new_v4(),
            namespace,
            &input,
            false
        )
        .is_err()
    );
    let preview =
        import_profile(&mut store, &Authority::Human, w, namespace, &input, true).unwrap();
    assert!(
        store
            .list_documents(DocumentKind::Source, w, 100, 0)
            .unwrap()
            .is_empty()
    );
    let imported =
        import_profile(&mut store, &Authority::Human, w, namespace, &input, false).unwrap();
    assert_eq!(imported.source_id, preview.source_id);
    assert_eq!(imported.inserted, 1);
    let id = imported.source_id;
    assert_eq!(imported.source.authority, "ai_generated_hypothesis");
    assert!(!imported.source.training_eligible && !imported.source.heldout_eligible);
    assert_eq!(teacher_context(&store, w).unwrap(), json!([]));
    let retry = import_profile(
        &mut store,
        &Authority::Human,
        w,
        namespace,
        &serde_json::to_vec_pretty(&profile()).unwrap(),
        false,
    )
    .unwrap();
    assert_eq!(retry.source_id, id);
    assert_eq!(retry.skipped, 1);
    assert!(set_teacher_sharing(&mut store, &agent, w, id, 1, true).is_err());
    let shared = set_teacher_sharing(&mut store, &Authority::Human, w, id, 1, true).unwrap();
    assert_eq!(shared.revision, 2);
    let context = teacher_context(&store, w).unwrap();
    assert_eq!(context[0]["revision"], 2);
    assert_eq!(context[0]["family_id"], json!(id));
    let mut corrected = profile();
    corrected["criteria"][0]["criterion"] = json!("User corrected hypothesis");
    assert!(revise_profile(&mut store, &agent, w, id, 2, &bytes(&corrected)).is_err());
    assert!(revise_profile(&mut store, &Authority::Human, w, id, 1, &bytes(&corrected)).is_err());
    let revised =
        revise_profile(&mut store, &Authority::Human, w, id, 2, &bytes(&corrected)).unwrap();
    assert_eq!(revised.revision, 3);
    assert_eq!(revised.payload["human_corrected"], true);
    assert_eq!(revised.payload["authority"], "ai_generated_hypothesis");
    assert_eq!(teacher_context(&store, w).unwrap(), json!([]));
    import_profile(&mut store, &Authority::Human, w, namespace, &input, false).unwrap();
    assert_eq!(
        store
            .get_document(DocumentKind::Source, w, id)
            .unwrap()
            .revision,
        3
    );
    set_teacher_sharing(&mut store, &Authority::Human, w, id, 3, true).unwrap();
    set_teacher_sharing(&mut store, &Authority::Human, w, id, 4, false).unwrap();
    assert_eq!(teacher_context(&store, w).unwrap(), json!([]));
    let other = Uuid::new_v4();
    store.create_workspace(other).unwrap();
    let isolated = import_profile(
        &mut store,
        &Authority::Human,
        other,
        namespace,
        &input,
        false,
    )
    .unwrap();
    assert_ne!(isolated.source_id, id);
    assert!(revise_profile(&mut store, &Authority::Human, other, id, 5, &input).is_err());
    store.delete_source_cascade(w, id, 5).unwrap();
    assert_eq!(teacher_context(&store, w).unwrap(), json!([]));
    let deleted_retry =
        import_profile(&mut store, &Authority::Human, w, namespace, &input, false).unwrap();
    assert_eq!(deleted_retry.inserted, 0);
    assert_eq!(deleted_retry.skipped, 1);
    assert!(store.get_document(DocumentKind::Source, w, id).is_err());
    assert!(revise_profile(&mut store, &Authority::Human, w, id, 5, &input).is_err());
    assert!(
        store
            .get_document(DocumentKind::Source, other, isolated.source_id)
            .is_ok()
    );
}

#[test]
fn teacher_pages_past_unrelated_history_without_leaking_it() {
    let (_dir, mut store, w) = fixture();
    let unrelated = (0..1001)
        .map(|_| {
            (
                Uuid::new_v4(),
                json!({"text":"private unrelated conversation"}),
            )
        })
        .collect::<Vec<_>>();
    store
        .insert_documents_batch(DocumentKind::Source, w, unrelated[..1000].to_vec())
        .unwrap();
    store
        .insert_documents_batch(DocumentKind::Source, w, unrelated[1000..].to_vec())
        .unwrap();
    let imported = import_profile(
        &mut store,
        &Authority::Human,
        w,
        Uuid::new_v4(),
        &bytes(&profile()),
        false,
    )
    .unwrap();
    set_teacher_sharing(
        &mut store,
        &Authority::Human,
        w,
        imported.source_id,
        1,
        true,
    )
    .unwrap();
    let context = teacher_context(&store, w).unwrap();
    assert_eq!(context.as_array().unwrap().len(), 1);
    assert_eq!(context[0]["source_id"], json!(imported.source_id));
    assert!(!context.to_string().contains("private unrelated"));
}
