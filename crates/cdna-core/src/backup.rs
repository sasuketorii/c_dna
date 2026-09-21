//! Portable authenticated backups. A restore is built in a NEW vault before swap.
use crate::{domain::*,store::{Store,random_key},policy::PolicyRecord};
use argon2::{Argon2,Algorithm,Version,Params};
use base64::{engine::general_purpose::STANDARD as B64,Engine};
use chacha20poly1305::{XChaCha20Poly1305,KeyInit,XNonce,aead::{Aead,Payload}};
use rand::RngCore;
use rusqlite::params;
use serde::{Serialize,Deserialize};
use serde_json::{json,Value};
use std::{collections::BTreeSet,path::Path};
use zeroize::Zeroizing;

const MAX_BACKUP:usize=128*1024*1024;
const AAD:&[u8]=b"C-DNA authenticated backup v1";
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope{format:String,version:u32,kdf:String,salt:String,nonce:String,ciphertext:String}
fn derive(passphrase:&str,salt:&[u8])->Result<Zeroizing<[u8;32]>>{
    ensure(passphrase.chars().count()>=12&&passphrase.len()<=1024&&salt.len()==16,"INVALID_PASSPHRASE")?;
    let params=Params::new(65536,3,1,Some(32)).map_err(|_|Error::new("CRYPTO_ERROR"))?;
    let mut key=Zeroizing::new([0u8;32]);
    Argon2::new(Algorithm::Argon2id,Version::V0x13,params).hash_password_into(passphrase.as_bytes(),salt,key.as_mut()).map_err(|_|Error::new("CRYPTO_ERROR"))?;Ok(key)
}
pub fn encrypt(bundle:&Value,passphrase:&str)->Result<Vec<u8>>{
    let raw=Zeroizing::new(serde_json::to_vec(bundle)?);ensure(raw.len()<=MAX_BACKUP,"BACKUP_TOO_LARGE")?;
    let mut salt=[0u8;16];let mut nonce=[0u8;24];rand::rngs::OsRng.fill_bytes(&mut salt);rand::rngs::OsRng.fill_bytes(&mut nonce);
    let key=derive(passphrase,&salt)?;
    let cipher=XChaCha20Poly1305::new_from_slice(key.as_slice()).map_err(|_|Error::new("CRYPTO_ERROR"))?;
    let encrypted=cipher.encrypt(XNonce::from_slice(&nonce),Payload{msg:&raw,aad:AAD}).map_err(|_|Error::new("CRYPTO_ERROR"))?;
    Ok(serde_json::to_vec(&Envelope{format:"cdna-encrypted-backup".into(),version:1,kdf:"argon2id-m65536-t3-p1".into(),salt:B64.encode(salt),nonce:B64.encode(nonce),ciphertext:B64.encode(encrypted)})?)
}
pub fn decrypt(raw:&[u8],passphrase:&str)->Result<Value>{
    ensure(raw.len()<=MAX_BACKUP*2,"BACKUP_TOO_LARGE")?;
    let envelope:Envelope=serde_json::from_slice(raw)?;
    ensure(envelope.format=="cdna-encrypted-backup"&&envelope.version==1&&envelope.kdf=="argon2id-m65536-t3-p1","SCHEMA_UNSUPPORTED")?;
    let salt=B64.decode(envelope.salt).map_err(|_|Error::new("INVALID_BACKUP"))?;
    let nonce=B64.decode(envelope.nonce).map_err(|_|Error::new("INVALID_BACKUP"))?;
    let ciphertext=B64.decode(envelope.ciphertext).map_err(|_|Error::new("INVALID_BACKUP"))?;
    ensure(nonce.len()==24&&ciphertext.len()<=MAX_BACKUP+16,"INVALID_BACKUP")?;
    let key=derive(passphrase,&salt)?;
    let cipher=XChaCha20Poly1305::new_from_slice(key.as_slice()).map_err(|_|Error::new("CRYPTO_ERROR"))?;
    let plain=Zeroizing::new(cipher.decrypt(XNonce::from_slice(&nonce),Payload{msg:&ciphertext,aad:AAD}).map_err(|_|Error::new("BACKUP_AUTH_FAILED"))?);
    crate::transport::strict_json(&plain)
}
pub fn export(store:&Store)->Result<Value>{
    let mut workspaces=vec![];
    for ws in store.workspaces()? {
        let imports=rows(store,"SELECT id,provider,content_hash,created_at FROM imports WHERE workspace_id=?1",&ws.id,&["id","provider","content_hash","created_at"])?;
        let messages=rows(store,"SELECT id,artifact_id,external_id,role,body,occurred_at FROM source_messages WHERE workspace_id=?1",&ws.id,&["id","artifact_id","external_id","role","body","occurred_at"])?;
        let outcomes=rows(store,"SELECT id,case_id,body,created_at FROM outcomes WHERE workspace_id=?1",&ws.id,&["id","case_id","body","created_at"])?;
        workspaces.push(json!({"workspace":ws,"cases":store.cases(&ws.id,100000,0)?,"policies":store.policies(&ws.id)?,"assessments":store.assessments(&ws.id)?,"imports":imports,"messages":messages,"outcomes":outcomes}));
    }
    Ok(json!({"format":"cdna-portable","schema_version":"1.0","source_vault_id":store.vault_id,"created_at":now(),"workspaces":workspaces,"tombstones":store.tombstones()?,"models":"rebuild_required","credentials":"excluded","consents":"require_reapproval"}))
}
fn rows(store:&Store,sql:&str,w:&str,columns:&[&str])->Result<Vec<Value>>{
    let mut st=store.conn.prepare(sql)?;
    Ok(st.query_map([w],|r|{let mut o=serde_json::Map::new();for(i,k)in columns.iter().enumerate(){o.insert((*k).into(),json!(r.get::<_,Option<String>>(i)?));}Ok(Value::Object(o))})?.collect::<std::result::Result<Vec<_>,_>>()?)
}
/// No changes are made to the old store. Caller adopts the returned vault only
/// after the new DB passes all validation and is saved in the OS credential store.
pub fn restore_new(root:&Path,bundle:&Value,known_tombstones:&[Value])->Result<(Store,Zeroizing<[u8;32]>)>{
    ensure(bundle["format"]=="cdna-portable"&&bundle["schema_version"]=="1.0","SCHEMA_UNSUPPORTED")?;
    let spaces=bundle["workspaces"].as_array().ok_or_else(||Error::new("INVALID_BACKUP"))?;ensure(spaces.len()<=100,"INVALID_BACKUP")?;
    let mut tombstones=known_tombstones.to_vec();
    tombstones.extend(bundle["tombstones"].as_array().ok_or_else(||Error::new("INVALID_BACKUP"))?.clone());
    let deleted:BTreeSet<(String,String,String)>=tombstones.iter().filter_map(|t|Some((t["workspace_id"].as_str()?.into(),t["kind"].as_str()?.into(),t["id"].as_str()?.into()))).collect();
    let vault_id=id();let key=random_key();let returned_key=Zeroizing::new(*key);
    let path=root.join(format!("{vault_id}.db"));
    let mut store=Store::open(&path,&vault_id,key,true)?;
    let result=(||->Result<()>{
        let tx=store.conn.transaction()?;
        let mut case_ids=BTreeSet::new();
        for item in spaces {
            let ws:Workspace=serde_json::from_value(item["workspace"].clone())?;ensure(valid_uuid(&ws.id),"INVALID_BACKUP")?;
            tx.execute("INSERT INTO workspaces VALUES(?1,?2,?3,?4,?5)",params![ws.id,ws.name,ws.is_demo,ws.deletion_epoch+1,ws.policy_revision+1])?;
            for a in item["imports"].as_array().ok_or_else(||Error::new("INVALID_BACKUP"))? {
                ensure(a["id"].as_str().is_some_and(valid_uuid),"INVALID_BACKUP")?;
                tx.execute("INSERT INTO imports VALUES(?1,?2,?3,?4,?5)",params![a["id"].as_str(),ws.id,a["provider"].as_str(),a["content_hash"].as_str(),a["created_at"].as_str()])?;
            }
            let mut restored_sources=BTreeSet::new();
            for m in item["messages"].as_array().ok_or_else(||Error::new("INVALID_BACKUP"))? {
                let mid=m["id"].as_str().ok_or_else(||Error::new("INVALID_BACKUP"))?;
                if deleted.contains(&(ws.id.clone(),"source".into(),mid.into())){continue;}
                ensure(valid_uuid(mid)&&m["body"].as_str().is_some_and(|s|s.len()<=crate::imports::MAX_MESSAGE),"INVALID_BACKUP")?;
                tx.execute("INSERT INTO source_messages VALUES(?1,?2,?3,?4,?5,?6,?7)",params![mid,ws.id,m["artifact_id"].as_str(),m["external_id"].as_str(),m["role"].as_str(),m["body"].as_str(),m["occurred_at"].as_str()])?;
                restored_sources.insert(mid.to_owned());
            }
            for raw in item["cases"].as_array().ok_or_else(||Error::new("INVALID_BACKUP"))? {
                let case:CaseRecord=serde_json::from_value(raw.clone())?;
                ensure(case.workspace_id==ws.id&&valid_uuid(&case.id)&&valid_uuid(&case.family_id)&&case.revision>=1,"INVALID_BACKUP")?;
                if deleted.contains(&(ws.id.clone(),"case".into(),case.id.clone())){continue;}
                let sid=case.source.reference.strip_prefix("cdna-source:");
                if sid.is_some_and(|s|!restored_sources.contains(s)){continue;}
                case.context.validate()?;validate_candidates(&case.candidates)?;
                ensure(["pending","confirmed"].contains(&case.verification_state.as_str()),"INVALID_BACKUP")?;
                tx.execute("INSERT INTO cases VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",params![case.id,ws.id,case.family_id,case.revision,case.domain,serde_json::to_string(&case.context)?,case.case_kind,case.origin,serde_json::to_string(&case.source)?,sid,case.verification_state,case.model_exposure,case.created_at,case.confirmed_at])?;
                for(i,c)in case.candidates.iter().enumerate(){tx.execute("INSERT INTO candidates VALUES(?1,?2,?3,?4,?5,?6)",params![ws.id,case.id,c.id,i as i64,c.text,serde_json::to_string(&c.attributes)?])?;}
                if let Some(r)=case.response {
                    // Pairwise answers are stored by canonical selected IDs; display
                    // order is not restored as an active, reusable exposure token.
                    ensure(r.selected_ids.iter().all(|id|case.candidates.iter().any(|c|&c.id==id)),"INVALID_BACKUP")?;
                    tx.execute("INSERT INTO observations VALUES(?1,?2,?3,?4,?5,?6)",params![ws.id,case.id,case.revision,serde_json::to_string(&r)?,case.verification_state,now()])?;
                }
                case_ids.insert(case.id);
            }
            for raw in item["policies"].as_array().ok_or_else(||Error::new("INVALID_BACKUP"))? {
                let p:PolicyRecord=serde_json::from_value(raw.clone())?;ensure(p.workspace_id==ws.id,"INVALID_BACKUP")?;p.proposal.validate()?;
                // Explicit approval is requested again because operational context may
                // have changed since this backup was made.
                tx.execute("INSERT INTO policies VALUES(?1,?2,?3,'hypothesis',?4,?5)",params![p.id,ws.id,p.revision+1,serde_json::to_string(&p.proposal)?,now()])?;
            }
            for a in item["assessments"].as_array().ok_or_else(||Error::new("INVALID_BACKUP"))? {
                let aid=a["id"].as_str().ok_or_else(||Error::new("INVALID_BACKUP"))?;
                if deleted.contains(&(ws.id.clone(),"assessment".into(),aid.into())){continue;}
                crate::diagnostics::validate_assessment(&a["body"])?;
                tx.execute("INSERT INTO assessments VALUES(?1,?2,?3,?4)",params![aid,ws.id,a["body"].to_string(),now()])?;
            }
            for o in item["outcomes"].as_array().ok_or_else(||Error::new("INVALID_BACKUP"))? {
                let cid=o["case_id"].as_str().ok_or_else(||Error::new("INVALID_BACKUP"))?;
                if case_ids.contains(cid){tx.execute("INSERT INTO outcomes VALUES(?1,?2,?3,?4,?5)",params![o["id"].as_str(),ws.id,cid,o["body"].as_str(),o["created_at"].as_str()])?;}
            }
        }
        for t in &tombstones {tx.execute("INSERT OR IGNORE INTO tombstones VALUES(?1,?2,?3,?4)",params![t["workspace_id"].as_str(),t["kind"].as_str(),t["id"].as_str(),t["deleted_at"].as_str()])?;}
        tx.commit()?;store.checkpoint()?;Ok(())
    })();
    match result {
        Ok(())=>Ok((store,returned_key)),
        Err(e)=>{drop(store);let _=std::fs::remove_file(&path);let _=std::fs::remove_file(path.with_extension("db-wal"));let _=std::fs::remove_file(path.with_extension("db-shm"));Err(e)}
    }
}
