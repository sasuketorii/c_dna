//! One application boundary shared by local user interfaces and scoped agent tools.
pub mod policy;
pub mod importer;
pub mod import_workflow;
pub mod assessment;
pub mod questions;
pub mod with_ai;
pub mod bootstrap;
pub mod evolution_bridge;
pub mod evolution_cli;
mod inference_runtime;
pub mod natural_language;
pub mod local_provider;
mod training_snapshot;
use anyhow::{Result, bail, ensure};
use cdna_domain::{ObservationProposal, ObservationResponse, RankRequest};
use cdna_inference::{DecisionScorer, LinearScorer, extract_features};
use cdna_store::{Status, Store};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::{Arc,atomic::{AtomicBool,Ordering}}, collections::HashSet, io::{Read, Write}, path::PathBuf, process::{Command, Stdio}, time::{Duration, Instant}};
use uuid::Uuid;

#[derive(Clone)]
pub enum Authority { Human, Agent { workspace_id: Uuid, scopes: Vec<String> } }
pub struct TrainingTask { pub workspace: Uuid, pub epoch: u64, pub learner: PathBuf, pub input: Value, pub lineage: Vec<Value> }
pub struct Engine { pub store: Store, pub learner: PathBuf, pub demo: bool, evolution_trust: Option<evolution_bridge::EvolutionBridge> }
#[derive(Deserialize)]
#[serde(tag="operation", rename_all="snake_case", deny_unknown_fields)]
enum Action {
    Status {},
    WorkspaceCreate { name: String },
    List { workspace_id: Uuid, #[serde(default)] offset: u32 },
    Propose { proposal: ObservationProposal },
    Confirm { workspace_id: Uuid, id: Uuid, expected_revision: u64, request_id: Uuid },
    Revise { workspace_id: Uuid, id: Uuid, expected_revision: u64, request_id: Uuid, proposal: ObservationProposal },
    Delete { workspace_id: Uuid, id: Uuid, expected_revision: u64 },
    Rank { request: RankRequest },
    #[serde(alias="with_ai_bootstrap")]
    Train { workspace_id: Uuid, #[serde(default)] domain:Option<cdna_domain::Domain> },
    Models { workspace_id: Uuid },
    PreviewModel { workspace_id: Uuid, id: Uuid, request: RankRequest },
    ActivateModel { workspace_id: Uuid, id: Uuid },
    Gaps { workspace_id: Uuid },
    Export { workspace_id: Uuid, #[serde(default)] offset: u32 },
    Policies { workspace_id: Uuid },
    ProposePolicy { workspace_id: Uuid, policy: policy::Policy },
    ApprovePolicy { workspace_id: Uuid, id: Uuid, expected_revision: u64 },
    RevokePolicy { workspace_id: Uuid, id: Uuid, expected_revision: u64 },
    CheckPolicy { request: RankRequest },
    ImportSources { workspace_id:Uuid, provider:importer::Provider, export_json:String, preview:bool,
        #[serde(default)] selected_conversation_ids:Vec<String>, #[serde(default)] cursor:Option<String>, #[serde(default)] page_size:Option<usize> },
    Sources { workspace_id:Uuid, #[serde(default)] offset:u32 },
    SourceGet { workspace_id:Uuid, id:Uuid },
    SourceDelete { workspace_id:Uuid, id:Uuid, expected_revision:u64 },
    Assessments { workspace_id:Uuid, #[serde(default)] offset:u32 },
    AssessmentSave { workspace_id:Uuid, id:Uuid, expected_revision:u64, result:assessment::AssessmentResult },
    AssessmentDelete { workspace_id:Uuid, id:Uuid, expected_revision:u64 },
    SelectQuestions { request:questions::QuestionSelectionRequest, model_id:Option<Uuid> },
    WithAiProfile { workspace_id:Uuid },
    WithAiEnqueue { request:cdna_mcp::EnqueueRequest },
    WithAiList { workspace_id:Uuid },
    WithAiAdopt { workspace_id:Uuid, id:Uuid, epoch:u64 },
    WithAiRespond { workspace_id:Uuid, id:Uuid, question_id:Uuid, request_id:Uuid, response:ObservationResponse, #[serde(default)] rationale_explicit:Option<String>, #[serde(default)] reversal_conditions:Vec<String>, #[serde(default)] model_exposure:bool },
    Lock {},
}
impl Engine {
    pub fn new(store: Store, learner: PathBuf, demo: bool) -> Self {
        Self { store, learner, demo, evolution_trust: None }
    }
    /// Host configuration only; these keys are never read from action JSON.
    pub fn with_evolution_trust(mut self, evaluator: ed25519_dalek::VerifyingKey, human: ed25519_dalek::VerifyingKey) -> Result<Self> {
        self.evolution_trust = Some(evolution_bridge::EvolutionBridge::new(evaluator, human)?);
        Ok(self)
    }

    pub fn execute(&mut self, command: Value, authority: Authority) -> Result<Value> {
        ensure!(serde_json::to_vec(&command)?.len() <= cdna_domain::MAX_REQUEST_BYTES, "INPUT_INVALID: request too large");
        let action: Action = serde_json::from_value(command)?;
        match action {
            Action::Status {} => {
                let workspaces = self.store.workspaces()?;
                let visible: Vec<_> = workspaces.into_iter().filter(|w|matches!(&authority,Authority::Human)||matches!(&authority,Authority::Agent{workspace_id,..} if workspace_id==w)).collect();
                Ok(json!({"schema_version":"1.0","workspaces":visible,"workspace_details":self.store.workspace_details()?.into_iter().filter(|(id,_)|visible.contains(id)).collect::<Vec<_>>(),"demo":self.demo,"locked":false,"cloud_enabled":false,"personal_accuracy":"unverified","capabilities":["observations","confirmation","revisions","encrypted_storage","ranking","learning","export"]}))
            }
            Action::WorkspaceCreate { name } => {
                human(&authority)?;
                ensure!(!name.trim().is_empty()&&name.len()<=256,"INPUT_INVALID: workspace name");
                let id=Uuid::new_v4(); self.store.create_workspace(id)?;self.store.set_workspace_name(id,&name)?;
                // Workspace names are display metadata; the UUID is the isolation boundary.
                Ok(json!({"id":id,"name":name}))
            }
            Action::List { workspace_id, offset } => {
                allow(&authority,workspace_id,"decision:read")?;
                Ok(json!({"records":self.store.list(workspace_id,100,offset)?,"offset":offset,"limit":100}))
            }
            Action::Propose { mut proposal } => {
                allow(&authority,proposal.workspace_id,"decision:propose")?;
                proposal.validate()?;
                if matches!(authority,Authority::Agent{..}) && !matches!(proposal.source.source_kind,cdna_domain::SourceKind::AiGenerated) {proposal.source.source_kind=cdna_domain::SourceKind::AgentProposal;}
                let id=proposal.request_id; // durable stable request identity permits retries
                Ok(serde_json::to_value(self.store.propose(proposal.workspace_id,id,proposal.request_id,serde_json::to_value(&proposal)?)?)?)
            }
            Action::Confirm {workspace_id,id,expected_revision,request_id} => {
                human(&authority)?;
                let record=self.store.get(workspace_id,id)?;
                let proposal:ObservationProposal=serde_json::from_value(record.payload)?;
                proposal.validate()?;
                ensure!(!matches!(proposal.source.source_kind,cdna_domain::SourceKind::AiGenerated),"INPUT_INVALID: AI-generated answers cannot become human labels");
                Ok(serde_json::to_value(self.store.confirm(workspace_id,id,expected_revision,request_id)?)?)
            }
            Action::Revise {workspace_id,id,expected_revision,request_id,mut proposal} => {
                human(&authority)?; proposal.validate()?;
                ensure!(workspace_id==proposal.workspace_id,"SCOPE_DENIED");
                let previous:ObservationProposal=serde_json::from_value(self.store.get(workspace_id,id)?.payload)?;
                ensure!(previous.family_id==proposal.family_id,"INPUT_INVALID: revision must retain family lineage");
                proposal.model_exposure |= previous.model_exposure;
                proposal.source=previous.source; // Ordinary corrections retain immutable provenance.
                Ok(serde_json::to_value(self.store.revise(workspace_id,id,expected_revision,request_id,serde_json::to_value(proposal)?)?)?)
            }
            Action::Delete {workspace_id,id,expected_revision} => {
                human(&authority)?; self.store.delete(workspace_id,id,expected_revision)?;
                Ok(json!({"deleted":id,"epoch":self.store.epoch()?}))
            }
            Action::Rank {request} => {allow(&authority,request.workspace_id,"judgment:infer")?;request.validate()?;self.rank(request)}
            Action::Train {workspace_id,domain} => {human(&authority)?;let task=self.prepare_train_domain(workspace_id,domain)?;let output=run_training(&task,Arc::new(AtomicBool::new(false)))?;self.finish_train(task,output)}
            Action::Models {workspace_id} => {human(&authority)?;Ok(json!({"models":self.store.list_models(workspace_id,100)?.into_iter().map(|(_,value)|value).collect::<Vec<_>>()}))}
            Action::PreviewModel {workspace_id,id,request} => {
                human(&authority)?;request.validate()?;
                ensure!(workspace_id==request.workspace_id,"SCOPE_DENIED");
                ensure!(!request.allow_cloud,"CONSENT_REQUIRED");
                let artifact=self.store.model(workspace_id,id)?;
                ensure!(artifact["model"]["training_domain"].is_null()||artifact["model"]["training_domain"]==serde_json::to_value(request.domain)?,"SCOPE_DENIED: model domain mismatch");
                let weights:Vec<f64>=serde_json::from_value(artifact["model"]["weights"].clone())?;
                let mut scorer=LinearScorer::new(artifact["model"]["feature_version"].as_str().unwrap_or(""),weights).map_err(anyhow::Error::msg)?;
                let context=serde_json::to_value(&request.context)?;
                let features=request.candidates.iter().map(|c|extract_features(&context,&serde_json::to_value(c).unwrap())).collect::<std::result::Result<Vec<_>,_>>().map_err(anyhow::Error::msg)?;
                let scores=scorer.score(&features).map_err(anyhow::Error::msg)?;
                let mut ranking:Vec<_>=request.candidates.iter().zip(scores.scores).map(|(c,s)|json!({"candidate_id":c.id,"raw_score":s,"probability":null,"policy_status":"unknown"})).collect();
                ranking.sort_by(|a,b|b["raw_score"].as_f64().unwrap().total_cmp(&a["raw_score"].as_f64().unwrap()));
                Ok(json!({"model_id":id,"mode":"experimental_preview","provisional":true,"ranking":ranking,"probability":null,"engine":scores.engine,"abstained":true,"abstention_reasons":["model_not_independently_validated"],"execution_authorization":"none","warning":"学習変化を確認する実験表示です。本人再現の精度は未検証です。"}))
            }
            Action::ActivateModel {workspace_id,id} => {
                human(&authority)?;
                let bridge=self.evolution_trust.as_ref().ok_or_else(||anyhow::anyhow!("MODEL_NOT_READY: host-pinned evolution trust required"))?;
                let epoch=self.store.epoch()?;
                let snapshot=bridge.load(&self.store,workspace_id,epoch)?;
                let active=bridge.activate(&mut self.store,workspace_id,epoch,snapshot.revision,id)?;
                Ok(serde_json::to_value(active)?)
            }
            Action::Gaps {workspace_id} => {allow(&authority,workspace_id,"learning:read")?;self.gaps(workspace_id)}
            Action::Export {workspace_id,offset} => {human(&authority)?;Ok(json!({"format":"cdna-jsonl-1","sensitive":true,"jsonl":self.store.export_jsonl(workspace_id,100,offset)?,"offset":offset,"limit":100}))}
            Action::Policies{workspace_id}=>{allow(&authority,workspace_id,"policy:read")?;Ok(json!({"policies":self.store.list_documents(cdna_store::DocumentKind::Policy,workspace_id,100,0)?}))},
            Action::ProposePolicy{workspace_id,policy}=>{human(&authority)?;policy.validate()?;let id=Uuid::new_v4();Ok(serde_json::to_value(self.store.put_document(cdna_store::DocumentKind::Policy,workspace_id,id,0,json!({"policy":policy,"status":"draft"}))?)?)},
            Action::ApprovePolicy{workspace_id,id,expected_revision}=>{human(&authority)?;let doc=self.store.get_document(cdna_store::DocumentKind::Policy,workspace_id,id)?;let mut value=doc.payload;ensure!(value["status"]=="draft","INPUT_INVALID: policy is not draft");let p:policy::Policy=serde_json::from_value(value["policy"].clone())?;p.validate()?;value["status"]=json!("approved");Ok(serde_json::to_value(self.store.put_document(cdna_store::DocumentKind::Policy,workspace_id,id,expected_revision,value)?)?)},
            Action::RevokePolicy{workspace_id,id,expected_revision}=>{human(&authority)?;self.store.delete_document(cdna_store::DocumentKind::Policy,workspace_id,id,expected_revision)?;Ok(json!({"revoked":id}))},
            Action::CheckPolicy{request}=>{allow(&authority,request.workspace_id,"policy:read")?;request.validate()?;self.check_policy(&request)},
            Action::ImportSources{workspace_id,provider,export_json,preview,selected_conversation_ids,cursor,page_size}=>{
                human(&authority)?;
                Ok(serde_json::to_value(import_workflow::import_page(&mut self.store,workspace_id,provider,export_json.as_bytes(),&selected_conversation_ids,cursor.as_deref(),page_size.unwrap_or(1000),preview)?)?)
            },
            Action::Sources{workspace_id,offset}=>{human(&authority)?;Ok(json!({"sources":self.store.list_documents(cdna_store::DocumentKind::Source,workspace_id,100,offset)?,"offset":offset,"limit":100}))},
            Action::SourceGet{workspace_id,id}=>{human(&authority)?;Ok(serde_json::to_value(self.store.get_document(cdna_store::DocumentKind::Source,workspace_id,id)?)?)},
            Action::SourceDelete{workspace_id,id,expected_revision}=>{human(&authority)?;let observations=self.store.delete_source_cascade(workspace_id,id,expected_revision)?;Ok(json!({"deleted":id,"deleted_observations":observations}))},
            Action::Assessments{workspace_id,offset}=>{human(&authority)?;Ok(json!({"assessments":self.store.list_documents(cdna_store::DocumentKind::Assessment,workspace_id,100,offset)?,"offset":offset,"limit":100}))},
            Action::AssessmentSave{workspace_id,id,expected_revision,result}=>{human(&authority)?;result.validate()?;Ok(serde_json::to_value(self.store.put_document(cdna_store::DocumentKind::Assessment,workspace_id,id,expected_revision,serde_json::to_value(result)?)?)?)},
            Action::AssessmentDelete{workspace_id,id,expected_revision}=>{human(&authority)?;self.store.delete_document(cdna_store::DocumentKind::Assessment,workspace_id,id,expected_revision)?;Ok(json!({"deleted":id}))},
            Action::SelectQuestions{request,model_id}=>{
                human(&authority)?;
                let records=self.store.list(request.workspace_id,1000,0)?;
                ensure!(records.len()<1000,"INPUT_INVALID: question history limit reached");
                let model=if let Some(id)=model_id{let artifact=self.store.model(request.workspace_id,id)?;Some(LinearScorer::new(artifact["model"]["feature_version"].as_str().unwrap_or(""),serde_json::from_value(artifact["model"]["weights"].clone())?).map_err(anyhow::Error::msg)?)}else{None};
                Ok(serde_json::to_value(questions::select_questions(&request,&records,model.as_ref())?)?)
            },
            Action::WithAiProfile{workspace_id}=>self.with_ai_profile(workspace_id,&authority),
            Action::WithAiEnqueue{request}=>self.with_ai_enqueue(request,&authority),
            Action::WithAiList{workspace_id}=>self.with_ai_list(workspace_id,&authority),
            Action::WithAiAdopt{workspace_id,id,epoch}=>self.with_ai_adopt(workspace_id,id,epoch,&authority),
            Action::WithAiRespond{workspace_id,id,question_id,request_id,response,rationale_explicit,reversal_conditions,model_exposure}=>self.with_ai_respond(workspace_id,id,question_id,request_id,with_ai::HumanAnswer{response,rationale_explicit,reversal_conditions,model_exposure},&authority),
            Action::Lock {} => {human(&authority)?;self.store.lock();Ok(json!({"locked":true}))}
        }
    }
    fn check_policy(&self,request:&RankRequest)->Result<Value> {
        let docs=self.store.list_documents(cdna_store::DocumentKind::Policy,request.workspace_id,100,0)?;
        ensure!(docs.len()<100,"INPUT_INVALID: policy evaluation limit reached");
        let policies:Vec<_>=docs.into_iter().map(|d|(d.id,d.payload)).collect();
        policy::check(&policies,&request.context,&request.candidates)
    }
    fn rank(&self, request: RankRequest) -> Result<Value> {
        ensure!(!request.allow_cloud,"CONSENT_REQUIRED: cloud processing is not configured");
        let input_epoch=self.store.epoch()?;
        let signature=cdna_store::matching_signature(&serde_json::to_value(&request)?)?;
        let matched=self.store.matching_records(request.workspace_id,&signature,1000)?;
        let truncated=matched.truncated;
        let records=matched.records;
        let policy_result=self.check_policy(&request)?;
        let mut evidence=Vec::new();
        let mut all_winners=HashSet::new();
        let mut ambiguous=false;
        let candidate_signature=|v:&[cdna_domain::Candidate]|->Result<Vec<String>> {let mut s=v.iter().map(|c|serde_json::to_string(&(c.text.clone(),&c.attributes))).collect::<std::result::Result<Vec<_>,_>>()?;s.sort();Ok(s)};
        let request_signature=candidate_signature(&request.candidates)?;
        for record in records.iter().filter(|r|r.status==Status::Confirmed) {
            let p:ObservationProposal=serde_json::from_value(record.payload.clone())?;
            if p.domain==request.domain && p.context.summary==request.context.summary && p.context.facts==request.context.facts && p.context.unknown_fields==request.context.unknown_fields && candidate_signature(&p.candidates)?==request_signature
                && let ObservationResponse::ChooseOne{candidate_id}=&p.response
                && let Some(chosen)=p.candidates.iter().find(|c|&c.id==candidate_id) {
                        let matches:Vec<_>=request.candidates.iter().filter(|c|c.text==chosen.text&&c.attributes==chosen.attributes).collect();
                        if matches.len()!=1 {ambiguous=true;}
                        if let Some(current)=matches.first() {
                            all_winners.insert(current.id.clone());
                            if evidence.len()<20 { evidence.push(json!({"record_id":record.id,"revision":record.revision,"candidate_id":current.id,"kind":"confirmed_observation","source":p.source,"case_kind":p.case_kind,"model_exposure":p.model_exposure})); }
                        }
            }
        }
        let winners:HashSet<_>=all_winners.iter().map(String::as_str).collect();
        let mut reasons=Vec::new();
        if truncated {reasons.push("memory_search_limit_reached");}
        if ambiguous {reasons.push("ambiguous_candidates");}
        if !request.context.unknown_fields.is_empty()||request.context.facts.iter().any(|f|f.evidence_status!=cdna_domain::EvidenceStatus::Explicit) {reasons.push("missing_information");}
        if winners.len()>1 {reasons.push("conflicting_evidence");}
        if evidence.is_empty() {reasons.push("no_matching_confirmed_memory");}
        if matches!(request.mode,cdna_domain::Mode::Advisor) {reasons.push("advisor_requires_separate_evidence");}
        if all_winners.iter().any(|id|policy_result["candidates"].as_array().is_some_and(|rows|rows.iter().any(|r|r["candidate_id"]==*id&&(r["status"]=="violated"||r["status"]=="unknown")))){reasons.push("policy_violation_or_unknown");}
        if reasons.len()==1 && reasons[0]=="no_matching_confirmed_memory" {
            if let Some(bridge)=&self.evolution_trust {
                match inference_runtime::rank(&self.store,bridge,&request,&policy_result,input_epoch) {
                    Ok(result)=>return Ok(result),
                    Err(reason)=>reasons.push(reason),
                }
            }
        }
        ensure!(self.store.epoch()?==input_epoch,"MODEL_INVALIDATED: input changed while ranking");
        let selected=if reasons.is_empty(){winners.iter().next().copied()}else{None};
        let ranking:Vec<_>=request.candidates.iter().map(|c|json!({"candidate_id":c.id,"raw_score":null,"probability":null,"memory_match":selected==Some(c.id.as_str()),"policy_status":policy_result["candidates"].as_array().and_then(|a|a.iter().find(|p|p["candidate_id"]==c.id)).map(|p|p["status"].clone()).unwrap_or(json!("unknown"))})).collect();
        Ok(json!({"mode":request.mode,"ranking":ranking,"selected_candidate_id":selected,"evidence":if request.include_evidence{evidence}else{Vec::new()},"abstained":!reasons.is_empty(),"abstention_reasons":reasons,"execution_authorization":"none","model_status":"memory_only","probability":null}))
    }
    pub fn prepare_train(&self, workspace:Uuid)->Result<TrainingTask> {
        training_snapshot::prepare(&self.store,&self.learner,workspace)
    }
    pub fn prepare_train_domain(&self,workspace:Uuid,domain:Option<cdna_domain::Domain>)->Result<TrainingTask> {
        training_snapshot::prepare_for_domain(&self.store,&self.learner,workspace,domain)
    }
    pub fn finish_train(&mut self,task:TrainingTask,output:Value)->Result<Value> {
        ensure!(output["ok"]==true,"MODEL_NOT_READY: learning worker rejected the snapshot");
        let id=Uuid::new_v4();
        let artifact=json!({"id":id,"workspace_id":task.workspace,"epoch":task.epoch,"state":"provisional","created_at":chrono::Utc::now(),"lineage":task.lineage,"model":output["model"],"evaluation":output["evaluation"],"counts":output["counts"],"splits":output["splits"],"split_family_ids":output["split_family_ids"],"acceptability_memory":output["acceptability_memory"],"calibration_status":output["calibration_status"],"validation_status":output["validation_status"]});
        self.store.save_model(task.workspace,id,task.epoch,artifact.clone())?;
        Ok(artifact)
    }
    fn gaps(&self,workspace:Uuid)->Result<Value> {
        let records=self.store.list(workspace,1000,0)?;
        let domains=["resource_allocation","product_delivery","customer_commercial","organization_delegation","growth_strategy","risk_reputation"];
        let gaps:Vec<_>=domains.iter().map(|d|{let count=records.iter().filter(|r|r.status==Status::Confirmed&&r.payload["domain"]==*d).count();json!({"domain":d,"confirmed_records":count,"status":if count==0{"unlearned"}else{"recorded_unverified"},"accuracy":null})}).collect();
        Ok(json!({"domains":gaps,"counts_truncated":records.len()==1000}))
    }
}
fn human(a:&Authority)->Result<()> {ensure!(matches!(a,Authority::Human),"SCOPE_DENIED: trusted owner operation");Ok(())}
fn allow(a:&Authority,w:Uuid,scope:&str)->Result<()> {match a {Authority::Human=>Ok(()),Authority::Agent{workspace_id,scopes}=>{ensure!(*workspace_id==w&&scopes.iter().any(|s|s==scope),"SCOPE_DENIED");Ok(())}}}

/// Bounded subprocess: fixed executable, no shell, bounded pipes, explicit reap on timeout.
pub fn run_training(task:&TrainingTask,cancel:Arc<AtomicBool>)->Result<Value> {
    let project=&task.learner; let input=&task.input;
    let bytes=serde_json::to_vec(input)?;ensure!(bytes.len()<=8*1024*1024,"INPUT_INVALID: snapshot too large");
    let bundled=project.join("python/bin/python3");
    let python=if bundled.is_file(){bundled}else{project.join(".venv/bin/python")};
    ensure!(python.is_file(),"PROVIDER_UNAVAILABLE: run uv sync --project learner first");
    let mut child=Command::new(python).args(["-I","-B","-m","cdna_learner"]).env("OMP_NUM_THREADS","1").env("OPENBLAS_NUM_THREADS","1").env("MKL_NUM_THREADS","1").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let mut stdin=child.stdin.take().unwrap();
    let writer=std::thread::spawn(move||stdin.write_all(&bytes));
    let stdout=child.stdout.take().unwrap();
    let reader=std::thread::spawn(move||{let mut data=Vec::new();stdout.take(8*1024*1024+1).read_to_end(&mut data).map(|_|data)});
    let started=Instant::now();
    let status=loop {if let Some(s)=child.try_wait()?{break s;}
        if cancel.load(Ordering::Acquire)||started.elapsed()>Duration::from_secs(30){let _=child.kill();let _=child.wait();let _=writer.join();let _=reader.join();bail!("CANCELLED: training exceeded 30 seconds");}std::thread::sleep(Duration::from_millis(10));};
    writer.join().map_err(|_|anyhow::anyhow!("worker input failed"))??;
    let output=reader.join().map_err(|_|anyhow::anyhow!("worker output failed"))??;
    ensure!(status.success()&&output.len()<=8*1024*1024,"INTERNAL_ERROR: training worker failed");
    Ok(serde_json::from_slice(&output)?)
}

pub fn error_json(error:&anyhow::Error)->Value {
    let message=error.to_string();
    let code=if matches!(error.downcast_ref::<cdna_store::StoreError>(),Some(cdna_store::StoreError::Locked)){"VAULT_LOCKED"}else if matches!(error.downcast_ref::<cdna_store::StoreError>(),Some(cdna_store::StoreError::RevisionConflict)){"REVISION_CONFLICT"}else if message.contains("CANCELLED"){"CANCELLED"}else if message.contains("SCOPE_DENIED"){"SCOPE_DENIED"}else if message.contains("MODEL_NOT_READY"){"MODEL_NOT_READY"}else if message.contains("CONSENT_REQUIRED"){"CONSENT_REQUIRED"}else if message.contains("Locked"){"VAULT_LOCKED"}else if message.contains("RevisionConflict"){"REVISION_CONFLICT"}else{"INPUT_INVALID"};
    // Parser messages can contain user input; never return raw errors across transports.
    json!({"ok":false,"error":{"code":code,"message":match code {"SCOPE_DENIED"=>"この操作は許可されていません。","MODEL_NOT_READY"=>"確認済み回答または独立した評価が不足しています。","VAULT_LOCKED"=>"保管庫はロックされています。","REVISION_CONFLICT"=>"別の操作で更新されています。再読み込みしてください。",_=>"操作を完了できませんでした。入力と状態を確認してください。"}}})
}
