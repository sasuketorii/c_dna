use crate::domain::*;
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Serialize, Deserialize};
use sha2::{Digest,Sha256};
use std::collections::{BTreeSet,HashMap};
use std::time::Instant;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

pub const SCOPES: [&str; 10]=["profile:read","decision:read","decision:propose","judgment:infer","policy:read","learning:read","question:propose","answer:propose","personality:read","job:read"];
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantProposal {
    pub client_name:String, pub workspace_id:String, pub scopes:Vec<String>, pub domains:Vec<String>,
    pub expires_at:String, pub include_text:bool,
}
impl GrantProposal {
    pub fn validate(&self)->Result<()> {
        ensure(valid_uuid(&self.workspace_id) && !self.client_name.is_empty() && self.client_name.len()<=128,"INPUT_INVALID")?;
        ensure(self.scopes.iter().all(|s|SCOPES.contains(&s.as_str())) && self.scopes.iter().collect::<BTreeSet<_>>().len()==self.scopes.len(),"INPUT_INVALID")?;
        ensure(!self.domains.is_empty() && self.domains.iter().all(|s|DOMAINS.contains(&s.as_str())),"INPUT_INVALID")?;
        let expires=DateTime::parse_from_rfc3339(&self.expires_at).map_err(|_|Error::new("INPUT_INVALID"))?;
        ensure(expires>Utc::now() && expires<=Utc::now()+chrono::Duration::days(365),"INPUT_INVALID")
    }
}
#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct Grant { pub id:String, pub proposal:GrantProposal, pub revoked:bool, pub revision:i64 }
impl Grant {
    pub fn allows(&self,workspace:&str,scope:&str,domain:Option<&str>)->Result<()> {
        let not_expired=DateTime::parse_from_rfc3339(&self.proposal.expires_at).is_ok_and(|t|t>Utc::now());
        ensure(!self.revoked && not_expired && self.proposal.workspace_id==workspace && self.proposal.scopes.iter().any(|s|s==scope)
            && domain.is_none_or(|d|self.proposal.domains.iter().any(|x|x==d)),"SCOPE_DENIED")
    }
}
pub fn new_secret()->Zeroizing<String> {
    let mut bytes=Zeroizing::new([0u8;32]); rand::rngs::OsRng.fill_bytes(bytes.as_mut());
    Zeroizing::new(hex::encode(bytes.as_slice()))
}
pub fn token_hash(token:&str)->[u8;32] {Sha256::digest(token.as_bytes()).into()}
pub fn verify_token(stored:&[u8],token:&str)->bool {
    stored.len()==32 && bool::from(stored.ct_eq(&token_hash(token)))
}
struct Bucket { tokens:f64, updated:Instant }
#[derive(Default)]
pub struct RateLimiter { buckets:HashMap<String,Bucket> }
impl RateLimiter {
    pub fn check(&mut self,client:&str)->Result<()> {
        let now=Instant::now();
        let b=self.buckets.entry(client.into()).or_insert(Bucket{tokens:10.0,updated:now});
        b.tokens=(b.tokens+now.duration_since(b.updated).as_secs_f64()).min(10.0); b.updated=now;
        ensure(b.tokens>=1.0,"RATE_LIMITED")?; b.tokens-=1.0; Ok(())
    }
    pub fn clear(&mut self) { self.buckets.clear(); }
}
#[derive(Debug,Clone)]
pub enum Actor { Owner, Agent(Grant) }
impl Actor {
    pub fn key(&self)->&str {match self {Self::Owner=>"owner",Self::Agent(g)=>&g.id}}
    pub fn owner(&self)->Result<()> {ensure(matches!(self,Self::Owner),"SCOPE_DENIED")}
    pub fn authorize(&self,w:&str,scope:&str,domain:Option<&str>)->Result<()> {
        match self {Self::Owner=>Ok(()),Self::Agent(g)=>g.allows(w,scope,domain)}
    }
    pub fn text_allowed(&self)->bool {match self {Self::Owner=>true,Self::Agent(g)=>g.proposal.include_text}}
    pub fn domains(&self)->Vec<String> {match self {Self::Owner=>DOMAINS.iter().map(|s|s.to_string()).collect(),Self::Agent(g)=>g.proposal.domains.clone()}}
}
