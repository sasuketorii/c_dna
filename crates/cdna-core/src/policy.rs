//! Bounded three-valued policy language. Never eval/shell/SQL.
use crate::domain::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Truth { True, False, Unknown }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Expr {
    All { args: Vec<Expr> }, Any { args: Vec<Expr> }, Not { arg: Box<Expr> },
    Exists { field: String },
    Eq { field: String, value: Value, #[serde(default)] unit: Option<String> },
    In { field: String, values: Vec<Value> },
    Lt { field: String, value: Value, #[serde(default)] unit: Option<String> },
    Lte { field: String, value: Value, #[serde(default)] unit: Option<String> },
    Gt { field: String, value: Value, #[serde(default)] unit: Option<String> },
    Gte { field: String, value: Value, #[serde(default)] unit: Option<String> },
}
impl Expr {
    pub fn validate(&self, depth: usize) -> Result<()> {
        ensure(depth <= 8, "POLICY_TOO_COMPLEX")?;
        match self {
            Self::All{args} | Self::Any{args} => {
                ensure(!args.is_empty() && args.len() <= 16, "POLICY_TOO_COMPLEX")?;
                for a in args { a.validate(depth+1)?; }
            },
            Self::Not{arg} => arg.validate(depth+1)?,
            Self::In{field,values} => {
                valid_field(field)?; ensure(!values.is_empty() && values.len() <= 32, "INPUT_INVALID")?;
                for v in values { validate_fact_value(v)?; }
            },
            Self::Exists{field} => valid_field(field)?,
            Self::Eq{field,value,..} | Self::Lt{field,value,..} | Self::Lte{field,value,..} | Self::Gt{field,value,..} | Self::Gte{field,value,..} => {
                valid_field(field)?; validate_fact_value(value)?;
            },
        }
        Ok(())
    }
    pub fn evaluate(&self, context: &Context, candidate: &Candidate) -> Truth {
        use Truth::*;
        match self {
            Self::All{args} => { let a: Vec<_> = args.iter().map(|a|a.evaluate(context,candidate)).collect(); if a.contains(&False) {False} else if a.contains(&Unknown) {Unknown} else {True} },
            Self::Any{args} => { let a: Vec<_> = args.iter().map(|a|a.evaluate(context,candidate)).collect(); if a.contains(&True) {True} else if a.contains(&Unknown) {Unknown} else {False} },
            Self::Not{arg} => match arg.evaluate(context,candidate) { True=>False,False=>True,Unknown=>Unknown },
            Self::Exists{field} => if lookup(field,context,candidate).is_some() {True} else {Unknown},
            Self::In{field,values} => match lookup(field,context,candidate) { Some((v,_))=>if values.contains(v) {True} else {False},None=>Unknown },
            Self::Eq{field,value,unit} => compare(field,value,unit,context,candidate,|o|o==std::cmp::Ordering::Equal,true),
            Self::Lt{field,value,unit} => compare(field,value,unit,context,candidate,|o|o.is_lt(),false),
            Self::Lte{field,value,unit} => compare(field,value,unit,context,candidate,|o|!o.is_gt(),false),
            Self::Gt{field,value,unit} => compare(field,value,unit,context,candidate,|o|o.is_gt(),false),
            Self::Gte{field,value,unit} => compare(field,value,unit,context,candidate,|o|!o.is_lt(),false),
        }
    }
}
fn valid_field(field: &str) -> Result<()> {
    let (_,key) = field.split_once('.').ok_or_else(||Error::new("INPUT_INVALID"))?;
    ensure((field.starts_with("context.") || field.starts_with("candidate.")) && valid_key(key), "INPUT_INVALID")
}
fn lookup<'a>(field: &str, context: &'a Context, candidate: &'a Candidate) -> Option<(&'a Value, Option<&'a str>)> {
    if let Some(key)=field.strip_prefix("candidate.") { return candidate.attributes.get(key).filter(|v| !v.is_null()).map(|v|(v,None)); }
    let key=field.strip_prefix("context.")?;
    if context.unknown_fields.iter().any(|s|s==key) { return None; }
    context.facts.iter().find(|f| f.key==key && f.evidence_status=="explicit" && !f.value.is_null()).map(|f|(&f.value,f.unit.as_deref()))
}
fn compare(field: &str, expected: &Value, unit: &Option<String>, context: &Context, candidate: &Candidate, predicate: impl Fn(std::cmp::Ordering)->bool, equality: bool) -> Truth {
    let Some((actual,actual_unit))=lookup(field,context,candidate) else { return Truth::Unknown; };
    if actual_unit != unit.as_deref() { return Truth::Unknown; }
    let ordering = if let (Some(a),Some(b))=(actual.as_f64(),expected.as_f64()) { a.partial_cmp(&b) }
    else if actual.is_object() && expected.is_object() {
        if actual.get("currency")!=expected.get("currency") { return Truth::Unknown; }
        actual.get("amount_minor").and_then(Value::as_i64).zip(expected.get("amount_minor").and_then(Value::as_i64)).map(|(a,b)|a.cmp(&b))
    } else if equality { Some(if actual==expected {std::cmp::Ordering::Equal} else {std::cmp::Ordering::Less}) }
    else { None };
    match ordering {Some(o) if predicate(o)=>Truth::True,Some(_)=>Truth::False,None=>Truth::Unknown}
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyProposal {
    pub title: String,
    pub domain: String,
    pub effect: String,
    pub condition: Expr,
    pub source_case_ids: Vec<String>,
    pub effective_from: String,
    pub effective_until: Option<String>,
}
impl PolicyProposal {
    pub fn validate(&self) -> Result<()> {
        ensure(!self.title.trim().is_empty() && self.title.len()<=2048 && (DOMAINS.contains(&self.domain.as_str()) || self.domain=="all"),"INPUT_INVALID")?;
        ensure(["forbid","require","prefer","require_review"].contains(&self.effect.as_str()),"INPUT_INVALID")?;
        ensure(valid_time(&self.effective_from) && self.effective_until.as_ref().is_none_or(|s|valid_time(s)),"INPUT_INVALID")?;
        if let Some(end)=&self.effective_until { ensure(chrono::DateTime::parse_from_rfc3339(end).ok()>chrono::DateTime::parse_from_rfc3339(&self.effective_from).ok(),"INPUT_INVALID")?; }
        ensure(self.source_case_ids.len()<=20 && self.source_case_ids.iter().all(|s|valid_uuid(s)),"INPUT_INVALID")?;
        self.condition.validate(0)?;
        ensure(serde_json::to_vec(&self.condition)?.len()<=16384,"POLICY_TOO_COMPLEX")
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRecord { pub id: String, pub workspace_id: String, pub revision: i64, pub status: String, pub proposal: PolicyProposal }
#[derive(Debug, Clone, Serialize, Default)]
pub struct PolicyCheck {
    pub excluded: Vec<Value>, pub matched: Vec<Value>, pub missing: Vec<Value>,
    pub conflict: bool, pub require_review: bool, pub preferred: Vec<String>,
}
pub fn check(policies: &[PolicyRecord], request: &RankRequest) -> PolicyCheck {
    let mut result=PolicyCheck::default();
    let all: BTreeSet<_>=request.candidates.iter().map(|c|c.id.clone()).collect();
    let mut allowed=all.clone();
    let mut has_constraint=false;
    for p in policies.iter().filter(|p|p.status=="effective" && (p.proposal.domain=="all" || p.proposal.domain==request.domain)) {
        let start=chrono::DateTime::parse_from_rfc3339(&p.proposal.effective_from).ok();
        let as_of=chrono::DateTime::parse_from_rfc3339(&request.context.as_of).ok();
        if start>as_of { continue; }
        if let Some(end)=p.proposal.effective_until.as_ref().and_then(|s|chrono::DateTime::parse_from_rfc3339(s).ok()) { if as_of.is_some_and(|t|t>=end) {continue;} }
        let mut required=BTreeSet::new();
        let mut unknown=false;
        for c in &request.candidates {
            match p.proposal.condition.evaluate(&request.context,c) {
                Truth::Unknown=>{
                    unknown=true;
                    if p.proposal.effect!="prefer" {result.missing.push(json!({"policy_id":p.id,"candidate_id":c.id,"reason":"INSUFFICIENT_CONTEXT"}));}
                },
                Truth::False=>(),
                Truth::True=>{
                    result.matched.push(json!({"policy_id":p.id,"revision":p.revision,"candidate_id":c.id,"effect":p.proposal.effect}));
                    match p.proposal.effect.as_str() {
                        "forbid"=>{allowed.remove(&c.id);has_constraint=true;result.excluded.push(json!({"candidate_id":c.id,"policy_id":p.id,"reason":"EXPLICIT_PROHIBITION"}));},
                        "require"=>{required.insert(c.id.clone());},
                        "prefer"=>result.preferred.push(c.id.clone()),
                        "require_review"=>result.require_review=true,
                        _=>(),
                    }
                }
            }
        }
        if p.proposal.effect=="require" && !unknown {
            has_constraint=true;
            for c in allowed.difference(&required) {result.excluded.push(json!({"candidate_id":c,"policy_id":p.id,"reason":"REQUIRED_ALTERNATIVE"}));}
            allowed=allowed.intersection(&required).cloned().collect();
        }
    }
    result.conflict=has_constraint && allowed.is_empty() && result.missing.is_empty();
    result
}
