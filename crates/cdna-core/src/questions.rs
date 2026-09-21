use crate::{domain::*,store::Store,features};
use rand::{Rng,seq::SliceRandom};
use serde::{Serialize,Deserialize};
use serde_json::{Value,json};
use std::collections::BTreeMap;

#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct Template {pub template_id:String,pub domain:String,pub title:String,pub summary:String,pub synthetic_scenario:bool,pub candidates:Vec<Candidate>,pub reversal_keys:Vec<String>,pub purpose:String}
pub fn templates()->Result<Vec<Template>> {Ok(serde_json::from_str(include_str!("../../../contracts/question-templates.json"))?)}
pub fn gaps(store:&Store,w:&str,domains:&[String])->Result<Value>{
    let cases=store.cases(w,100000,0)?;
    let model=store.active_model(w)?;
    let mut result=vec![];
    for domain in DOMAINS.iter().filter(|d|domains.iter().any(|x|x==**d)) {
        let selected:Vec<_>=cases.iter().filter(|c|c.domain==*domain).collect();
        let mut missing:BTreeMap<String,u64>=BTreeMap::new();
        for c in &selected {for k in &c.context.unknown_fields{*missing.entry(k.clone()).or_default()+=1;}}
        let confirmed=selected.iter().filter(|c|c.verification_state=="confirmed").count();
        let usable=selected.iter().filter(|c|c.verification_state=="confirmed"&&c.response.as_ref().is_some_and(Response::learnable)).count();
        let pending=selected.iter().filter(|c|c.verification_state=="pending").count();
        let has_model=model.as_ref().is_some_and(|m|m["domain_support"][*domain].as_u64().unwrap_or(0)>0);
        result.push(json!({"domain":domain,"confirmed_count":confirmed,"learnable_count":usable,"pending_count":pending,"missing_fields":missing,"model_available":has_model,"independent_agreement":null,"calibration_status":"unverified","status":if confirmed<5{"thin"}else if !has_model{"awaiting_training"}else{"needs_independent_evaluation"}}));
    }
    Ok(json!({"schema_version":"1.0","workspace_id":w,"domains":result,"counts_are_not_accuracy":true}))
}
fn context_for(t:&Template,seed:u64)->Context {
    let high=if seed%3==0{"high"}else if seed%3==1{"medium"}else{"low"};
    let deadline=[3,14,45,90][(seed as usize/3)%4];
    let resource=[0.2,0.5,0.9][(seed as usize/5)%3];
    let horizon=[3,12,36][(seed as usize/7)%3];
    let fact=|key:&str,value:Value,unit:Option<&str>|Fact{key:key.into(),value,evidence_status:"explicit".into(),unit:unit.map(str::to_owned)};
    Context{as_of:now(),summary:format!("【架空の判断練習】{}\n期限は{}日後。既存顧客への影響は{}、資源の余裕は{}です。{}か月先までを考えて判断してください。",t.summary,deadline,match high{"high"=>"大きい","medium"=>"中程度",_=>"小さい"},if resource>0.7{"少ない"}else{"ある"},horizon),facts:vec![
        fact("deadline_days",json!(deadline),Some("day")),fact("customer_impact",json!(high),None),fact("resource_pressure",json!(resource),None),fact("reversibility",json!(if seed%2==0{"high"}else{"low"}),None),fact("horizon_months",json!(horizon),Some("month")),fact("evidence_confidence",json!([0.3,0.7,1.0][(seed as usize/11)%3]),None),fact("available_people",json!([2,4,8][(seed as usize/13)%3]),Some("person"))],unknown_fields:vec![]}
}
pub fn next(store:&mut Store,w:&str,domain:Option<&str>)->Result<Value>{
    let ts=templates()?;let mut rng=rand::thread_rng();
    if let Some(d)=domain{ensure(DOMAINS.contains(&d),"UNSUPPORTED_DOMAIN")?;}
    let cases=store.cases(w,100000,0)?;
    let mut counts:BTreeMap<&str,usize>=DOMAINS.iter().map(|d|(*d,0)).collect();
    for c in &cases{if c.verification_state=="confirmed"{if let Some(v)=counts.get_mut(c.domain.as_str()){*v+=1;}}}
    let draw:f64=rng.r#gen();
    let branch=if draw<0.5{"active"}else if draw<0.8{"representative"}else{"diversity"};
    let chosen_domain=domain.map(str::to_owned).unwrap_or_else(||{
        if branch=="active"{counts.iter().min_by_key(|(_,n)|**n).map(|(d,_)|d.to_string()).unwrap_or_else(||DOMAINS[0].into())}
        else if branch=="representative"{
            let weights:Vec<_>=DOMAINS.iter().map(|d|(*d,counts[d]+1)).collect();
            let mut n=rng.gen_range(0..weights.iter().map(|(_,n)|n).sum::<usize>());
            for(d,weight)in weights{if n<weight{return d.into();}n-=weight;}
            DOMAINS[0].into()
        }else{DOMAINS[rng.gen_range(0..DOMAINS.len())].into()}
    });
    let recent:Vec<_>=cases.iter().take(3).map(|c|c.source.reference.as_str()).collect();
    let candidates:Vec<_>=ts.iter().filter(|t|t.domain==chosen_domain&&!recent.iter().any(|r|r.starts_with(&format!("template:{}:",t.template_id)))).collect();
    let pool=if candidates.is_empty(){ts.iter().filter(|t|t.domain==chosen_domain).collect::<Vec<_>>()}else{candidates};
    let t=pool.choose(&mut rng).ok_or_else(||Error::new("NO_QUESTION_AVAILABLE"))?;
    let seed:u64=rng.gen_range(0..1_000_000);
    let p=ObservationProposal{schema_version:SCHEMA.into(),request_id:id(),workspace_id:w.into(),case_kind:"hypothetical".into(),family_id:id(),domain:t.domain.clone(),context:context_for(t,seed),candidates:t.candidates.clone(),response:None,source:Source{kind:"manual".into(),reference:format!("template:{}:{seed}",t.template_id),quote:String::new()},model_exposure:false};
    let case=store.propose(&p,"generated_question","owner")?;let exposure=store.expose(w,&case.id,&format!("mixture_v1:{branch}:seed={seed}"))?;
    Ok(json!({"case":case,"exposure":exposure,"template_id":t.template_id,"title":t.title,"purpose":t.purpose,"synthetic_scenario":true,"question_selection_probability":null,"query_policy":{"id":"mixture_v1","active":0.5,"representative":0.3,"diversity":0.2,"branch":branch}}))
}
pub fn reversal(store:&mut Store,w:&str,case_id:&str,key:&str,value:Value)->Result<Value>{
    let original=store.case(w,case_id)?;
    ensure(features::manifest().context_features.iter().any(|f|f.key==key),"INPUT_INVALID")?;
    let mut context=original.context.clone();
    let f=context.facts.iter_mut().find(|f|f.key==key).ok_or_else(||Error::new("INPUT_INVALID"))?;
    ensure(f.value!=value,"NO_CHANGE")?;f.value=value.clone();f.evidence_status="explicit".into();
    context.unknown_fields.retain(|k|k!=key);context.as_of=now();
    context.summary=format!("【条件を一つ変更する練習】\n元の状況: {}\n変更点: {key} を {value} に変更します。他の条件は同じです。",original.context.summary);
    context.validate()?;features::context_values(&context)?;
    let p=ObservationProposal{schema_version:SCHEMA.into(),request_id:id(),workspace_id:w.into(),case_kind:"hypothetical".into(),family_id:original.family_id.clone(),domain:original.domain,context,candidates:original.candidates,response:None,source:Source{kind:"manual".into(),reference:format!("reversal:{case_id}"),quote:String::new()},model_exposure:false};
    let case=store.propose(&p,"generated_question","owner")?;let exposure=store.expose(w,&case.id,"reversal_v1")?;
    Ok(json!({"case":case,"exposure":exposure,"changed_keys":[key],"same_family_as":case_id,"synthetic_scenario":true}))
}
pub fn seed_demo(store:&mut Store)->Result<Workspace>{
    let ws=store.create_workspace("架空の創業者 · デモ",true)?;
    let ts=templates()?;let mut weights=vec![0.0;features::dimension()];let names=features::names();
    for (name,weight) in [("candidate:quality",0.7),("candidate:cost_efficiency",0.6),("candidate:speed",1.2),("context:deadline_days*knowledge_gain",2.5),("context:deadline_days*speed",-2.0),("context:customer_impact*customer_protection",2.0),("context:resource_pressure*cost_efficiency",1.5)] {
        if let Some(i)=names.iter().position(|n|n==name){weights[i]=weight;}
    }
    for i in 0..72 {
        let t=&ts[i%ts.len()];let seed=(i*37+11) as u64;let mut context=context_for(t,seed);
        // Dates, labels and situations are synthetic. No real conversation, person,
        // company, financial figure or personality assessment is included.
        context.as_of=(chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z").map_err(|_|Error::new("CONTRACT_ERROR"))?+chrono::Duration::days(i as i64)).to_rfc3339();
        let scores=features::score_rows(&weights,&features::transform(&t.domain,&context,&t.candidates)?)?;
        let best=(0..scores.len()).max_by(|&a,&b|scores[a].total_cmp(&scores[b])).ok_or_else(||Error::new("CONTRACT_ERROR"))?;
        let p=ObservationProposal{schema_version:SCHEMA.into(),request_id:id(),workspace_id:ws.id.clone(),case_kind:"hypothetical".into(),family_id:id(),domain:t.domain.clone(),context,candidates:t.candidates.clone(),response:Some(Response{kind:"choose_one".into(),selected_ids:vec![t.candidates[best].id.clone()],value:None,note:"合成データ。実在する経営者の回答ではありません。".into(),reversal_condition:String::new()}),source:Source{kind:"synthetic_seed".into(),reference:format!("synthetic-v1:{i}"),quote:String::new()},model_exposure:false};
        let c=store.propose(&p,"synthetic_seed","owner")?;store.confirm(&ws.id,&c.id,c.revision)?;
    }
    Ok(ws)
}
