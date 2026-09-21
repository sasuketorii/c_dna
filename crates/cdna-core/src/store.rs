//! Encrypted, single-writer storage. Every derived object is workspace/epoch bound.
use crate::{auth::*, domain::*, features, policy::*};
use chrono::Utc;
use fs2::FileExt;
use rand::{seq::SliceRandom, RngCore};
use rusqlite::{params, Connection, OptionalExtension, OpenFlags};
use serde_json::{json, Value};
use sha2::{Digest,Sha256};
use std::{collections::BTreeSet, fs::{self, File, OpenOptions}, path::{Path,PathBuf}, time::Duration};
use zeroize::Zeroizing;

const MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS workspaces(id TEXT PRIMARY KEY,name TEXT NOT NULL,is_demo INTEGER NOT NULL CHECK(is_demo IN(0,1)),deletion_epoch INTEGER NOT NULL DEFAULT 0,policy_revision INTEGER NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS imports(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,provider TEXT NOT NULL,content_hash TEXT NOT NULL,created_at TEXT NOT NULL,UNIQUE(workspace_id,provider,content_hash),UNIQUE(workspace_id,id));
CREATE TABLE IF NOT EXISTS source_messages(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL,artifact_id TEXT NOT NULL,external_id TEXT NOT NULL,role TEXT NOT NULL,body TEXT NOT NULL,occurred_at TEXT,FOREIGN KEY(workspace_id,artifact_id) REFERENCES imports(workspace_id,id) ON DELETE CASCADE,UNIQUE(workspace_id,artifact_id,external_id),UNIQUE(workspace_id,id));
CREATE TABLE IF NOT EXISTS cases(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,family_id TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>=1),domain TEXT NOT NULL,context TEXT NOT NULL,case_kind TEXT NOT NULL,origin TEXT NOT NULL,source TEXT NOT NULL,source_id TEXT,verification_state TEXT NOT NULL,model_exposure INTEGER NOT NULL,created_at TEXT NOT NULL,confirmed_at TEXT,UNIQUE(workspace_id,id),FOREIGN KEY(workspace_id,source_id) REFERENCES source_messages(workspace_id,id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS case_revisions(workspace_id TEXT NOT NULL,case_id TEXT NOT NULL,revision INTEGER NOT NULL,body TEXT NOT NULL,cause TEXT NOT NULL,PRIMARY KEY(case_id,revision),FOREIGN KEY(workspace_id,case_id) REFERENCES cases(workspace_id,id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS candidates(workspace_id TEXT NOT NULL,case_id TEXT NOT NULL,id TEXT NOT NULL,position INTEGER NOT NULL,text TEXT NOT NULL,attributes TEXT NOT NULL,PRIMARY KEY(case_id,id),FOREIGN KEY(workspace_id,case_id) REFERENCES cases(workspace_id,id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS observations(workspace_id TEXT NOT NULL,case_id TEXT NOT NULL,revision INTEGER NOT NULL,response TEXT NOT NULL,state TEXT NOT NULL,recorded_at TEXT NOT NULL,PRIMARY KEY(case_id,revision),FOREIGN KEY(workspace_id,case_id) REFERENCES cases(workspace_id,id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS exposures(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL,case_id TEXT NOT NULL,revision INTEGER NOT NULL,display_order TEXT NOT NULL,shown_at TEXT NOT NULL,query_policy TEXT NOT NULL,model_exposure INTEGER NOT NULL,FOREIGN KEY(workspace_id,case_id) REFERENCES cases(workspace_id,id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS policies(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,revision INTEGER NOT NULL,status TEXT NOT NULL,body TEXT NOT NULL,created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS snapshots(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,epoch INTEGER NOT NULL,body TEXT NOT NULL,created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS models(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,epoch INTEGER NOT NULL,state TEXT NOT NULL,body TEXT NOT NULL,created_at TEXT NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS one_active_model ON models(workspace_id) WHERE state='active';
CREATE TABLE IF NOT EXISTS model_inputs(model_id TEXT NOT NULL REFERENCES models(id) ON DELETE CASCADE,case_id TEXT NOT NULL,revision INTEGER NOT NULL,PRIMARY KEY(model_id,case_id,revision));
CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,actor_id TEXT NOT NULL,epoch INTEGER NOT NULL,state TEXT NOT NULL,kind TEXT NOT NULL,created_at TEXT NOT NULL,result_id TEXT,error_code TEXT);
CREATE UNIQUE INDEX IF NOT EXISTS one_running_training ON jobs(workspace_id) WHERE kind='train' AND state IN('queued','running','validating');
CREATE TABLE IF NOT EXISTS grants(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,token_hash BLOB NOT NULL,proposal TEXT NOT NULL,revoked INTEGER NOT NULL DEFAULT 0,revision INTEGER NOT NULL DEFAULT 1);
CREATE TABLE IF NOT EXISTS consents(workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,purpose TEXT NOT NULL,provider TEXT NOT NULL,allowed INTEGER NOT NULL CHECK(allowed IN(0,1)),revision INTEGER NOT NULL,updated_at TEXT NOT NULL,PRIMARY KEY(workspace_id,purpose,provider));
CREATE TABLE IF NOT EXISTS idempotency(workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,actor_id TEXT NOT NULL,key TEXT NOT NULL,input_hash TEXT NOT NULL,response TEXT NOT NULL,PRIMARY KEY(workspace_id,actor_id,key));
CREATE TABLE IF NOT EXISTS tombstones(workspace_id TEXT NOT NULL,kind TEXT NOT NULL,id TEXT NOT NULL,deleted_at TEXT NOT NULL,PRIMARY KEY(workspace_id,kind,id));
CREATE TABLE IF NOT EXISTS audit(id TEXT PRIMARY KEY,workspace_id TEXT,event TEXT NOT NULL,object_id TEXT,created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS assessments(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,body TEXT NOT NULL,created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS outcomes(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL,case_id TEXT NOT NULL,body TEXT NOT NULL,created_at TEXT NOT NULL,FOREIGN KEY(workspace_id,case_id) REFERENCES cases(workspace_id,id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS improvements(id TEXT PRIMARY KEY,workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,epoch INTEGER NOT NULL,state TEXT NOT NULL,body TEXT NOT NULL,created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS outbox(id INTEGER PRIMARY KEY AUTOINCREMENT,workspace_id TEXT NOT NULL,event TEXT NOT NULL,object_id TEXT NOT NULL,created_at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS case_domain_time ON cases(workspace_id,domain,created_at);
CREATE INDEX IF NOT EXISTS case_source ON cases(workspace_id,source_id);
CREATE INDEX IF NOT EXISTS case_state ON cases(workspace_id,verification_state);
CREATE INDEX IF NOT EXISTS exposure_case ON exposures(workspace_id,case_id,revision);
CREATE INDEX IF NOT EXISTS job_scope ON jobs(workspace_id,state,created_at);
PRAGMA user_version=1;
"#;

pub struct Store {
    pub(crate) conn: Connection,
    pub vault_id: String,
    pub path: PathBuf,
    _key: Zeroizing<[u8;32]>,
    _lock: File,
}
fn private_dir(path:&Path)->Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;fs::set_permissions(path,fs::Permissions::from_mode(0o700))?;}
    Ok(())
}
pub fn random_key()->Zeroizing<[u8;32]> {let mut key=Zeroizing::new([0u8;32]);rand::rngs::OsRng.fill_bytes(key.as_mut());key}
impl Store {
    pub fn open(path: &Path, vault_id:&str, key:Zeroizing<[u8;32]>, create:bool)->Result<Self> {
        ensure(valid_uuid(vault_id),"INPUT_INVALID")?;
        ensure(create != path.exists(), if create {"VAULT_EXISTS"} else {"VAULT_NOT_FOUND"})?;
        let parent=path.parent().ok_or_else(||Error::new("IO_ERROR"))?;private_dir(parent)?;
        ensure(!fs::symlink_metadata(parent)?.file_type().is_symlink(),"UNSAFE_PATH")?;
        let lock_path=path.with_extension("lock");
        let mut opts=OpenOptions::new();opts.create(true).read(true).write(true).truncate(false);
        #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);}
        let lock=opts.open(&lock_path)?;lock.try_lock_exclusive().map_err(|_|Error::new("VAULT_IN_USE"))?;
        if path.exists() {ensure(!fs::symlink_metadata(path)?.file_type().is_symlink(),"UNSAFE_PATH")?;}
        let flags=OpenFlags::SQLITE_OPEN_READ_WRITE | if create {OpenFlags::SQLITE_OPEN_CREATE}else{OpenFlags::empty()} | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut conn=Connection::open_with_flags(path,flags)?;
        let pragma=Zeroizing::new(format!("PRAGMA key = \"x'{}'\";",hex::encode(key.as_slice())));
        conn.execute_batch(&pragma)?;
        let cipher:String=conn.query_row("PRAGMA cipher_version",[],|r|r.get(0)).map_err(|_|Error::new("ENCRYPTION_UNAVAILABLE"))?;
        ensure(!cipher.is_empty(),"ENCRYPTION_UNAVAILABLE")?;
        conn.query_row("SELECT count(*) FROM sqlite_master",[],|r|r.get::<_,i64>(0)).map_err(|_|Error::new("VAULT_UNLOCK_FAILED"))?;
        conn.execute_batch("PRAGMA foreign_keys=ON;PRAGMA temp_store=MEMORY;PRAGMA cipher_memory_security=ON;PRAGMA secure_delete=ON;PRAGMA journal_mode=WAL;PRAGMA synchronous=FULL;")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        ensure(conn.query_row("PRAGMA foreign_keys",[],|r|r.get::<_,i64>(0))?==1,"STORE_ERROR")?;
        let version:i64=conn.query_row("PRAGMA user_version",[],|r|r.get(0))?;
        ensure(version<=1,"SCHEMA_UNSUPPORTED")?;
        if version==0 {
            let tx=conn.transaction()?;tx.execute_batch(MIGRATION)?;
            tx.execute("INSERT INTO meta(key,value) VALUES('vault_id',?1),('deletion_epoch','0')",[vault_id])?;tx.commit()?;
        }
        let actual:String=conn.query_row("SELECT value FROM meta WHERE key='vault_id'",[],|r|r.get(0))?;
        ensure(actual==vault_id,"VAULT_UNLOCK_FAILED")?;
        conn.execute("UPDATE jobs SET state='failed',error_code='WORKER_INTERRUPTED' WHERE state IN('queued','running','validating')",[])?;
        #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;fs::set_permissions(path,fs::Permissions::from_mode(0o600))?;}
        Ok(Self{conn,vault_id:vault_id.into(),path:path.into(),_key:key,_lock:lock})
    }
    pub fn checkpoint(&self)->Result<()> {self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;Ok(())}
    pub fn workspace(&self,w:&str)->Result<Workspace> {
        self.conn.query_row("SELECT id,name,is_demo,deletion_epoch,policy_revision FROM workspaces WHERE id=?1",[w],|r|Ok(Workspace{id:r.get(0)?,name:r.get(1)?,is_demo:r.get(2)?,deletion_epoch:r.get(3)?,policy_revision:r.get(4)?})).optional()?.ok_or_else(||Error::new("SCOPE_DENIED"))
    }
    pub fn workspaces(&self)->Result<Vec<Workspace>> {
        let mut st=self.conn.prepare("SELECT id,name,is_demo,deletion_epoch,policy_revision FROM workspaces ORDER BY rowid")?;
        Ok(st.query_map([],|r|Ok(Workspace{id:r.get(0)?,name:r.get(1)?,is_demo:r.get(2)?,deletion_epoch:r.get(3)?,policy_revision:r.get(4)?}))?.collect::<std::result::Result<Vec<_>,_>>()?)
    }
    pub fn create_workspace(&mut self,name:&str,is_demo:bool)->Result<Workspace> {
        ensure(!name.trim().is_empty() && name.len()<=256,"INPUT_INVALID")?;
        let w=id();self.conn.execute("INSERT INTO workspaces(id,name,is_demo) VALUES(?1,?2,?3)",params![w,name,is_demo])?;
        self.workspace(&w)
    }
    pub fn epoch(&self,w:&str)->Result<i64> {Ok(self.workspace(w)?.deletion_epoch)}
    pub fn assert_epoch(&self,w:&str,epoch:i64)->Result<()> {ensure(self.epoch(w)?==epoch,"MODEL_INVALIDATED")}
    pub fn audit(&self,w:&str,event:&str,object_id:&str)->Result<()> {
        self.conn.execute("INSERT INTO audit VALUES(?1,?2,?3,?4,?5)",params![id(),w,event,object_id,now()])?;Ok(())
    }
    pub fn idempotent(&self,w:&str,actor:&str,key:&str,input:&Value)->Result<Option<Value>> {
        ensure(valid_uuid(key),"INPUT_INVALID")?;
        let hash=hex::encode(Sha256::digest(serde_json::to_vec(input)?));
        let row:Option<(String,String)>=self.conn.query_row("SELECT input_hash,response FROM idempotency WHERE workspace_id=?1 AND actor_id=?2 AND key=?3",params![w,actor,key],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        match row {Some((old,raw))=>{ensure(hash==old,"CONFLICT")?;Ok(Some(serde_json::from_str(&raw)?))},None=>Ok(None)}
    }
    pub fn propose(&mut self,p:&ObservationProposal,origin:&str,actor:&str)->Result<CaseRecord> {
        self.workspace(&p.workspace_id)?;
        ensure(origin!="synthetic_seed" || self.workspace(&p.workspace_id)?.is_demo,"SCOPE_DENIED")?;
        let input=serde_json::to_value(p)?;
        if let Some(v)=self.idempotent(&p.workspace_id,actor,&p.request_id,&input)? {return self.case(&p.workspace_id,v["case_id"].as_str().ok_or_else(||Error::new("STORE_ERROR"))?);}
        let source_id=p.source.reference.strip_prefix("cdna-source:").map(str::to_owned);
        if let Some(s)=&source_id {
            let exists:bool=self.conn.query_row("SELECT EXISTS(SELECT 1 FROM source_messages WHERE workspace_id=?1 AND id=?2)",params![p.workspace_id,s],|r|r.get(0))?;ensure(exists,"SCOPE_DENIED")?;
        }
        let case_id=id();let ts=now();
        let tx=self.conn.transaction()?;
        tx.execute("INSERT INTO cases(id,workspace_id,family_id,revision,domain,context,case_kind,origin,source,source_id,verification_state,model_exposure,created_at) VALUES(?1,?2,?3,1,?4,?5,?6,?7,?8,?9,'pending',?10,?11)",params![case_id,p.workspace_id,p.family_id,p.domain,serde_json::to_string(&p.context)?,p.case_kind,origin,serde_json::to_string(&p.source)?,source_id,p.model_exposure,ts])?;
        for (position,c) in p.candidates.iter().enumerate() {tx.execute("INSERT INTO candidates VALUES(?1,?2,?3,?4,?5,?6)",params![p.workspace_id,case_id,c.id,position as i64,c.text,serde_json::to_string(&c.attributes)?])?;}
        if let Some(r)=&p.response {tx.execute("INSERT INTO observations VALUES(?1,?2,1,?3,'pending',?4)",params![p.workspace_id,case_id,serde_json::to_string(r)?,ts])?;}
        tx.execute("INSERT INTO idempotency VALUES(?1,?2,?3,?4,?5)",params![p.workspace_id,actor,p.request_id,hex::encode(Sha256::digest(serde_json::to_vec(&input)?)),json!({"case_id":case_id}).to_string()])?;
        tx.execute("INSERT INTO outbox(workspace_id,event,object_id,created_at) VALUES(?1,'observation_proposed',?2,?3)",params![p.workspace_id,case_id,ts])?;tx.commit()?;
        self.case(&p.workspace_id,&case_id)
    }
    pub fn case(&self,w:&str,case_id:&str)->Result<CaseRecord> {
        let mut value=self.conn.query_row("SELECT id,workspace_id,family_id,revision,domain,context,case_kind,origin,source,verification_state,model_exposure,created_at,confirmed_at FROM cases WHERE workspace_id=?1 AND id=?2",params![w,case_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?,r.get::<_,String>(7)?,r.get::<_,String>(8)?,r.get::<_,String>(9)?,r.get::<_,bool>(10)?,r.get::<_,String>(11)?,r.get::<_,Option<String>>(12)?))).optional()?.ok_or_else(||Error::new("SCOPE_DENIED"))?;
        let mut st=self.conn.prepare("SELECT id,text,attributes FROM candidates WHERE workspace_id=?1 AND case_id=?2 ORDER BY position")?;
        let raw=st.query_map(params![w,case_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let candidates=raw.into_iter().map(|(id,text,attributes)|Ok(Candidate{id,text,attributes:serde_json::from_str(&attributes)?})).collect::<Result<Vec<_>>>()?;
        let response:Option<String>=self.conn.query_row("SELECT response FROM observations WHERE workspace_id=?1 AND case_id=?2 AND revision=?3",params![w,case_id,value.3],|r|r.get(0)).optional()?;
        Ok(CaseRecord{id:std::mem::take(&mut value.0),workspace_id:value.1,family_id:value.2,revision:value.3,domain:value.4,context:serde_json::from_str(&value.5)?,case_kind:value.6,origin:value.7,source:serde_json::from_str(&value.8)?,verification_state:value.9,model_exposure:value.10,created_at:value.11,confirmed_at:value.12,candidates,response:response.map(|s|serde_json::from_str(&s)).transpose()?})
    }
    pub fn cases(&self,w:&str,limit:usize,offset:usize)->Result<Vec<CaseRecord>> {
        self.workspace(w)?;ensure(limit<=100000,"INPUT_INVALID")?;
        let ids={let mut st=self.conn.prepare("SELECT id FROM cases WHERE workspace_id=?1 ORDER BY created_at DESC,id LIMIT ?2 OFFSET ?3")?;st.query_map(params![w,limit as i64,offset as i64],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?};
        ids.iter().map(|i|self.case(w,i)).collect()
    }
    pub fn expose(&mut self,w:&str,case_id:&str,query_policy:&str)->Result<Exposure> {
        let case=self.case(w,case_id)?;
        let mut order:Vec<_>=case.candidates.iter().map(|c|c.id.clone()).collect();order.shuffle(&mut rand::thread_rng());
        let e=Exposure{id:id(),case_id:case.id,workspace_id:w.into(),revision:case.revision,display_order:order,shown_at:now(),query_policy:query_policy.into(),model_exposure:false};
        self.conn.execute("INSERT INTO exposures VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![e.id,w,e.case_id,e.revision,serde_json::to_string(&e.display_order)?,e.shown_at,e.query_policy,e.model_exposure])?;Ok(e)
    }
    pub fn exposure(&self,w:&str,exposure_id:&str)->Result<Exposure> {
        let row=self.conn.query_row("SELECT id,case_id,revision,display_order,shown_at,query_policy,model_exposure FROM exposures WHERE workspace_id=?1 AND id=?2",params![w,exposure_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,bool>(6)?))).optional()?.ok_or_else(||Error::new("SCOPE_DENIED"))?;
        Ok(Exposure{id:row.0,workspace_id:w.into(),case_id:row.1,revision:row.2,display_order:serde_json::from_str(&row.3)?,shown_at:row.4,query_policy:row.5,model_exposure:row.6})
    }
    /// Only a trusted UI command can call this path. MCP receives propose_answer.
    pub fn answer(&mut self,r:&AnswerRequest)->Result<CaseRecord> {
        let input=serde_json::to_value(r)?;
        if let Some(v)=self.idempotent(&r.workspace_id,"owner",&r.request_id,&input)? {return self.case(&r.workspace_id,v["case_id"].as_str().ok_or_else(||Error::new("STORE_ERROR"))?);}
        let case=self.case(&r.workspace_id,&r.case_id)?;
        let exposure=self.exposure(&r.workspace_id,&r.exposure_id)?;
        ensure(case.revision==r.expected_revision && exposure.revision==r.expected_revision,"REVISION_CONFLICT")?;
        ensure(exposure.case_id==r.case_id,"INVALID_EXPOSURE")?;
        r.response.validate(&case.candidates,&exposure.display_order)?;
        let previous_confirmed=case.verification_state=="confirmed";
        let rev=case.revision+1;let ts=now();
        let tx=self.conn.transaction()?;
        tx.execute("UPDATE observations SET state='superseded' WHERE workspace_id=?1 AND case_id=?2",params![r.workspace_id,r.case_id])?;
        tx.execute("INSERT INTO observations VALUES(?1,?2,?3,?4,'confirmed',?5)",params![r.workspace_id,r.case_id,rev,serde_json::to_string(&r.response)?,ts])?;
        let changed=tx.execute("UPDATE cases SET revision=?1,verification_state='confirmed',confirmed_at=?2,origin=CASE WHEN origin='synthetic_seed' THEN origin ELSE 'human_ui' END,model_exposure=?3 WHERE workspace_id=?4 AND id=?5 AND revision=?6",params![rev,ts,exposure.model_exposure,r.workspace_id,r.case_id,r.expected_revision])?;ensure(changed==1,"REVISION_CONFLICT")?;
        if previous_confirmed {invalidate(&tx,&r.workspace_id,"corrected")?;}
        tx.execute("INSERT INTO idempotency VALUES(?1,'owner',?2,?3,?4)",params![r.workspace_id,r.request_id,hex::encode(Sha256::digest(serde_json::to_vec(&input)?)),json!({"case_id":r.case_id,"revision":rev}).to_string()])?;
        tx.execute("INSERT INTO outbox(workspace_id,event,object_id,created_at) VALUES(?1,'answer_saved',?2,?3)",params![r.workspace_id,r.case_id,ts])?;tx.commit()?;
        self.case(&r.workspace_id,&r.case_id)
    }
    pub fn confirm(&mut self,w:&str,case_id:&str,revision:i64)->Result<CaseRecord> {
        let case=self.case(w,case_id)?;ensure(case.revision==revision,"REVISION_CONFLICT")?;
        ensure(case.verification_state=="pending","INVALID_STATE")?;
        let origin=if self.workspace(w)?.is_demo && case.origin=="synthetic_seed" {"synthetic_seed"} else if case.source.kind=="summary" {"endorsed_statement"} else {"human_confirmed_import"};
        let tx=self.conn.transaction()?;
        tx.execute("UPDATE cases SET verification_state='confirmed',confirmed_at=?1,origin=?2 WHERE workspace_id=?3 AND id=?4 AND revision=?5",params![now(),origin,w,case_id,revision])?;
        tx.execute("UPDATE observations SET state='confirmed' WHERE workspace_id=?1 AND case_id=?2 AND revision=?3",params![w,case_id,revision])?;
        tx.commit()?;self.case(w,case_id)
    }
    pub fn undo(&mut self,w:&str,case_id:&str,revision:i64)->Result<CaseRecord> {
        let case=self.case(w,case_id)?;ensure(case.revision==revision,"REVISION_CONFLICT")?;
        let previous:Option<String>=self.conn.query_row("SELECT response FROM observations WHERE workspace_id=?1 AND case_id=?2 AND revision<?3 AND state!='invalidated' ORDER BY revision DESC LIMIT 1",params![w,case_id,revision],|r|r.get(0)).optional()?;
        let tx=self.conn.transaction()?;invalidate(&tx,w,"answer_undone")?;
        tx.execute("UPDATE observations SET state='superseded' WHERE workspace_id=?1 AND case_id=?2",params![w,case_id])?;
        tx.execute("UPDATE cases SET revision=revision+1,verification_state='pending',confirmed_at=NULL WHERE workspace_id=?1 AND id=?2",params![w,case_id])?;
        if let Some(old)=previous {tx.execute("INSERT INTO observations VALUES(?1,?2,?3,?4,'pending',?5)",params![w,case_id,revision+1,old,now()])?;}
        tx.commit()?;self.case(w,case_id)
    }
    pub fn delete_case(&mut self,w:&str,case_id:&str,revision:i64)->Result<Value> {
        let case=self.case(w,case_id)?;ensure(case.revision==revision,"REVISION_CONFLICT")?;
        let source_id:Option<String>=self.conn.query_row("SELECT source_id FROM cases WHERE workspace_id=?1 AND id=?2",params![w,case_id],|r|r.get(0))?;
        let mut ids=vec![case_id.to_owned()];
        if let Some(s)=&source_id {let mut st=self.conn.prepare("SELECT id FROM cases WHERE workspace_id=?1 AND source_id=?2")?;ids=st.query_map(params![w,s],|r|r.get(0))?.collect::<std::result::Result<Vec<String>,_>>()?;}
        let derived_policy_ids:Vec<String>=self.policies(w)?.iter().filter(|p|p.proposal.source_case_ids.iter().any(|s|ids.contains(s))).map(|p|p.id.clone()).collect();
        let tx=self.conn.transaction()?;invalidate(&tx,w,"deleted")?;
        for pid in derived_policy_ids {tx.execute("DELETE FROM policies WHERE workspace_id=?1 AND id=?2",params![w,pid])?;tx.execute("INSERT OR REPLACE INTO tombstones VALUES(?1,'policy',?2,?3)",params![w,pid,now()])?;}
        tx.execute("UPDATE workspaces SET policy_revision=policy_revision+1 WHERE id=?1",[w])?;
        for i in &ids {tx.execute("INSERT OR REPLACE INTO tombstones VALUES(?1,'case',?2,?3)",params![w,i,now()])?;tx.execute("DELETE FROM cases WHERE workspace_id=?1 AND id=?2",params![w,i])?;}
        if let Some(s)=source_id {tx.execute("INSERT OR REPLACE INTO tombstones VALUES(?1,'source',?2,?3)",params![w,s,now()])?;tx.execute("DELETE FROM source_messages WHERE workspace_id=?1 AND id=?2",params![w,s])?;}
        tx.execute("INSERT INTO audit VALUES(?1,?2,'deletion_completed',?3,?4)",params![id(),w,case_id,now()])?;tx.commit()?;
        Ok(json!({"deleted_case_ids":ids,"deletion_epoch":self.epoch(w)?,"derived_models":"invalidated_and_erased","physical_ssd_erasure_guaranteed":false,"external_copies_deleted":false}))
    }
    pub fn policies(&self,w:&str)->Result<Vec<PolicyRecord>> {
        self.workspace(w)?;
        let rows={let mut st=self.conn.prepare("SELECT id,revision,status,body FROM policies WHERE workspace_id=?1 ORDER BY created_at")?;st.query_map([w],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?)))?.collect::<std::result::Result<Vec<_>,_>>()?};
        rows.into_iter().map(|(id,revision,status,body)|Ok(PolicyRecord{id,workspace_id:w.into(),revision,status,proposal:serde_json::from_str(&body)?})).collect()
    }
    pub fn propose_policy(&mut self,w:&str,p:PolicyProposal)->Result<PolicyRecord> {
        self.workspace(w)?;p.validate()?;
        for case_id in &p.source_case_ids {self.case(w,case_id)?;}
        let pid=id();self.conn.execute("INSERT INTO policies VALUES(?1,?2,1,'hypothesis',?3,?4)",params![pid,w,serde_json::to_string(&p)?,now()])?;
        Ok(PolicyRecord{id:pid,workspace_id:w.into(),revision:1,status:"hypothesis".into(),proposal:p})
    }
    pub fn set_policy_state(&mut self,w:&str,pid:&str,revision:i64,state:&str)->Result<Value> {
        ensure(["effective","revoked"].contains(&state),"INPUT_INVALID")?;
        let old=self.policies(w)?.into_iter().find(|p|p.id==pid).ok_or_else(||Error::new("SCOPE_DENIED"))?;
        ensure(old.revision==revision,"REVISION_CONFLICT")?;
        let tx=self.conn.transaction()?;
        tx.execute("UPDATE policies SET status=?1,revision=revision+1 WHERE workspace_id=?2 AND id=?3 AND revision=?4",params![state,w,pid,revision])?;
        tx.execute("UPDATE workspaces SET policy_revision=policy_revision+1 WHERE id=?1",[w])?;invalidate(&tx,w,"policy_changed")?;tx.commit()?;
        Ok(json!({"id":pid,"status":state,"revision":revision+1}))
    }
    pub fn snapshot(&mut self,w:&str)->Result<Value> {
        let ws=self.workspace(w)?;
        let records:Vec<Value>=self.cases(w,100000,0)?.into_iter().filter(|c|c.verification_state=="confirmed" && (c.origin=="human_ui" || c.origin=="human_confirmed_import" || ws.is_demo && c.origin=="synthetic_seed") && c.response.as_ref().is_some_and(Response::learnable) && c.candidates.len()>=2).map(|c|json!({"case_id":c.id,"family_id":c.family_id,"revision":c.revision,"as_of":c.context.as_of,"domain":c.domain,"context":c.context,"candidates":c.candidates,"response":c.response,"verified":true,"origin":c.origin,"model_exposure":c.model_exposure,"case_kind":c.case_kind,"weight":1.0})).collect();
        ensure(!records.is_empty(),"INSUFFICIENT_TRAINING_DATA")?;
        let sid=id();let snapshot=json!({"schema_version":"1.0","snapshot_id":sid,"workspace_id":w,"deletion_epoch":ws.deletion_epoch,"feature_version":"1.0","is_demo":ws.is_demo,"records":records,"seed":17,"compare_lightgbm":true});
        self.conn.execute("INSERT INTO snapshots VALUES(?1,?2,?3,?4,?5)",params![sid,w,ws.deletion_epoch,snapshot.to_string(),now()])?;Ok(snapshot)
    }
    pub fn new_job(&mut self,w:&str,actor_id:&str,kind:&str)->Result<Value> {
        let epoch=self.epoch(w)?;
        let running:i64=self.conn.query_row("SELECT count(*) FROM jobs WHERE state IN('queued','running','validating') AND kind='train'",[],|r|r.get(0))?;ensure(running==0,"JOB_BUSY")?;
        let jid=id();self.conn.execute("INSERT INTO jobs(id,workspace_id,actor_id,epoch,state,kind,created_at) VALUES(?1,?2,?3,?4,'queued',?5,?6)",params![jid,w,actor_id,epoch,kind,now()])?;self.job(w,&jid)
    }
    pub fn job(&self,w:&str,jid:&str)->Result<Value> {
        self.conn.query_row("SELECT id,actor_id,epoch,state,kind,created_at,result_id,error_code FROM jobs WHERE workspace_id=?1 AND id=?2",params![w,jid],|r|Ok(json!({"id":r.get::<_,String>(0)?,"workspace_id":w,"actor_id":r.get::<_,String>(1)?,"deletion_epoch":r.get::<_,i64>(2)?,"state":r.get::<_,String>(3)?,"kind":r.get::<_,String>(4)?,"created_at":r.get::<_,String>(5)?,"result_id":r.get::<_,Option<String>>(6)?,"error_code":r.get::<_,Option<String>>(7)?}))).optional()?.ok_or_else(||Error::new("SCOPE_DENIED"))
    }
    pub fn set_job(&mut self,w:&str,jid:&str,state:&str,error:Option<&str>)->Result<()> {
        ensure(["running","validating","succeeded","failed","cancelled","invalidated"].contains(&state),"INPUT_INVALID")?;
        let old=self.job(w,jid)?;
        ensure(["queued","running","validating"].contains(&old["state"].as_str().unwrap_or("")),"INVALID_STATE")?;
        self.conn.execute("UPDATE jobs SET state=?1,error_code=?2 WHERE workspace_id=?3 AND id=?4",params![state,error,w,jid])?;Ok(())
    }
    pub fn accept_model(&mut self,w:&str,jid:&str,model:Value)->Result<String> {
        let job=self.job(w,jid)?;let epoch=job["deletion_epoch"].as_i64().ok_or_else(||Error::new("STORE_ERROR"))?;
        self.assert_epoch(w,epoch)?;ensure(["running","validating"].contains(&job["state"].as_str().unwrap_or("")),"CANCELLED")?;
        validate_model(&model,w,epoch)?;
        let sid=model["snapshot_id"].as_str().ok_or_else(||Error::new("MODEL_INVALID"))?;
        let saved:Option<String>=self.conn.query_row("SELECT body FROM snapshots WHERE id=?1 AND workspace_id=?2 AND epoch=?3",params![sid,w,epoch],|r|r.get(0)).optional()?;ensure(saved.is_some(),"MODEL_INVALIDATED")?;
        let snapshot:Value=serde_json::from_str(&saved.unwrap_or_default())?;
        let model_id=model["id"].as_str().ok_or_else(||Error::new("MODEL_INVALID"))?.to_owned();
        let tx=self.conn.transaction()?;
        tx.execute("INSERT INTO models VALUES(?1,?2,?3,'evaluated',?4,?5)",params![model_id,w,epoch,model.to_string(),now()])?;
        if let Some(records)=snapshot["records"].as_array() {for r in records {tx.execute("INSERT INTO model_inputs VALUES(?1,?2,?3)",params![model_id,r["case_id"].as_str(),r["revision"].as_i64()])?;}}
        tx.execute("UPDATE jobs SET state='succeeded',result_id=?1 WHERE workspace_id=?2 AND id=?3",params![model_id,w,jid])?;tx.commit()?;Ok(model_id)
    }
    pub fn models(&self,w:&str)->Result<Vec<Value>> {
        self.workspace(w)?;let mut st=self.conn.prepare("SELECT id,epoch,state,body,created_at FROM models WHERE workspace_id=?1 ORDER BY created_at DESC")?;
        let rows=st.query_map([w],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        rows.into_iter().map(|(id,epoch,state,raw,created)|{let body:Value=serde_json::from_str(&raw)?;Ok(json!({"id":id,"deletion_epoch":epoch,"state":state,"created_at":created,"kind":body["kind"],"provisional":true,"is_demo":body["is_demo"],"training":body["training"],"evaluation":body["evaluation"],"calibration":body["calibration"]}))}).collect()
    }
    pub fn active_model(&self,w:&str)->Result<Option<Value>> {
        let epoch=self.epoch(w)?;
        let raw:Option<String>=self.conn.query_row("SELECT body FROM models WHERE workspace_id=?1 AND state='active' AND epoch=?2",params![w,epoch],|r|r.get(0)).optional()?;
        raw.map(|s|{let m:Value=serde_json::from_str(&s)?;validate_model(&m,w,epoch)?;Ok(m)}).transpose()
    }
    pub fn activate_model(&mut self,w:&str,mid:&str,expected_epoch:i64)->Result<Value> {
        self.assert_epoch(w,expected_epoch)?;
        let row:Option<(String,String)>=self.conn.query_row("SELECT state,body FROM models WHERE workspace_id=?1 AND id=?2 AND epoch=?3",params![w,mid,expected_epoch],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (state,raw)=row.ok_or_else(||Error::new("MODEL_INVALIDATED"))?;
        ensure(["evaluated","retired","approved"].contains(&state.as_str()),"INVALID_STATE")?;
        let model:Value=serde_json::from_str(&raw)?;validate_model(&model,w,expected_epoch)?;
        let tx=self.conn.transaction()?;
        tx.execute("UPDATE models SET state='retired' WHERE workspace_id=?1 AND state='active'",[w])?;
        tx.execute("UPDATE models SET state='active' WHERE workspace_id=?1 AND id=?2 AND epoch=?3",params![w,mid,expected_epoch])?;
        tx.execute("INSERT INTO audit VALUES(?1,?2,'model_activated',?3,?4)",params![id(),w,mid,now()])?;tx.commit()?;
        Ok(json!({"id":mid,"state":"active","provisional":true,"person_agreement_verified":false}))
    }
    pub fn grant(&self,gid:&str)->Result<(Grant,Vec<u8>)> {
        let row=self.conn.query_row("SELECT proposal,token_hash,revoked,revision FROM grants WHERE id=?1",[gid],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,bool>(2)?,r.get::<_,i64>(3)?))).optional()?.ok_or_else(||Error::new("SCOPE_DENIED"))?;
        Ok((Grant{id:gid.into(),proposal:serde_json::from_str(&row.0)?,revoked:row.2,revision:row.3},row.1))
    }
    pub fn create_grant(&mut self,p:GrantProposal)->Result<(Grant,Zeroizing<String>)> {
        p.validate()?;self.workspace(&p.workspace_id)?;
        let gid=id();let token=new_secret();
        self.conn.execute("INSERT INTO grants(id,workspace_id,token_hash,proposal) VALUES(?1,?2,?3,?4)",params![gid,p.workspace_id,token_hash(&token).as_slice(),serde_json::to_string(&p)?])?;
        Ok((Grant{id:gid,proposal:p,revoked:false,revision:1},token))
    }
    pub fn revoke_grant(&mut self,w:&str,gid:&str)->Result<()> {
        let changed=self.conn.execute("UPDATE grants SET revoked=1,revision=revision+1 WHERE workspace_id=?1 AND id=?2",params![w,gid])?;ensure(changed==1,"SCOPE_DENIED")?;
        self.conn.execute("UPDATE jobs SET state='invalidated',error_code='SCOPE_DENIED' WHERE workspace_id=?1 AND actor_id=?2 AND state IN('queued','running','validating')",params![w,gid])?;Ok(())
    }
    pub fn grants(&self,w:&str)->Result<Vec<Grant>> {
        let ids={let mut st=self.conn.prepare("SELECT id FROM grants WHERE workspace_id=?1")?;st.query_map([w],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?};ids.iter().map(|i|self.grant(i).map(|r|r.0)).collect()
    }
    pub fn consent(&self,w:&str,purpose:&str,provider:&str)->Result<bool> {
        Ok(self.conn.query_row("SELECT allowed FROM consents WHERE workspace_id=?1 AND purpose=?2 AND provider=?3",params![w,purpose,provider],|r|r.get(0)).optional()?.unwrap_or(false))
    }
    pub fn set_consent(&mut self,w:&str,purpose:&str,provider:&str,allowed:bool)->Result<()> {
        self.workspace(w)?;
        ensure(["cloud_processing","diagnostics","personality_external"].contains(&purpose) && ["codex","claude","diagnostics","none"].contains(&provider),"INPUT_INVALID")?;
        self.conn.execute("INSERT INTO consents VALUES(?1,?2,?3,?4,1,?5) ON CONFLICT(workspace_id,purpose,provider) DO UPDATE SET allowed=excluded.allowed,revision=consents.revision+1,updated_at=excluded.updated_at",params![w,purpose,provider,allowed,now()])?;
        if !allowed {self.conn.execute("UPDATE jobs SET state='invalidated',error_code='CONSENT_REQUIRED' WHERE workspace_id=?1 AND kind='agent' AND state IN('queued','running','validating')",[w])?;}
        Ok(())
    }
    pub fn tombstones(&self)->Result<Vec<Value>> {
        let mut st=self.conn.prepare("SELECT workspace_id,kind,id,deleted_at FROM tombstones")?;
        Ok(st.query_map([],|r|Ok(json!({"workspace_id":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"id":r.get::<_,String>(2)?,"deleted_at":r.get::<_,String>(3)?})))?.collect::<std::result::Result<Vec<_>,_>>()?)
    }
    pub fn assessment(&mut self,w:&str,body:Value)->Result<Value> {
        self.workspace(w)?;crate::diagnostics::validate_assessment(&body)?;
        let aid=id();self.conn.execute("INSERT INTO assessments VALUES(?1,?2,?3,?4)",params![aid,w,body.to_string(),now()])?;Ok(json!({"id":aid,"body":body}))
    }
    pub fn assessments(&self,w:&str)->Result<Vec<Value>> {
        let mut st=self.conn.prepare("SELECT id,body FROM assessments WHERE workspace_id=?1 ORDER BY created_at DESC")?;
        let rows=st.query_map([w],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        rows.into_iter().map(|(id,b)|Ok(json!({"id":id,"body":serde_json::from_str::<Value>(&b)?}))).collect()
    }
    pub fn delete_assessment(&mut self,w:&str,aid:&str)->Result<()> {
        let tx=self.conn.transaction()?;
        ensure(tx.execute("DELETE FROM assessments WHERE workspace_id=?1 AND id=?2",params![w,aid])?==1,"SCOPE_DENIED")?;
        tx.execute("INSERT OR REPLACE INTO tombstones VALUES(?1,'assessment',?2,?3)",params![w,aid,now()])?;invalidate(&tx,w,"assessment_deleted")?;tx.commit()?;Ok(())
    }
    pub fn save_outcome(&mut self,w:&str,case_id:&str,body:Value)->Result<Value> {
        self.case(w,case_id)?;ensure(body.is_object() && serde_json::to_vec(&body)?.len()<=16384,"INPUT_INVALID")?;
        let allowed=["observed_at","period_end","metric","value","unit","currency","status","external_factors","reflection"];
        ensure(body.as_object().is_some_and(|o|o.keys().all(|k|allowed.contains(&k.as_str()))) && body["observed_at"].as_str().is_some_and(valid_time),"INPUT_INVALID")?;
        let oid=id();self.conn.execute("INSERT INTO outcomes VALUES(?1,?2,?3,?4,?5)",params![oid,w,case_id,body.to_string(),now()])?;Ok(json!({"id":oid,"causal_effect_established":false}))
    }
    /// Context corrections are revisions, not replacements of historical evidence.
    /// Introducing alternatives into an imported case turns it into a hypothetical
    /// question; it must be answered again before it can be training data.
    pub fn revise_case(&mut self,w:&str,cid:&str,expected:i64,context:Context,candidates:Vec<Candidate>,cause:&str)->Result<CaseRecord>{
        let old=self.case(w,cid)?;ensure(old.revision==expected,"REVISION_CONFLICT")?;
        ensure(["correction","context_change","policy_change","accidental_input"].contains(&cause),"INPUT_INVALID")?;
        context.validate()?;validate_candidates(&candidates)?;ensure(candidates.len()<=16,"INPUT_INVALID")?;
        let changed_candidates=old.candidates!=candidates;
        let tx=self.conn.transaction()?;
        tx.execute("INSERT OR IGNORE INTO case_revisions VALUES(?1,?2,?3,?4,?5)",params![w,cid,expected,serde_json::to_string(&old)?,cause])?;
        invalidate(&tx,w,"corrected")?;
        tx.execute("UPDATE observations SET state='superseded' WHERE workspace_id=?1 AND case_id=?2",params![w,cid])?;
        tx.execute("DELETE FROM candidates WHERE workspace_id=?1 AND case_id=?2",params![w,cid])?;
        for(position,c)in candidates.iter().enumerate(){tx.execute("INSERT INTO candidates VALUES(?1,?2,?3,?4,?5,?6)",params![w,cid,c.id,position as i64,c.text,serde_json::to_string(&c.attributes)?])?;}
        tx.execute("UPDATE cases SET revision=revision+1,context=?1,case_kind=?2,verification_state='pending',confirmed_at=NULL WHERE workspace_id=?3 AND id=?4 AND revision=?5",params![serde_json::to_string(&context)?,if changed_candidates {"hypothetical"}else{old.case_kind.as_str()},w,cid,expected])?;
        tx.commit()?;self.case(w,cid)
    }
    pub fn jobs(&self,w:&str)->Result<Vec<Value>>{
        self.workspace(w)?;
        let ids={let mut st=self.conn.prepare("SELECT id FROM jobs WHERE workspace_id=?1 ORDER BY created_at DESC LIMIT 50")?;let rows=st.query_map([w],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;rows};
        ids.iter().map(|jid|self.job(w,jid)).collect()
    }
    pub fn improvements(&self,w:&str)->Result<Vec<Value>>{
        self.workspace(w)?;let mut st=self.conn.prepare("SELECT body,state FROM improvements WHERE workspace_id=?1 AND state!='invalidated' ORDER BY rowid DESC LIMIT 30")?;
        let rows=st.query_map([w],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        rows.into_iter().map(|(raw,state)|{let mut v:Value=serde_json::from_str(&raw)?;v["state"]=json!(state);Ok(v)}).collect()
    }
    pub fn transition_improvement(&mut self,w:&str,pid:&str,next:&str)->Result<Value>{
        let epoch=self.epoch(w)?;
        let row:Option<(String,String)>=self.conn.query_row("SELECT state,body FROM improvements WHERE workspace_id=?1 AND id=?2 AND epoch=?3",params![w,pid,epoch],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let (old,raw)=row.ok_or_else(||Error::new("SCOPE_DENIED"))?;
        ensure(matches!((old.as_str(),next),("proposed","approved_for_experiment")|("proposed","rejected")|("approved_for_experiment","rejected")|("evaluated","activated")|("evaluated","rejected")|("activated","rolled_back")),"INVALID_STATE")?;
        let mut body:Value=serde_json::from_str(&raw)?;
        // Activation requires measured evidence from the evaluator, never an agent claim.
        if next=="activated"{ensure(body["measured_gain"].is_number(),"EVALUATION_REQUIRED")?;}
        body["state"]=json!(next);self.conn.execute("UPDATE improvements SET state=?1,body=?2 WHERE workspace_id=?3 AND id=?4 AND epoch=?5",params![next,body.to_string(),w,pid,epoch])?;
        Ok(body)
    }
    pub fn invalidate_all(&mut self,w:&str,reason:&str)->Result<()> {let tx=self.conn.transaction()?;invalidate(&tx,w,reason)?;tx.commit()?;Ok(())}
}
pub(crate) fn invalidate(conn:&Connection,w:&str,_reason:&str)->Result<()> {
    conn.execute("UPDATE workspaces SET deletion_epoch=deletion_epoch+1 WHERE id=?1",[w])?;
    conn.execute("UPDATE meta SET value=CAST(CAST(value AS INTEGER)+1 AS TEXT) WHERE key='deletion_epoch'",[])?;
    conn.execute("UPDATE models SET state='deletion_invalidated',body='{}' WHERE workspace_id=?1",[w])?;
    conn.execute("DELETE FROM model_inputs WHERE model_id IN(SELECT id FROM models WHERE workspace_id=?1)",[w])?;
    conn.execute("DELETE FROM snapshots WHERE workspace_id=?1",[w])?;
    conn.execute("DELETE FROM idempotency WHERE workspace_id=?1",[w])?;
    conn.execute("UPDATE jobs SET state='invalidated',error_code='MODEL_INVALIDATED' WHERE workspace_id=?1 AND state IN('queued','running','validating')",[w])?;
    conn.execute("UPDATE improvements SET state='invalidated',body='{}' WHERE workspace_id=?1",[w])?;
    Ok(())
}
pub fn validate_model(m:&Value,w:&str,epoch:i64)->Result<()> {
    finite(m,0)?;
    ensure(m["schema_version"]=="1.0" && m["feature_version"]=="1.0" && m["workspace_id"]==w && m["deletion_epoch"].as_i64()==Some(epoch),"MODEL_INVALIDATED")?;
    ensure(m["feature_manifest_sha256"]==features::manifest_hash() && m["kind"]=="linear_pairwise" && m["dtype"]=="float64" && m["intercept"].as_f64()==Some(0.0),"MODEL_INVALID")?;
    ensure(m["id"].as_str().is_some_and(valid_uuid) && m["snapshot_id"].as_str().is_some_and(valid_uuid),"MODEL_INVALID")?;
    let weights=m["weights"].as_array().ok_or_else(||Error::new("MODEL_INVALID"))?;
    ensure(weights.len()==features::dimension() && weights.iter().all(|v|v.as_f64().is_some_and(f64::is_finite)),"MODEL_INVALID")?;
    ensure(m["feature_names"]==json!(features::names()),"MODEL_INVALID")
}
