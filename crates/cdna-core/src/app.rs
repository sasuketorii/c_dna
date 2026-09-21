//! Trusted application boundary. UI and MCP deliberately expose different APIs.
use crate::{auth::*,backup,diagnostics,domain::*,features,imports,jobs::{self,LearnerCommand},keys::KeyProvider,policy,questions,store::{Store,random_key}};
use rusqlite::{params,OptionalExtension};
use serde::Deserialize;
use serde_json::{json,Value};
use std::{collections::{HashMap,BTreeSet},fs,path::{Path,PathBuf},sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}},time::Instant};
use zeroize::Zeroizing;

struct State {store:Option<Store>,generation:u64,limiter:RateLimiter,cancellations:HashMap<String,Arc<AtomicBool>>}
pub struct Runtime {state:Mutex<State>,root:PathBuf,keys:Arc<dyn KeyProvider>,learner:LearnerCommand}
#[derive(Debug)]
pub struct AgentTicket {pub value:Value,workspace:String,epoch:i64,grant_revision:i64,generation:u64}
fn text<'a>(v:&'a Value,k:&str)->Result<&'a str>{v[k].as_str().ok_or_else(||Error::new("INPUT_INVALID"))}
fn number(v:&Value,k:&str)->Result<i64>{v[k].as_i64().ok_or_else(||Error::new("INPUT_INVALID"))}
fn fields(v:&Value,allowed:&[&str])->Result<()> {
    finite(v,0)?;let object=v.as_object().ok_or_else(||Error::new("INPUT_INVALID"))?;
    ensure(object.keys().all(|k|allowed.contains(&k.as_str())),"INPUT_INVALID")
}
fn safe_limit(p:&Value,max:usize,default:usize)->Result<usize>{let n=p.get("limit").map(|v|v.as_u64().ok_or_else(||Error::new("INPUT_INVALID"))).transpose()?.unwrap_or(default as u64);ensure(n<=max as u64,"INPUT_INVALID")?;Ok(n as usize)}

impl Runtime {
    pub fn new(root:PathBuf,keys:Arc<dyn KeyProvider>,learner:LearnerCommand)->Result<Arc<Self>>{
        fs::create_dir_all(&root)?;ensure(!fs::symlink_metadata(&root)?.file_type().is_symlink(),"UNSAFE_PATH")?;
        #[cfg(unix)]{use std::os::unix::fs::PermissionsExt;fs::set_permissions(&root,fs::Permissions::from_mode(0o700))?;}
        Ok(Arc::new(Self{state:Mutex::new(State{store:None,generation:0,limiter:RateLimiter::default(),cancellations:HashMap::new()}),root,keys,learner}))
    }
    pub fn socket_path(&self)->PathBuf{self.root.join("cdna.sock")}
    fn registry(&self)->Result<Vec<String>>{
        let path=self.root.join("vaults.json");if !path.exists(){return Ok(vec![]);}
        ensure(!fs::symlink_metadata(&path)?.file_type().is_symlink(),"UNSAFE_PATH")?;
        let raw=fs::read(path)?;ensure(raw.len()<=16384,"INVALID_REGISTRY")?;
        let ids:Vec<String>=serde_json::from_value(crate::transport::strict_json(&raw)?)?;
        ensure(ids.len()<=100&&ids.iter().all(|s|valid_uuid(s)),"INVALID_REGISTRY")?;Ok(ids)
    }
    fn register(&self,vid:&str)->Result<()> {
        let mut ids=self.registry()?;if !ids.iter().any(|v|v==vid){ids.push(vid.into());}
        ensure(ids.len()<=100,"VAULT_LIMIT")?;
        let temp=self.root.join(format!("registry-{}.tmp",id()));
        let mut options=fs::OpenOptions::new();options.create_new(true).write(true);
        #[cfg(unix)]{use std::os::unix::fs::OpenOptionsExt;options.mode(0o600).custom_flags(libc::O_NOFOLLOW);}
        let mut f=options.open(&temp)?;use std::io::Write;f.write_all(&serde_json::to_vec(&ids)?)?;f.sync_all()?;
        fs::rename(&temp,self.root.join("vaults.json"))?;Ok(())
    }
    fn lock_state(state:&mut State){
        for flag in state.cancellations.values(){flag.store(true,Ordering::SeqCst);}
        state.cancellations.clear();state.generation=state.generation.wrapping_add(1);state.limiter.clear();state.store.take();
    }
    /// Only the trusted Tauri owner window or an explicitly enabled test bridge
    /// calls this. There is no owner role field on the external protocol.
    pub fn owner_call(self:&Arc<Self>,method:&str,p:Value)->Result<Value>{
        let allowed=match method {
            "status"|"create_vault"|"lock"|"workspaces"|"seed_demo"=>vec![],
            "unlock"=>vec!["vault_id"],"create_workspace"=>vec!["name"],
            "dashboard"|"policies"|"models"|"jobs"|"grants"|"assessments"|"diagnostic_preview"|"improvements"|"analyze_improvements"|"consents"=>vec!["workspace_id"],
            "cases"=>vec!["workspace_id","limit","offset","domain","query","state"],
            "case"|"expose"=>vec!["workspace_id","case_id"],
            "confirm"|"undo"|"delete_case"=>vec!["workspace_id","case_id","expected_revision"],
            "revise_case"=>vec!["workspace_id","case_id","expected_revision","context","candidates","cause"],
            "answer"=>vec!["workspace_id","request_id","case_id","exposure_id","expected_revision","response"],
            "propose"=>vec!["proposal"],"rank"|"check_policy"=>vec!["request"],
            "next_question"=>vec!["workspace_id","domain"],"reversal_question"=>vec!["workspace_id","case_id","key","value"],
            "import_text"=>vec!["workspace_id","format","text"],
            "propose_policy"=>vec!["workspace_id","policy"],"policy_state"=>vec!["workspace_id","policy_id","expected_revision","state"],
            "train"=>vec!["workspace_id","request_id"],"cancel_job"|"job"=>vec!["workspace_id","job_id"],
            "activate_model"=>vec!["workspace_id","model_id","expected_epoch"],
            "create_grant"=>vec!["grant"],"revoke_grant"=>vec!["workspace_id","grant_id"],
            "set_consent"=>vec!["workspace_id","purpose","provider","allowed"],
            "assessment_import"=>vec!["workspace_id","result"],"assessment_delete"=>vec!["workspace_id","assessment_id"],
            "save_outcome"=>vec!["workspace_id","case_id","outcome"],
            "improvement_state"=>vec!["workspace_id","proposal_id","state"],
            "export_workspace"=>vec!["workspace_id","include_text","confirm_sensitive_export"],
            "backup_create"=>vec!["passphrase"],"backup_restore"=>vec!["passphrase","encrypted_backup"],
            _=>return Err(Error::new("METHOD_NOT_FOUND")),
        };
        fields(&p,&allowed)?;
        ensure(serde_json::to_vec(&p)?.len()<=if method.starts_with("backup_"){256*1024*1024}else if method=="import_text"{imports::MAX_DOCUMENT}else{MAX_REQUEST},"INPUT_TOO_LARGE")?;
        let mut state=self.state.lock().map_err(|_|Error::new("INTERNAL_ERROR"))?;
        if method=="status"{return Ok(json!({"schema_version":SCHEMA,"build":"0.1.0","vault_state":if state.store.is_some(){"unlocked"}else{"locked"},"vault_ids":self.registry()?,"active_vault_id":state.store.as_ref().map(|s|s.vault_id.clone()),"local_only":true,"learner_available":self.learner.available(),"person_agreement_verified":false}));}
        if method=="lock"{Self::lock_state(&mut state);return Ok(json!({"vault_state":"locked"}));}
        if method=="create_vault"{
            ensure(state.store.is_none(),"VAULT_ALREADY_OPEN")?;let vid=id();let key=random_key();
            self.keys.put(&vid,&key)?;
            let path=self.root.join(format!("{vid}.db"));
            let result=(||{let mut store=Store::open(&path,&vid,key,true)?;store.create_workspace("マイワークスペース",false)?;self.register(&vid)?;Ok::<Store,Error>(store)})();
            match result{Ok(store)=>{state.generation+=1;state.store=Some(store);return Ok(json!({"vault_id":vid,"vault_state":"unlocked"}));},Err(e)=>{let _=self.keys.delete(&vid);let _=fs::remove_file(path);return Err(e);}}
        }
        if method=="unlock"{
            ensure(state.store.is_none(),"VAULT_ALREADY_OPEN")?;let vid=text(&p,"vault_id")?;
            ensure(self.registry()?.iter().any(|v|v==vid),"VAULT_NOT_FOUND")?;
            let key=self.keys.get(vid)?;let store=Store::open(&self.root.join(format!("{vid}.db")),vid,key,false)?;
            state.generation+=1;state.store=Some(store);return Ok(json!({"vault_id":vid,"vault_state":"unlocked"}));
        }
        if method=="backup_restore"{
            let passphrase=Zeroizing::new(text(&p,"passphrase")?.to_owned());
            let bundle=backup::decrypt(text(&p,"encrypted_backup")?.as_bytes(),&passphrase)?;
            let tombstones=state.store.as_ref().map(|s|s.tombstones()).transpose()?.unwrap_or_default();
            let (new_store,key)=backup::restore_new(&self.root,&bundle,&tombstones)?;
            self.keys.put(&new_store.vault_id,&key)?;self.register(&new_store.vault_id)?;
            let vid=new_store.vault_id.clone();Self::lock_state(&mut state);state.store=Some(new_store);
            return Ok(json!({"vault_id":vid,"vault_state":"unlocked","models":"rebuild_required","unknown_offline_tombstones_cannot_be_inferred":true}));
        }
        if method=="train"{
            let w=text(&p,"workspace_id")?.to_owned();let request_id=text(&p,"request_id")?;ensure(valid_uuid(request_id),"INPUT_INVALID")?;
            ensure(self.learner.available(),"LEARNER_UNAVAILABLE")?;
            let generation=state.generation;
            let store=state.store.as_mut().ok_or_else(||Error::new("VAULT_LOCKED"))?;
            if let Some(job)=store.idempotent(&w,"owner:train",request_id,&p)?{return store.job(&w,text(&job,"job_id")?);}
            let snapshot=store.snapshot(&w)?;let job=store.new_job(&w,"owner","train")?;let jid=text(&job,"id")?.to_owned();
            store.set_job(&w,&jid,"running",None)?;
            use sha2::Digest;
            store.conn.execute("INSERT INTO idempotency VALUES(?1,'owner:train',?2,?3,?4)",params![w,request_id,hex::encode(sha2::Sha256::digest(serde_json::to_vec(&p)?)),json!({"job_id":jid}).to_string()])?;
            let cancel=Arc::new(AtomicBool::new(false));state.cancellations.insert(jid.clone(),cancel.clone());
            let runtime=self.clone();let worker=self.learner.clone();
            tokio::spawn(async move {
                let result=jobs::learn(worker,snapshot,cancel).await;
                if let Ok(mut state)=runtime.state.lock(){
                    state.cancellations.remove(&jid);
                    if state.generation!=generation{return;}
                    if let Some(store)=state.store.as_mut(){
                        match result {
                            Ok(model)=>{if let Err(e)=store.accept_model(&w,&jid,model){let _=store.set_job(&w,&jid,"failed",Some(&e.code));}},
                            Err(e)=>{let _=store.set_job(&w,&jid,if e.code=="CANCELLED"{"cancelled"}else{"failed"},Some(&e.code));}
                        }
                    }
                }
            });
            return Ok(job);
        }
        if method=="cancel_job"{
            let w=text(&p,"workspace_id")?;let jid=text(&p,"job_id")?;
            let store=state.store.as_mut().ok_or_else(||Error::new("VAULT_LOCKED"))?;store.set_job(w,jid,"cancelled",Some("CANCELLED"))?;
            if let Some(flag)=state.cancellations.get(jid){flag.store(true,Ordering::SeqCst);}return Ok(json!({"id":jid,"state":"cancelled"}));
        }
        let store=state.store.as_mut().ok_or_else(||Error::new("VAULT_LOCKED"))?;
        match method {
            "workspaces"=>Ok(json!(store.workspaces()?)),
            "create_workspace"=>Ok(json!(store.create_workspace(text(&p,"name")?,false)?)),
            "seed_demo"=>{if let Some(ws)=store.workspaces()?.into_iter().find(|w|w.is_demo){Ok(json!(ws))}else{Ok(json!(questions::seed_demo(store)?))}},
            "dashboard"=>{let w=text(&p,"workspace_id")?;Ok(json!({"workspace":store.workspace(w)?,"gaps":questions::gaps(store,w,&Actor::Owner.domains())?,"models":store.models(w)?,"jobs":store.jobs(w)?,"evidence_status":"not_independently_verified","learner_available":self.learner.available()}))},
            "cases"=>{
                let w=text(&p,"workspace_id")?;let limit=safe_limit(&p,100,30)?;
                let offset=p.get("offset").map(|v|v.as_u64().ok_or_else(||Error::new("INPUT_INVALID"))).transpose()?.unwrap_or(0)as usize;
                let cases=store.cases(w,100000,0)?;
                let matches:Vec<_>=cases.into_iter().filter(|c|p.get("domain").and_then(Value::as_str).is_none_or(|d|c.domain==d)&&p.get("state").and_then(Value::as_str).is_none_or(|s|c.verification_state==s)&&p.get("query").and_then(Value::as_str).is_none_or(|q|c.context.summary.contains(q)||c.response.as_ref().is_some_and(|r|r.note.contains(q)))).collect();
                let count=matches.len();Ok(json!({"items":matches.into_iter().skip(offset).take(limit).collect::<Vec<_>>(),"total":count,"limit":limit,"offset":offset}))
            },
            "case"=>Ok(json!(store.case(text(&p,"workspace_id")?,text(&p,"case_id")?)?)),
            "expose"=>{let w=text(&p,"workspace_id")?;let cid=text(&p,"case_id")?;let exposure=store.expose(w,cid,"manual_review")?;Ok(json!({"case":store.case(w,cid)?,"exposure":exposure}))},
            "confirm"=>Ok(json!(store.confirm(text(&p,"workspace_id")?,text(&p,"case_id")?,number(&p,"expected_revision")?)?)),
            "undo"=>Ok(json!(store.undo(text(&p,"workspace_id")?,text(&p,"case_id")?,number(&p,"expected_revision")?)?)),
            "delete_case"=>store.delete_case(text(&p,"workspace_id")?,text(&p,"case_id")?,number(&p,"expected_revision")?),
            "revise_case"=>Ok(json!(store.revise_case(text(&p,"workspace_id")?,text(&p,"case_id")?,number(&p,"expected_revision")?,serde_json::from_value(p["context"].clone())?,serde_json::from_value(p["candidates"].clone())?,text(&p,"cause")?)?)),
            "answer"=>Ok(json!(store.answer(&serde_json::from_value(p)?)?)),
            "propose"=>Ok(json!(store.propose(&ObservationProposal::parse(p["proposal"].clone())?,"manual_proposal","owner")?)),
            "rank"=>Ok(json!(rank(store,&Actor::Owner,RankRequest::parse(p["request"].clone())?)?)),
            "check_policy"=>{let r=RankRequest::parse(p["request"].clone())?;Ok(json!(policy::check(&store.policies(&r.workspace_id)?,&r)))},
            "next_question"=>questions::next(store,text(&p,"workspace_id")?,p.get("domain").and_then(Value::as_str)),
            "reversal_question"=>questions::reversal(store,text(&p,"workspace_id")?,text(&p,"case_id")?,text(&p,"key")?,p["value"].clone()),
            "import_text"=>{let w=text(&p,"workspace_id")?;let raw=text(&p,"text")?;let parsed=match text(&p,"format")?{"summary"=>imports::parse_summary(raw.as_bytes())?,"json"=>imports::parse_json(raw.as_bytes())?,"html"=>imports::parse_html(raw.as_bytes())?,_=>return Err(Error::new("UNSUPPORTED_EXPORT_FORMAT"))};imports::ingest(store,w,parsed)},
            "policies"=>Ok(json!(store.policies(text(&p,"workspace_id")?)?)),
            "propose_policy"=>Ok(json!(store.propose_policy(text(&p,"workspace_id")?,serde_json::from_value(p["policy"].clone())?)?)),
            "policy_state"=>store.set_policy_state(text(&p,"workspace_id")?,text(&p,"policy_id")?,number(&p,"expected_revision")?,text(&p,"state")?),
            "models"=>Ok(json!(store.models(text(&p,"workspace_id")?)?)),
            "activate_model"=>store.activate_model(text(&p,"workspace_id")?,text(&p,"model_id")?,number(&p,"expected_epoch")?),
            "jobs"=>Ok(json!(store.jobs(text(&p,"workspace_id")?)?)),
            "job"=>store.job(text(&p,"workspace_id")?,text(&p,"job_id")?),
            "create_grant"=>{let(grant,token)=store.create_grant(serde_json::from_value(p["grant"].clone())?)?;let keychain_saved=crate::keys::store_mcp_token(&grant.id,&token).is_ok();Ok(json!({"grant":grant,"token":if keychain_saved{Value::Null}else{json!(token.as_str())},"keychain_saved":keychain_saved,"display_once":true}))},
            "grants"=>Ok(json!(store.grants(text(&p,"workspace_id")?)?)),
            "revoke_grant"=>{store.revoke_grant(text(&p,"workspace_id")?,text(&p,"grant_id")?)?;crate::keys::remove_mcp_token(text(&p,"grant_id")?);Ok(json!({"revoked":true}))},
            "consents"=>{let w=text(&p,"workspace_id")?;store.workspace(w)?;Ok(json!({"codex":store.consent(w,"cloud_processing","codex")?,"claude":store.consent(w,"cloud_processing","claude")?,"diagnostics":store.consent(w,"diagnostics","diagnostics")?,"personality":store.consent(w,"personality_external","none")?,"receiver_state":"not_configured"}))},
            "set_consent"=>{store.set_consent(text(&p,"workspace_id")?,text(&p,"purpose")?,text(&p,"provider")?,p["allowed"].as_bool().ok_or_else(||Error::new("INPUT_INVALID"))?)?;Ok(json!({"saved":true}))},
            "assessment_import"=>store.assessment(text(&p,"workspace_id")?,p["result"].clone()),
            "assessments"=>Ok(json!(store.assessments(text(&p,"workspace_id")?)?)),
            "assessment_delete"=>{store.delete_assessment(text(&p,"workspace_id")?,text(&p,"assessment_id")?)?;Ok(json!({"deleted":true}))},
            "save_outcome"=>store.save_outcome(text(&p,"workspace_id")?,text(&p,"case_id")?,p["outcome"].clone()),
            "diagnostic_preview"=>Ok(json!({"report":diagnostics::preview(store,text(&p,"workspace_id")?)?,"sent":false,"requires_explicit_export_or_send":true,"no_raw_content":true})),
            "improvements"=>Ok(json!(store.improvements(text(&p,"workspace_id")?)?)),
            "analyze_improvements"=>Ok(json!(diagnostics::improvement_proposals(store,text(&p,"workspace_id")?)?)),
            "improvement_state"=>store.transition_improvement(text(&p,"workspace_id")?,text(&p,"proposal_id")?,text(&p,"state")?),
            "export_workspace"=>{
                ensure(p["confirm_sensitive_export"]==true,"CONFIRMATION_REQUIRED")?;let w=text(&p,"workspace_id")?;
                let mut cases=store.cases(w,100000,0)?;if p["include_text"]!=true{for c in &mut cases{redact_case(c);}}
                Ok(json!({"format":"cdna-workspace-export","schema_version":"1.0","workspace":store.workspace(w)?,"exported_at":now(),"cases":cases,"credentials":"excluded","sensitive":true,"trust_flags_are_not_portable":true}))
            },
            "backup_create"=>{
                let passphrase=Zeroizing::new(text(&p,"passphrase")?.to_owned());
                let encrypted=backup::encrypt(&backup::export(store)?,&passphrase)?;
                Ok(json!({"format":"cdna-encrypted-backup","data":String::from_utf8(encrypted).map_err(|_|Error::new("BACKUP_ERROR"))?,"includes_models":false,"contains_sensitive_data":true}))
            },
            _=>Err(Error::new("METHOD_NOT_FOUND")),
        }
    }
    /// Native file dialogs supply bytes here, not paths supplied by a web page.
    pub fn import_bytes(&self,w:&str,format:&str,bytes:&[u8])->Result<Value>{
        let parsed=match format{"zip"=>imports::parse_archive(std::io::Cursor::new(bytes),bytes.len() as u64)?,"json"=>imports::parse_json(bytes)?,"html"=>imports::parse_html(bytes)?,_=>return Err(Error::new("UNSUPPORTED_EXPORT_FORMAT"))};
        let mut state=self.state.lock().map_err(|_|Error::new("INTERNAL_ERROR"))?;
        let store=state.store.as_mut().ok_or_else(||Error::new("VAULT_LOCKED"))?;imports::ingest(store,w,parsed)
    }
    pub fn agent_call(&self,connection_id:&str,token:&str,method:&str,p:Value)->Result<AgentTicket>{
        ensure(serde_json::to_vec(&p)?.len()<=MAX_REQUEST,"INPUT_TOO_LARGE")?;finite(&p,0)?;
        let mut state=self.state.lock().map_err(|_|Error::new("INTERNAL_ERROR"))?;
        let generation=state.generation;
        let (grant,hash)=state.store.as_ref().ok_or_else(||Error::new("VAULT_LOCKED"))?.grant(connection_id)?;
        ensure(verify_token(&hash,token),"SCOPE_DENIED")?;ensure(!grant.revoked,"SCOPE_DENIED")?;
        state.limiter.check(connection_id)?;
        let store=state.store.as_mut().ok_or_else(||Error::new("VAULT_LOCKED"))?;
        let w=text(&p,"workspace_id")?.to_owned();let epoch=store.epoch(&w)?;
        // Even capabilities has a bound workspace and an unexpired grant.
        ensure(grant.proposal.workspace_id==w&&chrono::DateTime::parse_from_rfc3339(&grant.proposal.expires_at).is_ok_and(|t|t>chrono::Utc::now()),"SCOPE_DENIED")?;
        let actor=Actor::Agent(grant.clone());
        let value=match method {
            "cdna_get_capabilities"=>{fields(&p,&["workspace_id"])?;json!({"schema_version":"1.0","vault_state":"unlocked","workspace_id":w,"feature_version":"1.0","max_candidates":16,"tools":MCP_TOOL_NAMES,"granted_scopes":grant.proposal.scopes,"domains":grant.proposal.domains,"execution_authorization":"none"})},
            "cdna_get_profile"=>{fields(&p,&["workspace_id"])?;actor.authorize(&w,"profile:read",None)?;
                let policies:Vec<_>=store.policies(&w)?.into_iter().filter(|p|p.status=="effective"&&(p.proposal.domain=="all"||actor.domains().contains(&p.proposal.domain))).map(|p|if actor.text_allowed(){json!(p)}else{json!({"id":p.id,"revision":p.revision,"domain":p.proposal.domain,"effect":p.proposal.effect,"status":p.status,"text_redacted":true})}).collect();
                json!({"workspace_id":w,"explicit_policies":policies,"learning":questions::gaps(store,&w,&actor.domains())?,"personality_included":false,"verification":"unverified","source":"confirmed_records_only"})},
            "cdna_find_decisions"=>{fields(&p,&["workspace_id","domain","limit","query"])?;let domain=p.get("domain").and_then(Value::as_str);actor.authorize(&w,"decision:read",domain)?;
                let limit=safe_limit(&p,20,5)?;let query=p.get("query").and_then(Value::as_str).unwrap_or("");ensure(query.len()<=2048,"INPUT_TOO_LARGE")?;
                let mut cases:Vec<_>=store.cases(&w,100000,0)?.into_iter().filter(|c|c.verification_state=="confirmed"&&actor.domains().contains(&c.domain)&&domain.is_none_or(|d|c.domain==d)&&c.context.summary.contains(query)).take(limit).collect();
                if !actor.text_allowed(){for c in &mut cases{redact_case(c);}}
                json!({"items":cases,"matching":"confirmed_text_filter","is_prediction":false})},
            "cdna_propose_observation"=>{let proposal=ObservationProposal::parse(p.clone())?;actor.authorize(&w,"decision:propose",Some(&proposal.domain))?;json!(store.propose(&proposal,"agent_proposal",actor.key())?)},
            "cdna_rank_options"=>{let r=RankRequest::parse(p.clone())?;actor.authorize(&w,"judgment:infer",Some(&r.domain))?;json!(rank(store,&actor,r)?)},
            "cdna_check_policy"=>{let r=RankRequest::parse(p.clone())?;actor.authorize(&w,"policy:read",Some(&r.domain))?;json!(policy::check(&store.policies(&w)?,&r))},
            "cdna_get_learning_gaps"=>{fields(&p,&["workspace_id"])?;actor.authorize(&w,"learning:read",None)?;questions::gaps(store,&w,&actor.domains())?},
            "cdna_enqueue_questions"=>{
                fields(&p,&["workspace_id","questions"])?;let qs=p["questions"].as_array().ok_or_else(||Error::new("INPUT_INVALID"))?;ensure((1..=5).contains(&qs.len()),"INPUT_INVALID")?;
                let mut ids=vec![];
                // Validate the complete batch before any write.
                let proposals=qs.iter().map(|q|ObservationProposal::parse(q.clone())).collect::<Result<Vec<_>>>()?;
                for q in &proposals{ensure(q.workspace_id==w&&q.case_kind=="hypothetical"&&q.response.is_none(),"INPUT_INVALID")?;actor.authorize(&w,"question:propose",Some(&q.domain))?;}
                for q in &proposals{ids.push(store.propose(q,"agent_question",actor.key())?.id);}
                json!({"question_ids":ids,"verification_state":"pending"})
            },
            "cdna_propose_answer"=>{
                fields(&p,&["workspace_id","request_id","case_id","response"])?;
                let c=store.case(&w,text(&p,"case_id")?)?;actor.authorize(&w,"answer:propose",Some(&c.domain))?;
                let response:Response=serde_json::from_value(p["response"].clone())?;response.validate(&c.candidates,&c.candidates.iter().map(|c|c.id.clone()).collect::<Vec<_>>())?;
                let proposal=ObservationProposal{schema_version:SCHEMA.into(),request_id:text(&p,"request_id")?.into(),workspace_id:w.clone(),case_kind:c.case_kind,family_id:c.family_id,domain:c.domain,context:c.context,candidates:c.candidates,response:Some(response),source:Source{kind:"agent".into(),reference:format!("answer-proposal:{}",c.id),quote:String::new()},model_exposure:true};
                json!(store.propose(&proposal,"agent_answer_proposal",actor.key())?)
            },
            "cdna_get_job"|"cdna_cancel_job"=>{
                fields(&p,&["workspace_id","job_id"])?;actor.authorize(&w,"job:read",None)?;
                let jid=text(&p,"job_id")?;let job=store.job(&w,jid)?;ensure(job["actor_id"]==actor.key(),"SCOPE_DENIED")?;
                if method=="cdna_cancel_job"{store.set_job(&w,jid,"cancelled",Some("CANCELLED"))?;json!({"id":jid,"state":"cancelled"})}else{job}
            },
            _=>return Err(Error::new("METHOD_NOT_FOUND")),
        };
        store.assert_epoch(&w,epoch)?;let(current,_)=store.grant(connection_id)?;ensure(current.revision==grant.revision&&!current.revoked,"SCOPE_DENIED")?;
        ensure(serde_json::to_vec(&value)?.len()<=MAX_REQUEST,"OUTPUT_TOO_LARGE")?;
        Ok(AgentTicket{value,workspace:w,epoch,grant_revision:grant.revision,generation})
    }
    pub fn validate_ticket(&self,connection_id:&str,token:&str,ticket:&AgentTicket)->Result<()> {
        let state=self.state.lock().map_err(|_|Error::new("INTERNAL_ERROR"))?;ensure(state.generation==ticket.generation,"VAULT_LOCKED")?;
        let store=state.store.as_ref().ok_or_else(||Error::new("VAULT_LOCKED"))?;store.assert_epoch(&ticket.workspace,ticket.epoch)?;
        let(g,hash)=store.grant(connection_id)?;ensure(verify_token(&hash,token)&&!g.revoked&&g.revision==ticket.grant_revision&&g.proposal.workspace_id==ticket.workspace&&chrono::DateTime::parse_from_rfc3339(&g.proposal.expires_at).is_ok_and(|t|t>chrono::Utc::now()),"SCOPE_DENIED")
    }
}
pub const MCP_TOOL_NAMES:[&str;11]=["cdna_get_capabilities","cdna_get_profile","cdna_find_decisions","cdna_propose_observation","cdna_rank_options","cdna_check_policy","cdna_get_learning_gaps","cdna_enqueue_questions","cdna_propose_answer","cdna_get_job","cdna_cancel_job"];
fn redact_case(c:&mut CaseRecord){c.context.summary.clear();c.context.facts.clear();c.context.unknown_fields.clear();c.source.quote.clear();c.source.reference.clear();for candidate in &mut c.candidates{candidate.text.clear();candidate.attributes.clear();}if let Some(r)=&mut c.response{r.note.clear();r.reversal_condition.clear();}}
fn rank(store:&Store,actor:&Actor,r:RankRequest)->Result<Prediction>{
    let began=Instant::now();let ws=store.workspace(&r.workspace_id)?;let epoch=ws.deletion_epoch;
    actor.authorize(&r.workspace_id,"judgment:infer",Some(&r.domain))?;
    if r.allow_cloud {ensure(store.consent(&r.workspace_id,"cloud_processing","codex")?||store.consent(&r.workspace_id,"cloud_processing","claude")?,"CONSENT_REQUIRED")?;}
    let policies:Vec<_>=store.policies(&r.workspace_id)?.into_iter().filter(|p|p.proposal.domain=="all"||actor.domains().contains(&p.proposal.domain)).collect();
    let policy_check=policy::check(&policies,&r);let excluded:BTreeSet<_>=policy_check.excluded.iter().filter_map(|v|v["candidate_id"].as_str()).collect();
    let candidates:Vec<_>=r.candidates.iter().filter(|c|!excluded.contains(c.id.as_str())).cloned().collect();
    let cx=features::context_values(&r.context)?;
    let mut reasons=vec![];let mut missing:Vec<_>=features::manifest().context_features.iter().zip(&cx.missing).filter(|(_,m)|**m>0.0).map(|(f,_)|json!({"field":f.key,"question":format!("{}の条件を確認してください。",f.key)})).collect();
    missing.extend(policy_check.missing.clone());
    let model=store.active_model(&r.workspace_id)?;
    let model_id=model.as_ref().map(|m|m["id"].clone()).unwrap_or(Value::Null);
    let mut ranking=vec![];
    let scoring_start=Instant::now();
    if let Some(model)=&model {
        let weights=model["weights"].as_array().ok_or_else(||Error::new("MODEL_INVALID"))?.iter().map(|v|v.as_f64().ok_or_else(||Error::new("MODEL_INVALID"))).collect::<Result<Vec<_>>>()?;
        ranking=features::rank(&features::score_rows(&weights,&features::transform(&r.domain,&r.context,&candidates)?)?,&candidates);
        if model["domain_support"][&r.domain].as_u64().unwrap_or(0)<3{reasons.push("LOW_EVIDENCE".to_string());}
        reasons.push("UNCALIBRATED_MODEL".into());
    }else{reasons.push("MODEL_UNAVAILABLE".into());}
    let scoring_ms=scoring_start.elapsed().as_secs_f64()*1000.0;
    if !cx.out_of_range.is_empty(){reasons.push("OUT_OF_DISTRIBUTION".into());}
    if candidates.iter().any(|c|features::manifest().candidate_features.iter().any(|k|!c.attributes.get(k).is_some_and(|v|v.is_number()))){reasons.push("LOW_EVIDENCE".into());}
    if ranking.len()>1&&ranking[0].raw_score-ranking[1].raw_score<=1e-6{reasons.push("CLOSE_CANDIDATES".into());}
    let status=if policy_check.conflict {reasons.push("CONFLICTING_PRINCIPLES".into());"policy_conflict"}else if !missing.is_empty(){reasons.push("INSUFFICIENT_CONTEXT".into());"needs_information"}else{"abstain"};
    let mut evidence=vec![];
    if r.include_evidence && actor.text_allowed() {
        let cases=store.cases(&r.workspace_id,100000,0)?;
        let mut examples:Vec<_>=cases.into_iter().filter(|c|c.domain==r.domain&&c.verification_state=="confirmed"&&c.context.as_of<=r.context.as_of).map(|c|{
            let differences:Vec<_>=features::manifest().context_features.iter().filter(|f|c.context.facts.iter().find(|a|a.key==f.key).map(|f|&f.value)!=r.context.facts.iter().find(|a|a.key==f.key).map(|f|&f.value)).map(|f|f.key.clone()).collect();(differences.len(),differences,c)
        }).collect();examples.sort_by_key(|v|v.0);
        evidence=examples.into_iter().take(3).map(|(_,differences,c)|json!({"kind":"past_case","case_id":c.id,"revision":c.revision,"domain":c.domain,"summary":c.context.summary,"source":c.source,"differences":differences,"same_judgment_not_guaranteed":true})).collect();
    }
    reasons.sort();reasons.dedup();store.assert_epoch(&r.workspace_id,epoch)?;
    Ok(Prediction{schema_version:SCHEMA.into(),request_id:r.request_id,status:status.into(),mode:r.mode.clone(),ranking,score_scope:if excluded.is_empty(){"all_presented_candidates"}else{"policy_compatible_candidates"}.into(),pairwise_probability:None,top_choice_agreement_estimate:None,calibration_status:"not_independently_validated".into(),reason_codes:reasons,missing_information:missing,evidence,excluded_candidates:policy_check.excluded,
        explanation:if policy_check.conflict{"承認済み方針が衝突しています。方針の確認が必要です。"}else if status=="needs_information"{"暫定の順位があっても、判断材料が不足しています。"}else if model.is_some(){"学習したモデルの暫定順位です。独立した本人一致率は未検証のため、本人判断として確定しません。"}else{"学習済みモデルがありません。確認済み記録と明示方針を参照できます。"}.into(),
        advisor:if r.mode=="advisor"{Some(json!({"status":"additional_evidence_required","message":"本人再現と成果改善は別の評価です。未選択案の成果は推定済みと扱いません。"}))}else{None},
        versions:json!({"model_id":model_id,"feature_schema":"1.0","policy_revision":ws.policy_revision,"deletion_epoch":epoch,"is_demo":ws.is_demo}),timing_ms:json!({"scoring":scoring_ms,"total":began.elapsed().as_secs_f64()*1000.0}),human_review_required:true,execution_authorization:"none".into()})
}
