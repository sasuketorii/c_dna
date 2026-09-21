use super::*;
use domain::*;
use store::{Store,random_key};
use policy::{Expr,PolicyProposal};
use serde_json::{json,Value};
use std::sync::Arc;
use zeroize::Zeroizing;

fn setup()->(tempfile::TempDir,Store,Workspace){
    let temp=tempfile::tempdir().unwrap();let vid=id();let mut store=Store::open(&temp.path().join(format!("{vid}.db")),&vid,random_key(),true).unwrap();let ws=store.create_workspace("検証用",false).unwrap();(temp,store,ws)
}
fn proposal(w:&str)->ObservationProposal{
    let t=questions::templates().unwrap().remove(1);
    ObservationProposal{schema_version:"1.0".into(),request_id:id(),workspace_id:w.into(),family_id:id(),case_kind:"hypothetical".into(),domain:t.domain,context:Context{as_of:"2024-01-01T00:00:00Z".into(),summary:"検証用の架空判断。機密ではない。".into(),facts:vec![Fact{key:"deadline_days".into(),value:json!(7),evidence_status:"explicit".into(),unit:Some("day".into())},Fact{key:"customer_impact".into(),value:json!("high"),evidence_status:"explicit".into(),unit:None}],unknown_fields:vec![]},candidates:t.candidates,response:None,source:Source{kind:"manual".into(),reference:"test-fixture".into(),quote:String::new()},model_exposure:false}
}
fn answered(store:&mut Store,w:&str)->CaseRecord{
    let c=store.propose(&proposal(w),"manual_proposal","owner").unwrap();let e=store.expose(w,&c.id,"unit").unwrap();
    store.answer(&AnswerRequest{request_id:id(),workspace_id:w.into(),case_id:c.id,exposure_id:e.id,expected_revision:1,response:Response{kind:"choose_one".into(),selected_ids:vec![c.candidates[0].id.clone()],value:None,note:"条件に基づく回答".into(),reversal_condition:"期限が長いときは再検討する".into()}}).unwrap()
}
fn model_fixture(snapshot:&Value)->Value{json!({"schema_version":"1.0","id":id(),"workspace_id":snapshot["workspace_id"],"snapshot_id":snapshot["snapshot_id"],"deletion_epoch":snapshot["deletion_epoch"],"feature_version":"1.0","feature_manifest_sha256":features::manifest_hash(),"feature_names":features::names(),"kind":"linear_pairwise","dtype":"float64","weights":vec![0.1;features::dimension()],"intercept":0.0,"is_demo":snapshot["is_demo"],"domain_support":{"product_delivery":3},"training":{},"evaluation":{"person_agreement_verified":false},"calibration":{"status":"insufficient_data","models":[]}})}
fn active_model(store:&mut Store,w:&str)->String{let snapshot=store.snapshot(w).unwrap();let job=store.new_job(w,"owner","train").unwrap();let jid=job["id"].as_str().unwrap();store.set_job(w,jid,"running",None).unwrap();let mid=store.accept_model(w,jid,model_fixture(&snapshot)).unwrap();store.activate_model(w,&mid,store.epoch(w).unwrap()).unwrap();mid}
#[test]
fn strict_json_rejects_duplicate_nonfinite_and_deep_values(){
    for raw in [r#"{"a":1,"a":2}"#,r#"{"a":NaN}"#,r#"{"a":Infinity}"#,r#"{"a":1}true"#]{assert!(transport::strict_json(raw.as_bytes()).is_err());}
    let raw=format!("{}0{}","[".repeat(25),"]".repeat(25));assert!(transport::strict_json(raw.as_bytes()).is_err());
    assert_eq!(transport::strict_json(br#"{"a":0,"b":false,"c":null}"#).unwrap()["a"],0);
}
#[test]
fn rank_contract_and_authority_fields_are_strict(){
    let v:Value=serde_json::from_str(include_str!("../../../contracts/fixtures/rank-request.json")).unwrap();assert!(RankRequest::parse(v.clone()).is_ok());
    let mut forged=v.clone();forged["verification_state"]=json!("confirmed");assert!(RankRequest::parse(forged).is_err());
    let mut duplicate=v.clone();duplicate["candidates"][1]["id"]=duplicate["candidates"][0]["id"].clone();assert!(RankRequest::parse(duplicate).is_err());
    let mut bad=v;bad["context"]["facts"][0]["evidence_status"]=json!("unknown");assert!(RankRequest::parse(bad).is_err());
}
#[test]
fn python_rust_feature_parity(){
    let fixtures:Value=serde_json::from_str(include_str!("../../../contracts/fixtures/feature-parity.json")).unwrap();
    assert_eq!(fixtures["feature_names"],json!(features::names()));assert_eq!(fixtures["manifest_sha256"],features::manifest_hash());
    let weights:Vec<f64>=serde_json::from_value(fixtures["weights"].clone()).unwrap();
    for f in fixtures["cases"].as_array().unwrap(){
        let rows=features::transform(f["domain"].as_str().unwrap(),&serde_json::from_value(f["context"].clone()).unwrap(),&serde_json::from_value::<Vec<Candidate>>(f["candidates"].clone()).unwrap()).unwrap();
        let expected:Vec<Vec<f64>>=serde_json::from_value(f["features"].clone()).unwrap();
        for (actual,want)in rows.iter().flatten().zip(expected.iter().flatten()){assert!((actual-want).abs()<1e-12);}
        for(actual,want)in features::score_rows(&weights,&rows).unwrap().iter().zip(f["scores"].as_array().unwrap()){assert!((actual-want.as_f64().unwrap()).abs()<1e-10);}
    }
}
#[test]
fn missing_is_not_zero_and_units_are_required(){let mut p=proposal(&id());let zero=Fact{key:"resource_pressure".into(),value:json!(0),evidence_status:"explicit".into(),unit:None};p.context.facts.push(zero);let a=features::transform(&p.domain,&p.context,&p.candidates).unwrap();p.context.facts.pop();let b=features::transform(&p.domain,&p.context,&p.candidates).unwrap();assert_ne!(a,b);p.context.facts[0].unit=None;assert_eq!(features::transform(&p.domain,&p.context,&p.candidates).unwrap_err().code,"UNIT_REQUIRED");}
#[test]
fn vault_and_wal_are_not_plaintext_and_wrong_keys_fail(){
    let(temp,mut store,ws)=setup();let mut p=proposal(&ws.id);p.context.summary="CANARY_NEVER_PLAINTEXT_7319".into();store.propose(&p,"manual_proposal","owner").unwrap();
    let path=store.path.clone();let vid=store.vault_id.clone();
    for entry in std::fs::read_dir(temp.path()).unwrap(){let bytes=std::fs::read(entry.unwrap().path()).unwrap();assert!(!String::from_utf8_lossy(&bytes).contains("CANARY_NEVER_PLAINTEXT_7319"));}
    assert_ne!(&std::fs::read(&path).unwrap()[..16],b"SQLite format 3\0");drop(store);
    assert!(Store::open(&path,&vid,random_key(),false).is_err());
}
#[test]
fn vault_has_one_writer(){let(_temp,store,_)=setup();assert!(Store::open(&store.path,&store.vault_id,random_key(),false).is_err());}
#[test]
fn observation_confirmation_cannot_be_forged(){let p=proposal(&id());let mut v=serde_json::to_value(p).unwrap();v["verified"]=json!(true);assert!(ObservationProposal::parse(v).is_err());}
#[test]
fn proposal_and_answer_are_idempotent(){let(_temp,mut store,ws)=setup();let p=proposal(&ws.id);let c=store.propose(&p,"agent_proposal","connection").unwrap();assert_eq!(store.propose(&p,"agent_proposal","connection").unwrap().id,c.id);let e=store.expose(&ws.id,&c.id,"unit").unwrap();let r=AnswerRequest{request_id:id(),workspace_id:ws.id.clone(),case_id:c.id.clone(),exposure_id:e.id,expected_revision:1,response:Response{kind:"choose_one".into(),selected_ids:vec![c.candidates[0].id.clone()],value:None,note:String::new(),reversal_condition:String::new()}};let a=store.answer(&r).unwrap();assert_eq!(store.answer(&r).unwrap().revision,a.revision);let mut changed=r;changed.response.note="changed".into();assert_eq!(store.answer(&changed).unwrap_err().code,"CONFLICT");}
#[test]
fn workspace_scope_and_foreign_source_are_enforced(){let(_temp,mut store,ws)=setup();let c=answered(&mut store,&ws.id);let other=store.create_workspace("別の領域",false).unwrap();assert!(store.case(&other.id,&c.id).is_err());let mut p=proposal(&other.id);p.source.reference=format!("cdna-source:{}",id());assert!(store.propose(&p,"agent_proposal","x").is_err());}
#[test]
fn pairwise_uses_the_displayed_order(){let mut p=proposal(&id());p.candidates.truncate(2);let order=vec![p.candidates[1].id.clone(),p.candidates[0].id.clone()];let mut r=Response{kind:"pairwise".into(),selected_ids:vec![order[0].clone()],value:Some("left".into()),note:String::new(),reversal_condition:String::new()};assert!(r.validate(&p.candidates,&order).is_ok());r.selected_ids=vec![order[1].clone()];assert!(r.validate(&p.candidates,&order).is_err());r.value=Some("tie".into());r.selected_ids.clear();assert!(r.validate(&p.candidates,&order).is_ok());assert!(!r.learnable());}
#[test]
fn skip_is_recorded_but_not_a_training_label(){let(_temp,mut store,ws)=setup();let c=store.propose(&proposal(&ws.id),"manual_proposal","owner").unwrap();let e=store.expose(&ws.id,&c.id,"unit").unwrap();let r=AnswerRequest{request_id:id(),workspace_id:ws.id.clone(),case_id:c.id,exposure_id:e.id,expected_revision:1,response:Response{kind:"skip".into(),selected_ids:vec![],value:None,note:String::new(),reversal_condition:String::new()}};store.answer(&r).unwrap();assert_eq!(store.snapshot(&ws.id).unwrap_err().code,"INSUFFICIENT_TRAINING_DATA");}
#[test]
fn accepted_summary_is_a_statement_not_a_training_label(){let(_temp,mut store,ws)=setup();let parsed=imports::parse_summary("速度を大切にするという初期仮説".as_bytes()).unwrap();let imported=imports::ingest(&mut store,&ws.id,parsed).unwrap();let c=store.case(&ws.id,imported["case_ids"][0].as_str().unwrap()).unwrap();let c=store.confirm(&ws.id,&c.id,c.revision).unwrap();assert_eq!(c.origin,"endorsed_statement");assert!(store.snapshot(&ws.id).is_err());}
#[test]
fn synthetic_answers_cannot_enter_a_personal_workspace(){let(_temp,mut store,ws)=setup();assert_eq!(store.propose(&proposal(&ws.id),"synthetic_seed","owner").unwrap_err().code,"SCOPE_DENIED");}
#[test]
fn demo_is_separate_and_carries_no_real_person_claim(){let(_temp,mut store,_)=setup();let ws=questions::seed_demo(&mut store).unwrap();assert!(ws.is_demo);let snapshot=store.snapshot(&ws.id).unwrap();assert_eq!(snapshot["records"].as_array().unwrap().len(),72);assert!(snapshot["records"].as_array().unwrap().iter().all(|r|r["origin"]=="synthetic_seed"));}
#[test]
fn cancel_and_epoch_checks_reject_late_models(){let(_temp,mut store,ws)=setup();let c=answered(&mut store,&ws.id);let s=store.snapshot(&ws.id).unwrap();let j=store.new_job(&ws.id,"owner","train").unwrap();let jid=j["id"].as_str().unwrap();store.set_job(&ws.id,jid,"running",None).unwrap();store.delete_case(&ws.id,&c.id,c.revision).unwrap();assert!(store.accept_model(&ws.id,jid,model_fixture(&s)).is_err());assert!(store.active_model(&ws.id).unwrap().is_none());}
#[test]
fn corrections_erase_derived_weights_and_invalidate_active_pointer(){let(_temp,mut store,ws)=setup();let c=answered(&mut store,&ws.id);let mid=active_model(&mut store,&ws.id);assert!(store.active_model(&ws.id).unwrap().is_some());let mut context=c.context.clone();context.facts[0].value=json!(28);let revised=store.revise_case(&ws.id,&c.id,c.revision,context,c.candidates,"correction").unwrap();assert_eq!(revised.verification_state,"pending");assert!(store.active_model(&ws.id).unwrap().is_none());let raw:String=store.conn.query_row("SELECT body FROM models WHERE id=?1",[mid],|r|r.get(0)).unwrap();assert_eq!(raw,"{}");assert!(store.snapshot(&ws.id).is_err());}
#[test]
fn undo_creates_pending_revision_without_learning_it(){let(_temp,mut store,ws)=setup();let c=answered(&mut store,&ws.id);let new=store.undo(&ws.id,&c.id,c.revision).unwrap();assert_eq!(new.revision,c.revision+1);assert_eq!(new.verification_state,"pending");assert!(store.snapshot(&ws.id).is_err());}
fn policy_for(w:&str,effect:&str,source:Vec<String>)->PolicyProposal {let _=w;PolicyProposal{title:"顧客保護を確認する".into(),domain:"all".into(),effect:effect.into(),condition:serde_json::from_value(json!({"op":"gte","field":"candidate.customer_protection","value":0.8,"unit":null})).unwrap(),source_case_ids:source,effective_from:"2020-01-01T00:00:00Z".into(),effective_until:None}}
#[test]
fn deleting_a_source_removes_dependent_policies(){let(_temp,mut store,ws)=setup();let c=answered(&mut store,&ws.id);let p=store.propose_policy(&ws.id,policy_for(&ws.id,"prefer",vec![c.id.clone()])).unwrap();store.set_policy_state(&ws.id,&p.id,1,"effective").unwrap();store.delete_case(&ws.id,&c.id,c.revision).unwrap();assert!(store.policies(&ws.id).unwrap().is_empty());assert!(store.tombstones().unwrap().iter().any(|t|t["kind"]=="policy"));}
#[test]
fn policy_unknown_is_not_false_and_code_is_not_evaluated(){let p=proposal(&id());let e:Expr=serde_json::from_value(json!({"op":"gte","field":"context.unknown_budget","value":5,"unit":null})).unwrap();assert_eq!(e.evaluate(&p.context,&p.candidates[0]),policy::Truth::Unknown);assert!(serde_json::from_value::<Expr>(json!({"op":"eval","code":"danger"})).is_err());}
#[test]
fn policy_temporal_comparison_respects_offsets(){let mut p=policy_for(&id(),"prefer",vec![]);p.effective_from="2024-01-02T00:00:00+09:00".into();p.effective_until=Some("2024-01-01T16:00:00Z".into());assert!(p.validate().is_ok());}
#[test]
fn malformed_and_traversal_archives_are_rejected(){use std::io::Write;let mut buffer=std::io::Cursor::new(vec![]);{let mut writer=zip::ZipWriter::new(&mut buffer);writer.start_file("../conversations.json",zip::write::SimpleFileOptions::default()).unwrap();writer.write_all(b"[]").unwrap();writer.finish().unwrap();}let bytes=buffer.into_inner();assert!(imports::parse_archive(std::io::Cursor::new(&bytes),bytes.len()as u64).is_err());assert!(imports::parse_json(b"{\"mapping\":{},\"mapping\":{}}").is_err());}
#[test]
fn conversation_roles_are_not_promoted_to_human_decisions(){let(_temp,mut store,ws)=setup();let export=json!([{"id":"fixture","mapping":{"a":{"message":{"author":{"role":"assistant"},"content":{"parts":["I recommend A"]}}},"b":{"message":{"author":{"role":"user"},"content":{"parts":["I selected B"]}}}}}]);let parsed=imports::parse_json(&serde_json::to_vec(&export).unwrap()).unwrap();let r=imports::ingest(&mut store,&ws.id,parsed.clone()).unwrap();assert_eq!(r["new_cases"],1);assert_eq!(store.cases(&ws.id,20,0).unwrap()[0].verification_state,"pending");assert_eq!(imports::ingest(&mut store,&ws.id,parsed).unwrap()["duplicate"],true);}
#[test]
fn diagnostic_export_suppresses_small_cells_and_free_text(){let(_temp,mut store,ws)=setup();for _ in 0..21{answered(&mut store,&ws.id);}let r=diagnostics::preview(&store,&ws.id).unwrap();assert_eq!(r.cells.len(),1);assert_eq!(r.cells[0].confirmed_count_bucket,20);let mut v=serde_json::to_value(r).unwrap();v["raw_note"]=json!("CANARY_PRIVATE");assert!(serde_json::from_value::<diagnostics::DiagnosticReport>(v).is_err());}
#[test]
fn backup_is_authenticated_and_does_not_resurrect_deleted_cases(){let(temp,mut store,ws)=setup();let c=answered(&mut store,&ws.id);let bundle=backup::export(&store).unwrap();let raw=backup::encrypt(&bundle,"a-test-only-long-passphrase").unwrap();assert!(!String::from_utf8_lossy(&raw).contains("検証用"));assert!(backup::decrypt(&raw,"a-different-long-passphrase").is_err());let decoded=backup::decrypt(&raw,"a-test-only-long-passphrase").unwrap();store.delete_case(&ws.id,&c.id,c.revision).unwrap();let(restored,_)=backup::restore_new(temp.path(),&decoded,&store.tombstones().unwrap()).unwrap();assert!(restored.cases(&ws.id,20,0).unwrap().is_empty());assert!(restored.grants(&ws.id).unwrap().is_empty());assert!(restored.active_model(&ws.id).unwrap().is_none());}
#[test]
fn personality_requires_valid_scale_and_is_not_automatic_training_data(){let(_temp,mut store,ws)=setup();let body=json!({"instrument":"self_report","instrument_version":"example","language":"ja","assessed_at":"2024-01-01T00:00:00Z","source_kind":"self_report","scales":[{"key":"openness","score":3,"minimum":1,"maximum":5}]});let a=store.assessment(&ws.id,body.clone()).unwrap();assert!(!store.consent(&ws.id,"personality_external","none").unwrap());assert!(store.snapshot(&ws.id).is_err());let mut bad=body;bad["scales"][0]["score"]=json!(9);assert!(store.assessment(&ws.id,bad).is_err());store.delete_assessment(&ws.id,a["id"].as_str().unwrap()).unwrap();assert!(store.assessments(&ws.id).unwrap().is_empty());}
fn runtime()->(tempfile::TempDir,Arc<Runtime>,String){let temp=tempfile::tempdir().unwrap();let rt=Runtime::new(temp.path().into(),Arc::new(keys::MemoryKeys::default()),jobs::LearnerCommand{executable:"/not-installed/cdna-learner".into(),args:vec![],python_path:None}).unwrap();rt.owner_call("create_vault",json!({})).unwrap();let ws=rt.owner_call("workspaces",json!({})).unwrap()[0]["id"].as_str().unwrap().to_owned();(temp,rt,ws)}
fn grant_for(rt:&Arc<Runtime>,w:&str,scopes:&[&str])->(String,String){let g=rt.owner_call("create_grant",json!({"grant":{"client_name":"unit-test","workspace_id":w,"scopes":scopes,"domains":["product_delivery"],"expires_at":(chrono::Utc::now()+chrono::Duration::days(1)).to_rfc3339(),"include_text":false}})).unwrap();let token=if g["keychain_saved"]==true{keys::load_mcp_token(g["grant"]["id"].as_str().unwrap()).unwrap().to_string()}else{g["token"].as_str().unwrap().to_owned()};(g["grant"]["id"].as_str().unwrap().into(),token)}
#[test]
fn agent_cannot_call_owner_functions_or_forge_scope(){let(_temp,rt,w)=runtime();let(gid,token)=grant_for(&rt,&w,&["profile:read"]);assert!(rt.agent_call(&gid,&token,"confirm",json!({"workspace_id":w})).is_err());assert!(rt.agent_call(&gid,&token,"cdna_get_profile",json!({"workspace_id":id()})).is_err());assert!(rt.agent_call(&gid,"0000000000000000000000000000000000000000000000000000000000000000","cdna_get_profile",json!({"workspace_id":w})).is_err());}
#[test]
fn revoke_and_lock_invalidate_inflight_results(){let(_temp,rt,w)=runtime();let(gid,token)=grant_for(&rt,&w,&["profile:read"]);let ticket=rt.agent_call(&gid,&token,"cdna_get_profile",json!({"workspace_id":w})).unwrap();rt.owner_call("revoke_grant",json!({"workspace_id":w,"grant_id":gid})).unwrap();assert!(rt.validate_ticket(&gid,&token,&ticket).is_err());let(gid,token)=grant_for(&rt,&w,&["profile:read"]);let ticket=rt.agent_call(&gid,&token,"cdna_get_profile",json!({"workspace_id":w})).unwrap();rt.owner_call("lock",json!({})).unwrap();assert_eq!(rt.validate_ticket(&gid,&token,&ticket).unwrap_err().code,"VAULT_LOCKED");assert_eq!(rt.agent_call(&gid,&token,"cdna_get_profile",json!({"workspace_id":w})).unwrap_err().code,"VAULT_LOCKED");}
#[test]
fn new_question_has_no_preselected_model_answer(){let(_temp,rt,w)=runtime();let q=rt.owner_call("next_question",json!({"workspace_id":w,"domain":"product_delivery"})).unwrap();assert!(q["case"]["response"].is_null());assert_eq!(q["case"]["verification_state"],"pending");assert_eq!(q["exposure"]["model_exposure"],false);assert_eq!(q["exposure"]["display_order"].as_array().unwrap().len(),4);}
