use cdna_app::{Authority,Engine};
use cdna_store::Store;
use serde_json::{json,Value};
use uuid::Uuid;
use zeroize::Zeroizing;
fn engine()->(tempfile::TempDir,Engine,Uuid) {
    let dir=tempfile::tempdir().unwrap();
    let store=Store::create(dir.path().join("test.db"),&Zeroizing::new("a".repeat(64))).unwrap();
    let mut engine=Engine::new(store,std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../learner"),true);
    let w=engine.execute(json!({"operation":"workspace_create","name":"test"}),Authority::Human).unwrap()["id"].as_str().unwrap().parse().unwrap();
    (dir,engine,w)
}
fn proposal(w:Uuid)->Value {
    json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":w,"family_id":Uuid::new_v4(),"case_kind":"hypothetical","domain":"product_delivery","context":{"as_of":"2026-09-01T00:00:00Z","summary":"納期が短い開発案件","facts":[{"key":"deadline_pressure","value":1.0,"unit":"ratio","evidence_status":"explicit"}],"unknown_fields":[]},"candidates":[{"id":"fast","text":"外注する","attributes":{"speed":1.0,"cost":0.5}},{"id":"slow","text":"内製する","attributes":{"speed":-1.0,"cost":-0.5}}],"response":{"response_type":"choose_one","candidate_id":"fast"},"source":{"artifact_id":Uuid::new_v4(),"source_kind":"human_app","occurred_at":null,"observed_at":"2026-09-01T01:00:00Z"},"model_exposure":false,"rationale_explicit":"納期を優先する","reversal_conditions":["余裕があれば内製する"]})
}
fn request(p:&Value)->Value {json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":p["workspace_id"],"mode":"imitate","domain":p["domain"],"context":p["context"],"candidates":p["candidates"]})}
fn save(engine:&mut Engine,p:&Value)->Value {
    let r=engine.execute(json!({"operation":"propose","proposal":p}),Authority::Human).unwrap();
    engine.execute(json!({"operation":"confirm","workspace_id":p["workspace_id"],"id":r["id"],"expected_revision":r["revision"],"request_id":Uuid::new_v4()}),Authority::Human).unwrap()
}
#[test]
fn learn_recall_correct_delete_and_restore() {
    let (dir,mut e,w)=engine();let p=proposal(w);let r=save(&mut e,&p);
    let query=request(&p);
    let recalled=e.execute(json!({"operation":"rank","request":query}),Authority::Human).unwrap();
    assert_eq!(recalled["selected_candidate_id"],"fast");assert_eq!(recalled["execution_authorization"],"none");
    let model=e.execute(json!({"operation":"train","workspace_id":w}),Authority::Human).unwrap();
    let preview=e.execute(json!({"operation":"preview_model","workspace_id":w,"id":model["id"],"request":query}),Authority::Human).unwrap();
    assert_eq!(preview["ranking"][0]["candidate_id"],"fast");assert_eq!(preview["probability"],Value::Null);assert_eq!(preview["provisional"],true);
    assert!(e.execute(json!({"operation":"activate_model","workspace_id":w,"id":model["id"]}),Authority::Human).is_err());
    e.store.backup(dir.path().join("backup.db"),&Zeroizing::new("b".repeat(64))).unwrap();
    e.execute(json!({"operation":"delete","workspace_id":w,"id":r["id"],"expected_revision":r["revision"]}),Authority::Human).unwrap();
    assert!(e.execute(json!({"operation":"preview_model","workspace_id":w,"id":model["id"],"request":query}),Authority::Human).is_err());
    let restored=Store::restore(dir.path().join("backup.db"),&Zeroizing::new("b".repeat(64)),dir.path().join("restored.db"),&Zeroizing::new("c".repeat(64)),&e.store.tombstones().unwrap(),&e.store.document_tombstones().unwrap()).unwrap();
    assert!(restored.list(w,100,0).unwrap().is_empty());
}
#[test]
fn agent_cannot_confirm_or_launder_ai_labels() {
    let (_dir,mut e,w)=engine();let mut p=proposal(w);p["source"]["source_kind"]=json!("ai_generated");
    let agent=Authority::Agent{workspace_id:w,scopes:vec!["decision:propose".into()]};
    let r=e.execute(json!({"operation":"propose","proposal":p}),agent.clone()).unwrap();
    assert_eq!(r["payload"]["source"]["source_kind"],"ai_generated");
    let c=json!({"operation":"confirm","workspace_id":w,"id":r["id"],"expected_revision":1,"request_id":Uuid::new_v4()});
    assert!(e.execute(c.clone(),agent.clone()).is_err());assert!(e.execute(c,Authority::Human).is_err());
    assert!(e.execute(json!({"operation":"list","workspace_id":Uuid::new_v4()}),agent).is_err());
}
#[test]
fn conflicting_and_ambiguous_memories_abstain() {
    let (_dir,mut e,w)=engine();let p=proposal(w);save(&mut e,&p);
    let mut other=proposal(w);other["response"]["candidate_id"]=json!("slow");save(&mut e,&other);
    let result=e.execute(json!({"operation":"rank","request":request(&p)}),Authority::Human).unwrap();
    assert_eq!(result["abstained"],true);assert_eq!(result["selected_candidate_id"],Value::Null);
}
