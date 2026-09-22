//! Synthetic snapshot isolation only; no human fidelity or promotion evidence.
use cdna_app::Engine;
use cdna_domain::Domain;
use cdna_store::Store;
use serde_json::{Value, json};
use uuid::Uuid;
use zeroize::Zeroizing;

fn setup() -> (tempfile::TempDir, Engine, Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(dir.path().join("domain.db"), &Zeroizing::new("d".repeat(64))).unwrap();
    let workspace = Uuid::new_v4();
    store.create_workspace(workspace).unwrap();
    (dir, Engine::new(store, "unused".into(), false), workspace)
}

fn insert(engine: &mut Engine, workspace: Uuid, domain: &str) -> Uuid {
    let id = Uuid::new_v4();
    let proposal: Value = json!({"schema_version":"1.0","request_id":Uuid::new_v4(),
        "workspace_id":workspace,"family_id":Uuid::new_v4(),"case_kind":"hypothetical",
        "domain":domain,"context":{"as_of":"2026-09-01T00:00:00Z",
        "summary":"Synthetic scope test","facts":[],"unknown_fields":[]},
        "candidates":[{"id":"a","text":"A","attributes":{}},{"id":"b","text":"B","attributes":{}}],
        "response":{"response_type":"choose_one","candidate_id":"a"},
        "source":{"artifact_id":Uuid::new_v4(),"source_kind":"human_app","occurred_at":null,
        "observed_at":"2026-09-01T01:00:00Z"},"model_exposure":false,
        "rationale_explicit":null,"reversal_conditions":[]});
    engine.store.propose(workspace, id, Uuid::new_v4(), proposal).unwrap();
    engine.store.confirm(workspace, id, 1, Uuid::new_v4()).unwrap();
    id
}

#[test]
fn domain_snapshot_filters_rows_and_lineage_and_preserves_pooled_default() {
    let (_dir, mut engine, workspace) = setup();
    let delivery = insert(&mut engine, workspace, "product_delivery");
    let risk = insert(&mut engine, workspace, "risk_reputation");
    for (domain, id, name) in [(Domain::ProductDelivery, delivery, "product_delivery"),
                              (Domain::RiskReputation, risk, "risk_reputation")] {
        let task = engine.prepare_train_domain(workspace, Some(domain)).unwrap();
        assert_eq!(task.input["domain"], name);
        assert_eq!(task.input["records"].as_array().unwrap().len(), 1);
        assert_eq!(task.input["records"][0]["domain"], name);
        assert_eq!(task.lineage.len(), 1);
        assert_eq!(task.lineage[0]["id"], id.to_string());
        assert_eq!(task.lineage[0]["revision"], 2);
    }
    let pooled = engine.prepare_train(workspace).unwrap();
    assert!(pooled.input["domain"].is_null());
    assert_eq!(pooled.input["records"].as_array().unwrap().len(), 2);
    assert!(engine.prepare_train_domain(workspace, Some(Domain::GrowthStrategy)).is_err());
    assert!(engine.prepare_train_domain(Uuid::new_v4(), Some(Domain::ProductDelivery)).is_err());
}

#[test]
fn unrelated_confirmed_domain_does_not_consume_eligible_cap() {
    let (_dir, mut engine, workspace) = setup();
    for _ in 0..2001 {
        insert(&mut engine, workspace, "risk_reputation");
    }
    let delivery = insert(&mut engine, workspace, "product_delivery");
    let task = engine.prepare_train_domain(workspace, Some(Domain::ProductDelivery)).unwrap();
    assert_eq!(task.input["records"].as_array().unwrap().len(), 1);
    assert_eq!(task.lineage[0]["id"], delivery.to_string());
    assert!(engine.prepare_train(workspace).err().unwrap().to_string().contains("2000 records"));
    assert!(engine.prepare_train_domain(workspace, Some(Domain::RiskReputation))
        .err().unwrap().to_string().contains("2000 records"));
}
