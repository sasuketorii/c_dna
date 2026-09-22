//! Synthetic signatures and labels test gates only; never production evidence.
use cdna_app::{Authority,Engine,evolution_cli};
use cdna_evolution::{Candidate,Dataset,DatasetCase,ModelConfig,Outcome,PairedCase,Split,EvaluationReceipt,HumanApproval,Signed,signing_bytes,content_hash,hash};
use ed25519_dalek::{Signer,SigningKey};
use serde::Serialize;
use serde_json::{Value,json};
use uuid::Uuid;
use cdna_store::Store;
use zeroize::Zeroizing;
fn signed<T:Serialize>(payload:T,key:&SigningKey)->Signed<T>{let signature=key.sign(&signing_bytes(&payload).unwrap()).to_bytes().to_vec();Signed{payload,signature}}
fn keys()->(SigningKey,SigningKey){(SigningKey::from_bytes(&[31;32]),SigningKey::from_bytes(&[32;32]))}
fn engine()->(tempfile::TempDir,Engine,Uuid){
 let dir=tempfile::tempdir().unwrap();let store=Store::create(dir.path().join("test.db"),&Zeroizing::new("a".repeat(64))).unwrap();
 let (e,h)=keys();let engine=Engine::new(store,"unused".into(),false).with_evolution_trust(e.verifying_key(),h.verifying_key()).unwrap();
 let w=Uuid::new_v4();engine.store.create_workspace(w).unwrap();(dir,engine,w)
}
fn op(e:&mut Engine,v:Value)->Value{let (eval,human)=keys();evolution_cli::run(&mut e.store,v,eval.verifying_key().to_bytes(),human.verifying_key().to_bytes()).unwrap()}
fn promote(e:&mut Engine,workspace:Uuid,weight:f64,baseline:Option<String>,revision:u64)->(Uuid,String){
 promote_custom(e,workspace,weight,baseline,revision, |_| {})
}
fn promote_custom(e:&mut Engine,workspace:Uuid,weight:f64,baseline:Option<String>,revision:u64,adjust:impl FnOnce(&mut ModelConfig))->(Uuid,String){
 let (evaluator,human)=keys();let epoch=e.store.epoch().unwrap();
            let mut cases = Vec::new();
            for index in 0..201 {
                cases.push(DatasetCase {
                    id: Uuid::new_v4(),
                    family_id: Uuid::new_v4(),
                    split: if index == 0 {
                        Split::Train
                    } else {
                        Split::Test
                    },
                    timestamp: if index == 0 { 1 } else { 2 },
                    input_hash: hash(format!("synthetic-test-input-{index}").as_bytes()),
                    answer: "synthetic-a".into(),
                    independent_human_answer: true,
                    model_exposure: false,
                    domain: "product_delivery".into(),
                });
            }
            let dataset = Dataset {
                id: Uuid::new_v4(),
                workspace_id: workspace,
                input_revision: epoch,
                deletion_epoch: epoch,
                cases,
            };
            let mut config = ModelConfig {
                inference: Some(cdna_evolution::InferenceSupport {
                    schema_version:"1.0".into(), slope:2.0, domains:vec!["product_delivery".into()],
                    valid_from_unix:0, valid_until_unix:4_000_000_000,
                    min_margin:0.05,max_margin:2.0,missing_patterns:vec![0],
                    feature_min:vec![-1.0;17],feature_max:vec![1.0;17],
                }),
                feature_version: "1.0".into(),
                weights: (0..17).map(|i| if i==2 {weight} else {0.0}).collect(),
            };
            adjust(&mut config);
            let artifact = signing_bytes(&config).unwrap();
            let candidate = Candidate {
                id: Uuid::new_v4(),
                workspace_id: workspace,
                input_revision: epoch,
                deletion_epoch: epoch,
                artifact_hash: hash(&artifact),
                training_dataset_id: dataset.id,
                training_dataset_hash: content_hash(&dataset).unwrap(),
                config,
            };
            let outcome = Outcome {
                choice: Some("synthetic-a".into()),
                policy_violations: 0,
                latency_us: 10,
                peak_memory_bytes: 1024,
            };
            let receipt = signed(
                EvaluationReceipt {
                    id: Uuid::new_v4(),
                    workspace_id: workspace,
                    input_revision: epoch,
                    deletion_epoch: epoch,
                    candidate_id: candidate.id,
                    artifact_hash: candidate.artifact_hash.clone(),
                    baseline_artifact_hash: baseline.unwrap_or_else(|| hash(b"synthetic-baseline")),
                    dataset_id: dataset.id,
                    dataset_hash: content_hash(&dataset).unwrap(),
                    evaluator_version: "synthetic-persistence-test-only".into(),
                    test_previously_exposed: false,
                    rows: dataset
                        .cases
                        .iter()
                        .filter(|case| case.split == Split::Test)
                        .map(|case| PairedCase {
                            case_id: case.id,
                            input_hash: case.input_hash.clone(),
                            candidate: outcome.clone(),
                            baseline: outcome.clone(),
                        })
                        .collect(),
                    candidate_learning_seconds: 1,
                    baseline_learning_seconds: 1,
                },
                &evaluator,
            );
            let approval = signed(
                HumanApproval {
                    workspace_id: workspace,
                    input_revision: epoch,
                    deletion_epoch: epoch,
                    candidate_id: candidate.id,
                    artifact_hash: candidate.artifact_hash.clone(),
                    receipt_hash: content_hash(&receipt).unwrap(),
                    human_id: Uuid::new_v4(),
                },
                &human,
            );
 op(e,json!({"operation":"register","workspace_id":workspace,"expected_revision":revision,"candidate":candidate,"artifact":artifact}));
 assert!(e.execute(json!({"operation":"activate_model","workspace_id":workspace,"id":candidate.id}),Authority::Human).is_err());
 op(e,json!({"operation":"evaluate","workspace_id":workspace,"expected_revision":revision+1,"receipt":receipt,"dataset":dataset}));
 assert!(e.execute(json!({"operation":"activate_model","workspace_id":workspace,"id":candidate.id}),Authority::Human).is_err());
 op(e,json!({"operation":"approve","workspace_id":workspace,"expected_revision":revision+2,"approval":approval}));
 e.execute(json!({"operation":"activate_model","workspace_id":workspace,"id":candidate.id}),Authority::Human).unwrap();
 (candidate.id,candidate.artifact_hash)
}
fn request(w:Uuid)->Value{json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":w,"mode":"imitate","domain":"product_delivery","context":{"as_of":"2026-09-22T00:00:00Z","summary":"Previously unseen synthetic context","facts":[{"key":"deadline_pressure","value":0.5,"unit":"ratio","evidence_status":"explicit"},{"key":"asset_importance","value":0.5,"unit":"ratio","evidence_status":"explicit"},{"key":"loss_tolerance","value":0.5,"unit":"ratio","evidence_status":"explicit"},{"key":"customer_impact","value":0.5,"unit":"ratio","evidence_status":"explicit"},{"key":"budget_pressure","value":0.5,"unit":"ratio","evidence_status":"explicit"}],"unknown_fields":[]},"candidates":[{"id":"a","text":"Option A","attributes":{"cost":0.1,"effort":0.2,"speed":0.8,"reuse":0.3,"customer_impact":0.4,"irreversibility":0.2}},{"id":"b","text":"Option B","attributes":{"cost":0.1,"effort":0.2,"speed":0.2,"reuse":0.3,"customer_impact":0.4,"irreversibility":0.2}}],"include_evidence":true,"allow_cloud":false})}
fn rank(e:&mut Engine,r:Value)->Value{e.execute(json!({"operation":"rank","request":r}),Authority::Human).unwrap()}
#[test]
fn public_signed_activation_generalizes_and_rollback_restores(){
 let (_dir,mut e,w)=engine();let (first,h)=promote(&mut e,w,1.0,None,0);
 let result=rank(&mut e,request(w));assert_eq!(result["selected_candidate_id"],"a");assert_eq!(result["model_status"],"approved_learned");assert_eq!(result["ranking"][0]["raw_score"],0.8);assert!((result["pairwise_probability"]["probability_left"].as_f64().unwrap()-1.0/(1.0+(-1.2f64).exp())).abs()<1e-12);
 let (second,_)=promote(&mut e,w,-1.0,Some(h),4);assert_eq!(rank(&mut e,request(w))["selected_candidate_id"],"b");
 op(&mut e,json!({"operation":"rollback","workspace_id":w,"expected_revision":8,"candidate_id":first}));
 assert_eq!(rank(&mut e,request(w))["model_id"],json!(first));assert_ne!(first,second);
}
#[test]
fn unsupported_inputs_and_rotated_trust_abstain(){
 let (_dir,mut e,w)=engine();promote(&mut e,w,1.0,None,0);
 for change in 0..8 {let mut r=request(w);match change {
 0=>r["domain"]=json!("risk_reputation"),
 1=>r["context"]["unknown_fields"]=json!(["budget"]),
 2=>{r["candidates"][0]["attributes"].as_object_mut().unwrap().remove("cost");},
 3=>r["candidates"][0]["attributes"]["speed"]=json!(0.2),
 4=>r["context"]["facts"][0]["key"]=json!("unsupported"),
 5=>r["context"]["as_of"]=json!("2200-01-01T00:00:00Z"),
 6=>r["candidates"].as_array_mut().unwrap().push(json!({"id":"c","text":"Third","attributes":{}})),
 _=>r["context"]["facts"][0]["evidence_status"]=json!("inferred")};
 let out=rank(&mut e,r);assert_eq!(out["abstained"],true,"{change}: {out}");assert!(out["probability"].is_null());}
 e=e.with_evolution_trust(SigningKey::from_bytes(&[44;32]).verifying_key(),keys().1.verifying_key()).unwrap();assert_eq!(rank(&mut e,request(w))["abstained"],true);
}
#[test]
fn permissions_and_epoch_invalidation_remain_closed(){
 let (_dir,mut e,w)=engine();let (id,_)=promote(&mut e,w,1.0,None,0);
 assert!(e.execute(json!({"operation":"activate_model","workspace_id":w,"id":id}),Authority::Agent{workspace_id:w,scopes:vec!["decision:read".into()]}).is_err());
 assert!(e.execute(json!({"operation":"rank","request":request(w)}),Authority::Agent{workspace_id:Uuid::new_v4(),scopes:vec!["decision:read".into()]}).is_err());
 e.store.propose(w,Uuid::new_v4(),Uuid::new_v4(),json!({"synthetic":true})).unwrap();assert_eq!(rank(&mut e,request(w))["abstained"],true);
}

fn remember(e:&mut Engine,w:Uuid,r:&Value,chosen:&str){
 let proposal=json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":w,"family_id":Uuid::new_v4(),"case_kind":"hypothetical","domain":r["domain"],"context":r["context"],"candidates":r["candidates"],"response":{"response_type":"choose_one","candidate_id":chosen},"source":{"artifact_id":Uuid::new_v4(),"source_kind":"human_app","occurred_at":null,"observed_at":"2026-09-22T00:00:00Z"},"model_exposure":false});
 let record=e.execute(json!({"operation":"propose","proposal":proposal}),Authority::Human).unwrap();
 e.execute(json!({"operation":"confirm","workspace_id":w,"id":record["id"],"expected_revision":record["revision"],"request_id":Uuid::new_v4()}),Authority::Human).unwrap();
}
#[test]
fn memory_precedence_and_conflict_prevent_model_override(){
 for conflict in [false,true] {
 let (_dir,mut e,w)=engine();let r=request(w);remember(&mut e,w,&r,"b");if conflict {remember(&mut e,w,&r,"a");}
 promote(&mut e,w,1.0,None,0);let result=rank(&mut e,r);
 assert_eq!(result["model_status"],"memory_only");assert_eq!(result["abstained"],conflict);
 if conflict{assert!(result["abstention_reasons"].as_array().unwrap().contains(&json!("conflicting_evidence")));}else{assert_eq!(result["selected_candidate_id"],"b");}
 }
}
#[test]
fn policy_blocks_winner_without_reranking(){
 let (_dir,mut e,w)=engine();let p=json!({"statement":"Synthetic speed cap","when":{"op":"compare","field":"deadline_pressure","comparison":"gt","value":0.0,"unit":"ratio"},"requires":{"op":"compare","field":"speed","comparison":"lte","value":0.5,"unit":"ratio"},"effective_from":"2020-01-01T00:00:00Z","expires_at":null});
 let doc=e.execute(json!({"operation":"propose_policy","workspace_id":w,"policy":p}),Authority::Human).unwrap();e.execute(json!({"operation":"approve_policy","workspace_id":w,"id":doc["id"],"expected_revision":doc["revision"]}),Authority::Human).unwrap();
 promote(&mut e,w,1.0,None,0);let result=rank(&mut e,request(w));assert_eq!(result["abstained"],true);assert!(result["selected_candidate_id"].is_null());assert!(result["abstention_reasons"].as_array().unwrap().contains(&json!("policy_violation_or_unknown")));
}
#[test]
fn signed_support_absence_bounds_and_margin_are_enforced(){
 for case in 0..4 {
 let (_dir,mut e,w)=engine();promote_custom(&mut e,w,1.0,None,0,|config|match case {
 0=>config.inference=None,
 1=>config.inference.as_mut().unwrap().feature_max[2]=0.5,
 2=>config.inference.as_mut().unwrap().max_margin=0.1,
 _=>config.inference.as_mut().unwrap().valid_until_unix=1,
 });assert_eq!(rank(&mut e,request(w))["abstained"],true,"{case}");
 }
}
#[test]
fn detached_weights_and_stale_signed_document_cannot_score(){
 let (_dir,mut e,w)=engine();promote(&mut e,w,1.0,None,0);
 let original=e.store.get_document(cdna_store::DocumentKind::Improvement,w,cdna_app::evolution_bridge::EVOLUTION_DOCUMENT_ID).unwrap();
 let mut payload=original.payload.clone();let id=payload["active"].as_str().unwrap().to_owned();payload["entries"][&id]["candidate"]["config"]["weights"][2]=json!(-1.0);
 e.store.put_evolution_document(w,cdna_app::evolution_bridge::EVOLUTION_DOCUMENT_ID,original.revision,e.store.epoch().unwrap(),payload).unwrap();assert_eq!(rank(&mut e,request(w))["abstained"],true);
 e.store.propose(w,Uuid::new_v4(),Uuid::new_v4(),json!({"synthetic":true})).unwrap();
 e.store.put_evolution_document(w,cdna_app::evolution_bridge::EVOLUTION_DOCUMENT_ID,0,e.store.epoch().unwrap(),original.payload).unwrap();assert_eq!(rank(&mut e,request(w))["abstained"],true);
}

#[test]
fn learner_conversion_preserves_config_and_never_promotes(){
 let (_dir,mut e,w)=engine();let epoch=e.store.epoch().unwrap();let id=Uuid::new_v4();
 let model=json!({"state":"provisional","model":{"feature_version":"1.0","weights":vec![0.2;17],"provisional":true,"calibration":{"method":"sigmoid_pairwise","slope":2.0},"calibration_scope":["product_delivery"],"abstention_margin":0.05,"supported_candidate_missing_masks":[0]}});
 e.store.save_model(w,id,epoch,model).unwrap();
 let dataset=Dataset{id:Uuid::new_v4(),workspace_id:w,input_revision:epoch,deletion_epoch:epoch,cases:(0..2).map(|i|DatasetCase{id:Uuid::new_v4(),family_id:Uuid::new_v4(),split:if i==0{Split::Train}else{Split::Test},timestamp:i+1,input_hash:hash(b"synthetic"),answer:"a".into(),independent_human_answer:true,model_exposure:false,domain:"product_delivery".into()}).collect()};
 let scope=json!({"schema_version":"1.0","slope":2.0,"domains":["product_delivery"],"valid_from_unix":0,"valid_until_unix":4_000_000_000i64,"min_margin":0.05,"max_margin":2.0,"missing_patterns":[0],"feature_min":vec![-1.0;17],"feature_max":vec![1.0;17]});
 let command=json!({"operation":"prepare_candidate","workspace_id":w,"model_id":id,"dataset":dataset,"inference":scope});
 let result=op(&mut e,command.clone());assert_eq!(result["promotion_eligible"],false);assert_eq!(result["state"],"unsigned_candidate");assert_eq!(rank(&mut e,request(w))["abstained"],true);
 let (eval,human)=keys();for field in ["slope","min_margin","missing_patterns"]{let mut bad=command.clone();bad["inference"][field]=if field=="missing_patterns"{json!([1])}else{json!(0.1)};assert!(evolution_cli::run(&mut e.store,bad,eval.verifying_key().to_bytes(),human.verifying_key().to_bytes()).is_err());}
 assert!(e.execute(json!({"operation":"activate_model","workspace_id":w,"id":result["candidate"]["id"]}),Authority::Human).is_err());
}
