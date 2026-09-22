use anyhow::{Result,ensure};
use cdna_domain::{Context,Candidate,PolicyExpr,Truth,Fact,FactValue,EvidenceStatus};
use chrono::{DateTime,Utc};
use serde::{Deserialize,Serialize};
use serde_json::{Value,json};
use uuid::Uuid;
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub statement:String,
    pub when:PolicyExpr,
    pub requires:PolicyExpr,
    pub effective_from:DateTime<Utc>,
    pub expires_at:Option<DateTime<Utc>>,
}
impl Policy {
    pub fn validate(&self)->Result<()> {
        ensure!(!self.statement.trim().is_empty()&&self.statement.len()<=4096,"INPUT_INVALID: policy statement");
        ensure!(self.expires_at.is_none_or(|end|end>self.effective_from),"INPUT_INVALID: policy period");
        let empty=Context{as_of:Utc::now(),summary:"validation".into(),facts:vec![],unknown_fields:vec![]};
        self.when.evaluate(&empty)?;self.requires.evaluate(&empty)?;Ok(())
    }
}
pub fn check(policies:&[(Uuid,Value)],context:&Context,candidates:&[Candidate])->Result<Value> {
    let mut results=Vec::new();
    for c in candidates {
        let option=Context{as_of:context.as_of,summary:c.text.clone(),facts:c.attributes.iter().map(|(key,value)|Fact{key:key.clone(),value:value.clone(),evidence_status:if matches!(value,FactValue::Null){EvidenceStatus::Unknown}else{EvidenceStatus::Explicit},unit:if matches!(value,FactValue::Number(_)){Some("ratio".into())}else{None}}).collect(),unknown_fields:vec![]};
        let mut checks=Vec::new();let mut violated=false;let mut unknown=false;let mut applied=false;
        for (id,value) in policies {
            if value["status"]!="approved"{continue;}
            let policy:Policy=serde_json::from_value(value["policy"].clone())?;
            let now=Utc::now();if now<policy.effective_from||policy.expires_at.is_some_and(|e|now>=e){continue;}
            let condition=policy.when.evaluate(context)?;
            let status=match condition {Truth::False=>"not_applicable",Truth::Unknown=>{unknown=true;"unknown"},Truth::True=>{applied=true;match policy.requires.evaluate(&option)? {Truth::True=>"compliant",Truth::False=>{violated=true;"violated"},Truth::Unknown=>{unknown=true;"unknown"}}}};
            checks.push(json!({"policy_id":id,"statement":policy.statement,"status":status}));
        }
        results.push(json!({"candidate_id":c.id,"status":if violated{"violated"}else if unknown{"unknown"}else if applied{"compliant"}else{"no_applicable_policy"},"checks":checks}));
    }
    Ok(json!({"mode":"policy_check","candidates":results,"execution_authorization":"none"}))
}
