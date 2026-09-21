use crate::{domain::*, store::Store};
use rusqlite::params;
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
use std::collections::{BTreeMap,BTreeSet};

#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticCell {pub domain:String,pub confirmed_count_bucket:u32,pub missing_count_bucket:u32,pub independent_evaluation_count_bucket:u32}
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticReport {
    pub schema_version:String,pub app_build:String,pub os_family:String,pub cpu_class:String,
    pub period_week:String,pub cells:Vec<DiagnosticCell>,pub errors:BTreeMap<String,u32>,
}
impl DiagnosticReport {
    pub fn validate(&self)->Result<()> {
        ensure(self.schema_version=="1.0" && ["0.1.0"].contains(&self.app_build.as_str()),"INPUT_INVALID")?;
        ensure(["macos","linux","windows"].contains(&self.os_family.as_str()) && ["apple_silicon","x86_64","other"].contains(&self.cpu_class.as_str()),"INPUT_INVALID")?;
        ensure(self.period_week.len()==8 && self.period_week.as_bytes()[4]==b'-' && self.period_week.as_bytes()[5]==b'W' && self.period_week[0..4].bytes().all(|b|b.is_ascii_digit()) && self.period_week[6..].parse::<u32>().is_ok_and(|n|(1..=53).contains(&n)),"INPUT_INVALID")?;
        ensure(self.cells.len()<=6 && self.errors.len()<=8,"INPUT_INVALID")?;
        let mut seen=BTreeSet::new();
        for c in &self.cells {
            ensure(DOMAINS.contains(&c.domain.as_str()) && seen.insert(&c.domain),"INPUT_INVALID")?;
            for n in [c.confirmed_count_bucket,c.missing_count_bucket,c.independent_evaluation_count_bucket] {ensure(n==0 || n>=20 && n%20==0 && n<=100000,"SMALL_CELL_UNSAFE")?;}
            ensure(c.confirmed_count_bucket>=20,"SMALL_CELL_UNSAFE")?;
        }
        for(k,v)in &self.errors {ensure(["IMPORT_REJECTED","TRAINING_FAILED","WORKER_INTERRUPTED","MODEL_INVALIDATED","CONTRACT_REJECTED","PROVIDER_UNAVAILABLE","BUDGET_EXCEEDED","CANCELLED"].contains(&k.as_str()) && *v%20==0,"INPUT_INVALID")?;}
        Ok(())
    }
}
pub fn preview(store:&Store,w:&str)->Result<DiagnosticReport>{
    store.workspace(w)?;
    let mut counts:BTreeMap<String,(u32,u32)>=BTreeMap::new();
    for c in store.cases(w,100000,0)? {
        if c.verification_state!="confirmed"{continue;}
        let v=counts.entry(c.domain).or_default();v.0+=1;
        if !c.context.unknown_fields.is_empty(){v.1+=1;}
    }
    let cells=counts.into_iter().filter(|(_,v)|v.0>=20).map(|(domain,(n,m))|DiagnosticCell{domain,confirmed_count_bucket:(n/20)*20,missing_count_bucket:if m<20{0}else{(m/20)*20},independent_evaluation_count_bucket:0}).collect();
    let r=DiagnosticReport{schema_version:"1.0".into(),app_build:"0.1.0".into(),os_family:if cfg!(target_os="macos"){"macos"}else if cfg!(windows){"windows"}else{"linux"}.into(),cpu_class:if cfg!(all(target_os="macos",target_arch="aarch64")){"apple_silicon"}else if cfg!(target_arch="x86_64"){"x86_64"}else{"other"}.into(),period_week:chrono::Utc::now().format("%G-W%V").to_string(),cells,errors:BTreeMap::new()};
    r.validate()?;Ok(r)
}
pub fn validate_assessment(v:&Value)->Result<()> {
    let o=v.as_object().ok_or_else(||Error::new("INPUT_INVALID"))?;
    ensure(o.keys().all(|k|["instrument","instrument_version","language","assessed_at","source_kind","scales","note"].contains(&k.as_str())),"INPUT_INVALID")?;
    ensure(v["instrument"].as_str().is_some_and(|s|["BFI-2-J","IPIP","TIPI-J","HEXACO","self_report"].contains(&s)),"INPUT_INVALID")?;
    ensure(v["language"].as_str().is_some_and(|s|s=="ja"||s=="en") && v["assessed_at"].as_str().is_some_and(valid_time),"INPUT_INVALID")?;
    ensure(v["source_kind"].as_str().is_some_and(|s|s=="provided_result"||s=="self_report") && v["instrument_version"].as_str().is_some_and(|s|!s.is_empty()&&s.len()<=64),"INPUT_INVALID")?;
    let scales=v["scales"].as_array().ok_or_else(||Error::new("INPUT_INVALID"))?;ensure(!scales.is_empty()&&scales.len()<=30,"INPUT_INVALID")?;
    let mut ids=BTreeSet::new();
    for s in scales {
        let o=s.as_object().ok_or_else(||Error::new("INPUT_INVALID"))?;ensure(o.keys().all(|k|["key","score","minimum","maximum"].contains(&k.as_str())) && o.len()==4,"INPUT_INVALID")?;
        let key=s["key"].as_str().ok_or_else(||Error::new("INPUT_INVALID"))?;ensure(valid_key(key)&&ids.insert(key),"INPUT_INVALID")?;
        let score=s["score"].as_f64().ok_or_else(||Error::new("INPUT_INVALID"))?;
        let min=s["minimum"].as_f64().ok_or_else(||Error::new("INPUT_INVALID"))?;let max=s["maximum"].as_f64().ok_or_else(||Error::new("INPUT_INVALID"))?;
        ensure(score.is_finite()&&min.is_finite()&&max.is_finite()&&min<max&&score>=min&&score<=max,"INPUT_INVALID")?;
    }
    ensure(v.get("note").is_none_or(|x|x.as_str().is_some_and(|s|s.len()<=8192)),"INPUT_INVALID")?;Ok(())
}
pub fn improvement_proposals(store:&mut Store,w:&str)->Result<Vec<Value>> {
    let ws=store.workspace(w)?;
    let cases=store.cases(w,100000,0)?;
    let mut proposals=vec![];
    for domain in DOMAINS {
        let selected:Vec<_>=cases.iter().filter(|c|c.domain==domain).collect();
        let confirmed=selected.iter().filter(|c|c.verification_state=="confirmed").count();
        let mut missing=BTreeMap::<String,usize>::new();
        for c in &selected {for f in &c.context.unknown_fields{*missing.entry(f.clone()).or_default()+=1;}}
        if confirmed<10 || missing.values().any(|v|*v>=3) {
            let proposal=json!({"id":id(),"workspace_id":w,"deletion_epoch":ws.deletion_epoch,"state":"proposed","kind":"interview_gap","domain":domain,
                "problem":"条件と回答の確認が不足しています。","evidence":{"confirmed_count":confirmed,"missing_counts":missing},
                "hypothesis":"判断が逆転する条件を限定して聞くと、少ない回答で境界を確認できる可能性があります。",
                "change":{"type":"prioritize_domain","domain":domain,"questions":3},"expected_direction":"less_answering_burden","measured_gain":null,
                "evaluation_plan":"別familyの未提示事例で、回答前の予測と本人回答を比較する。","permissions":["question:propose"],"privacy":"local_only","rollback":"質問配分を既定値へ戻す。","activation_requires_separate_approval":true});
            store.conn.execute("INSERT INTO improvements VALUES(?1,?2,?3,'proposed',?4,?5)",params![proposal["id"].as_str(),w,ws.deletion_epoch,proposal.to_string(),now()])?;
            proposals.push(proposal);
        }
    }
    Ok(proposals)
}
