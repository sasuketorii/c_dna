//! Application-boundary policy lifecycle: persisted documents, authority and ranking.
use cdna_app::{Authority, Engine};
use cdna_store::Store;
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use uuid::Uuid;
use zeroize::Zeroizing;

fn fixture() -> (tempfile::TempDir, Engine, Uuid, Value) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(
        dir.path().join("policy.db"),
        &Zeroizing::new("a".repeat(64)),
    )
    .unwrap();
    let mut engine = Engine::new(store, "unused".into(), true);
    let w = Uuid::new_v4();
    engine.store.create_workspace(w).unwrap();
    let request = json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":w,"mode":"imitate","domain":"product_delivery","context":{"as_of":Utc::now(),"summary":"A hypothetical delivery choice","facts":[{"key":"deadline_pressure","value":1.0,"unit":"ratio","evidence_status":"explicit"}],"unknown_fields":[]},"candidates":[{"id":"expensive","text":"Buy delivery","attributes":{"cost":0.8}},{"id":"economical","text":"Build delivery","attributes":{"cost":0.2}}],"include_evidence":true,"allow_cloud":false});
    let proposal = json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":w,"family_id":Uuid::new_v4(),"case_kind":"hypothetical","domain":request["domain"],"context":request["context"],"candidates":request["candidates"],"response":{"response_type":"choose_one","candidate_id":"expensive"},"source":{"artifact_id":Uuid::new_v4(),"source_kind":"human_app","occurred_at":null,"observed_at":Utc::now()},"model_exposure":false});
    let record = engine
        .execute(
            json!({"operation":"propose","proposal":proposal}),
            Authority::Human,
        )
        .unwrap();
    engine.execute(json!({"operation":"confirm","workspace_id":w,"id":record["id"],"expected_revision":record["revision"],"request_id":Uuid::new_v4()}), Authority::Human).unwrap();
    (dir, engine, w, request)
}
fn policy() -> Value {
    json!({"statement":"When deadlines matter, prefer a cost ratio no higher than 0.5.","when":{"op":"compare","field":"deadline_pressure","comparison":"gt","value":0.0,"unit":"ratio"},"requires":{"op":"compare","field":"cost","comparison":"lte","value":0.5,"unit":"ratio"},"effective_from":Utc::now()-Duration::days(1),"expires_at":Utc::now()+Duration::days(1)})
}
fn agent(w: Uuid, scopes: &[&str]) -> Authority {
    Authority::Agent {
        workspace_id: w,
        scopes: scopes.iter().map(|s| (*s).into()).collect(),
    }
}
fn draft(engine: &mut Engine, w: Uuid, p: Value) -> Value {
    engine
        .execute(
            json!({"operation":"propose_policy","workspace_id":w,"policy":p}),
            Authority::Human,
        )
        .unwrap()
}
fn approve(engine: &mut Engine, w: Uuid, doc: &Value) -> Value {
    engine.execute(json!({"operation":"approve_policy","workspace_id":w,"id":doc["id"],"expected_revision":doc["revision"]}), Authority::Human).unwrap()
}
fn check(engine: &mut Engine, request: &Value) -> Value {
    engine
        .execute(
            json!({"operation":"check_policy","request":request}),
            Authority::Human,
        )
        .unwrap()
}
fn rank(engine: &mut Engine, request: &Value) -> Value {
    engine
        .execute(
            json!({"operation":"rank","request":request}),
            Authority::Human,
        )
        .unwrap()
}

#[test]
fn draft_approval_rank_gate_and_revocation_use_persisted_revisions() {
    let (_dir, mut e, w, request) = fixture();
    let draft = draft(&mut e, w, policy());
    assert_eq!(draft["payload"]["status"], "draft");
    assert_eq!(
        check(&mut e, &request)["candidates"][0]["status"],
        "no_applicable_policy"
    );
    assert_eq!(rank(&mut e, &request)["selected_candidate_id"], "expensive");

    let approved = approve(&mut e, w, &draft);
    assert_eq!(approved["payload"]["status"], "approved");
    assert!(approved["revision"].as_u64().unwrap() > draft["revision"].as_u64().unwrap());
    let checked = check(&mut e, &request);
    assert_eq!(checked["candidates"][0]["status"], "violated");
    assert_eq!(checked["candidates"][1]["status"], "compliant");
    assert_eq!(checked["execution_authorization"], "none");
    let ranked = rank(&mut e, &request);
    assert_eq!(ranked["abstained"], true);
    assert!(ranked["selected_candidate_id"].is_null());
    assert_eq!(ranked["ranking"][0]["policy_status"], "violated");
    assert!(
        ranked["abstention_reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("policy_violation_or_unknown"))
    );
    assert_eq!(ranked["execution_authorization"], "none");

    assert!(e.execute(json!({"operation":"revoke_policy","workspace_id":w,"id":approved["id"],"expected_revision":draft["revision"]}), Authority::Human).is_err());
    assert_eq!(
        check(&mut e, &request)["candidates"][0]["status"],
        "violated"
    );
    e.execute(json!({"operation":"revoke_policy","workspace_id":w,"id":approved["id"],"expected_revision":approved["revision"]}), Authority::Human).unwrap();
    assert_eq!(
        check(&mut e, &request)["candidates"][0]["status"],
        "no_applicable_policy"
    );
    assert_eq!(rank(&mut e, &request)["selected_candidate_id"], "expensive");
    assert!(e.execute(json!({"operation":"approve_policy","workspace_id":w,"id":approved["id"],"expected_revision":approved["revision"]}), Authority::Human).is_err());
}

#[test]
fn agent_scopes_and_workspace_never_authorize_policy_mutations() {
    let (_dir, mut e, w, request) = fixture();
    let draft = draft(&mut e, w, policy());
    let reader = agent(w, &["policy:read", "judgment:infer"]);
    for command in [
        json!({"operation":"propose_policy","workspace_id":w,"policy":policy()}),
        json!({"operation":"approve_policy","workspace_id":w,"id":draft["id"],"expected_revision":draft["revision"]}),
        json!({"operation":"revoke_policy","workspace_id":w,"id":draft["id"],"expected_revision":draft["revision"]}),
    ] {
        assert!(
            e.execute(command, reader.clone())
                .unwrap_err()
                .to_string()
                .contains("SCOPE_DENIED")
        );
    }
    approve(&mut e, w, &draft);
    assert!(
        e.execute(
            json!({"operation":"policies","workspace_id":w}),
            reader.clone()
        )
        .is_ok()
    );
    assert_eq!(
        e.execute(
            json!({"operation":"check_policy","request":request}),
            reader.clone()
        )
        .unwrap()["candidates"][0]["status"],
        "violated"
    );
    assert_eq!(
        e.execute(json!({"operation":"rank","request":request}), reader)
            .unwrap()["abstained"],
        true
    );
    for authority in [
        agent(w, &["decision:read"]),
        agent(Uuid::new_v4(), &["policy:read"]),
    ] {
        for command in [
            json!({"operation":"policies","workspace_id":w}),
            json!({"operation":"check_policy","request":request}),
        ] {
            assert!(
                e.execute(command, authority.clone())
                    .unwrap_err()
                    .to_string()
                    .contains("SCOPE_DENIED")
            );
        }
    }
}

#[test]
fn unknown_inputs_are_not_compliant_and_invalid_contracts_do_not_write() {
    let (_dir, mut e, w, mut request) = fixture();
    let draft = draft(&mut e, w, policy());
    approve(&mut e, w, &draft);
    request["context"]["facts"][0]["value"] = Value::Null;
    request["context"]["facts"][0]["evidence_status"] = json!("unknown");
    request["context"]["unknown_fields"] = json!(["deadline_pressure"]);
    let result = check(&mut e, &request);
    assert!(
        result["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["status"] == "unknown")
    );
    assert_eq!(rank(&mut e, &request)["abstained"], true);
    request["context"]["facts"][0]["value"] = json!(1.0);
    request["context"]["facts"][0]["evidence_status"] = json!("explicit");
    request["context"]["unknown_fields"] = json!([]);
    request["candidates"][0]["attributes"] = json!({});
    assert_eq!(
        check(&mut e, &request)["candidates"][0]["status"],
        "unknown"
    );
    let before = e
        .execute(
            json!({"operation":"policies","workspace_id":w}),
            Authority::Human,
        )
        .unwrap();
    let mut injected = policy();
    injected["status"] = json!("approved");
    assert!(
        e.execute(
            json!({"operation":"propose_policy","workspace_id":w,"policy":injected}),
            Authority::Human
        )
        .is_err()
    );
    let mut bad_period = policy();
    bad_period["expires_at"] = bad_period["effective_from"].clone();
    assert!(
        e.execute(
            json!({"operation":"propose_policy","workspace_id":w,"policy":bad_period}),
            Authority::Human
        )
        .is_err()
    );
    assert_eq!(
        e.execute(
            json!({"operation":"policies","workspace_id":w}),
            Authority::Human
        )
        .unwrap(),
        before
    );
}

#[test]
fn impossible_ordered_policy_comparison_is_rejected_before_persistence() {
    let (_dir, mut e, w, request) = fixture();
    let mut invalid = policy();
    invalid["requires"] =
        json!({"op":"compare","field":"cost","comparison":"gt","value":"cheap","unit":null});
    assert!(
        e.execute(
            json!({"operation":"propose_policy","workspace_id":w,"policy":invalid}),
            Authority::Human
        )
        .is_err(),
        "an ordered string comparison can never evaluate successfully and must not become an approved policy"
    );
    let documents = e
        .execute(
            json!({"operation":"policies","workspace_id":w}),
            Authority::Human,
        )
        .unwrap();
    assert_eq!(documents["policies"], json!([]));
    assert_eq!(rank(&mut e, &request)["selected_candidate_id"], "expensive");
}
