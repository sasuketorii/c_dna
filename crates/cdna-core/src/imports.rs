//! Export adapters do not infer that an assistant suggestion was a human decision.
//! All imports remain pending. Archives are inspected before any entry is read.
use crate::{domain::*,store::Store};
use rusqlite::{params,OptionalExtension};
use serde::{Serialize,Deserialize};
use serde_json::{Value,json};
use sha2::{Digest,Sha256};
use std::{collections::BTreeSet,io::{Read,Seek,Cursor},path::Path};

pub const MAX_COMPRESSED:u64=1024*1024*1024;
pub const MAX_EXPANDED:u64=4*1024*1024*1024;
pub const MAX_MESSAGE:usize=2*1024*1024;
pub const MAX_MESSAGES:usize=100000;
// Parsing memory is bounded independently of the archive's aggregate limit.
pub const MAX_DOCUMENT:usize=128*1024*1024;
#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct Message {pub external_id:String,pub role:String,pub text:String,pub occurred_at:Option<String>}
#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct ParsedImport {pub provider:String,pub format_version:String,pub content_hash:String,pub messages:Vec<Message>,pub warnings:Vec<String>}
fn content(parts:&Value)->String {
    if let Some(s)=parts.as_str(){return s.into();}
    parts.as_array().map(|a|a.iter().filter_map(|x|x.as_str().or_else(||x.get("text").and_then(Value::as_str))).collect::<Vec<_>>().join("\n")).unwrap_or_default()
}
fn timestamp(v:Option<&Value>)->Option<String>{
    let v=v?;
    if let Some(s)=v.as_str(){return valid_time(s).then(||s.into());}
    let secs=v.as_f64()?;if !secs.is_finite(){return None;}
    chrono::DateTime::from_timestamp(secs as i64,0).map(|t|t.to_rfc3339())
}
fn checked_message(external_id:String,role:&str,text:String,time:Option<String>)->Result<Message>{
    ensure(text.len()<=MAX_MESSAGE && external_id.len()<=1024,"MESSAGE_TOO_LARGE")?;
    Ok(Message{external_id,role:match role{"user"|"human"=>"human","assistant"=>"assistant","system"|"developer"=>"system",_=>"unknown"}.into(),text,occurred_at:time})
}
fn conversation(v:&Value,index:usize,out:&mut Vec<Message>,provider:&mut Option<String>)->Result<()> {
    if let Some(mapping)=v.get("mapping").and_then(Value::as_object) {
        if provider.as_deref().is_some_and(|p|p!="chatgpt_export"){return Err(Error::new("MIXED_FORMAT"));}
        *provider=Some("chatgpt_export".into());
        let conversation_id=v.get("id").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(||format!("conversation_{index}"));
        for (node_id,node) in mapping {
            let Some(m)=node.get("message").filter(|m|m.is_object()) else{continue;};
            let role=m.pointer("/author/role").and_then(Value::as_str).unwrap_or("unknown");
            let text=m.pointer("/content/parts").map(content).unwrap_or_default();
            if text.trim().is_empty(){continue;}
            out.push(checked_message(format!("{conversation_id}:{node_id}"),role,text,timestamp(m.get("create_time")))?);
            ensure(out.len()<=MAX_MESSAGES,"TOO_MANY_MESSAGES")?;
        }
    } else if let Some(messages)=v.get("chat_messages").and_then(Value::as_array) {
        if provider.as_deref().is_some_and(|p|p!="claude_export"){return Err(Error::new("MIXED_FORMAT"));}
        *provider=Some("claude_export".into());
        let conversation_id=v.get("uuid").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(||format!("conversation_{index}"));
        for (i,m) in messages.iter().enumerate(){
            let role=m.get("sender").and_then(Value::as_str).unwrap_or("unknown");
            let text=m.get("text").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(||m.get("content").map(content).unwrap_or_default());
            if text.trim().is_empty(){continue;}
            let mid=m.get("uuid").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(||format!("message_{i}"));
            out.push(checked_message(format!("{conversation_id}:{mid}"),role,text,timestamp(m.get("created_at")))?);
            ensure(out.len()<=MAX_MESSAGES,"TOO_MANY_MESSAGES")?;
        }
    }else{return Err(Error::new("UNSUPPORTED_EXPORT_FORMAT"));}
    Ok(())
}
pub fn parse_json(raw:&[u8])->Result<ParsedImport>{
    ensure(raw.len()<=MAX_DOCUMENT,"DOCUMENT_TOO_LARGE")?;
    let doc:Value=crate::transport::strict_json(raw)?;
    let mut messages=vec![];let mut provider=None;
    match &doc {
        Value::Array(a)=>{for(i,v)in a.iter().enumerate(){conversation(v,i,&mut messages,&mut provider)?;}},
        Value::Object(_)=>{
            if let Some(a)=doc.get("conversations").and_then(Value::as_array){for(i,v)in a.iter().enumerate(){conversation(v,i,&mut messages,&mut provider)?;}}
            else{conversation(&doc,0,&mut messages,&mut provider)?;}
        },_=>return Err(Error::new("UNSUPPORTED_EXPORT_FORMAT")),
    }
    ensure(!messages.is_empty(),"EMPTY_IMPORT")?;
    Ok(ParsedImport{provider:provider.unwrap_or_default(),format_version:"supported_export_v1".into(),content_hash:hex::encode(Sha256::digest(raw)),messages,warnings:vec!["AIの発言は出典として保存しますが、本人の選択として学習しません。".into(),"発言だけでは候補の比較を確定せず、確認待ちの記録として取り込みます。".into()]})
}
pub fn parse_summary(raw:&[u8])->Result<ParsedImport>{
    ensure(raw.len()<=MAX_MESSAGE,"MESSAGE_TOO_LARGE")?;
    let text=String::from_utf8(raw.to_vec()).map_err(|_|Error::new("INPUT_INVALID"))?;
    ensure(!text.trim().is_empty(),"EMPTY_IMPORT")?;
    Ok(ParsedImport{provider:"summary".into(),format_version:"summary_v1".into(),content_hash:hex::encode(Sha256::digest(raw)),messages:vec![Message{external_id:"summary".into(),role:"unknown".into(),text,occurred_at:None}],warnings:vec!["要約は初期仮説です。承認しても、実際に行動した記録や比較学習の正解には変換しません。".into()]})
}
pub fn parse_html(raw:&[u8])->Result<ParsedImport>{
    ensure(raw.len()<=MAX_DOCUMENT,"DOCUMENT_TOO_LARGE")?;
    let text=std::str::from_utf8(raw).map_err(|_|Error::new("INPUT_INVALID"))?;
    // A real HTML parser, not a WebView: no scripts, images, URLs or event handlers run.
    let doc=scraper::Html::parse_document(text);
    let messages_sel=scraper::Selector::parse(".message").map_err(|_|Error::new("CONTRACT_ERROR"))?;
    let author_sel=scraper::Selector::parse("h4, .author").map_err(|_|Error::new("CONTRACT_ERROR"))?;
    let content_sel=scraper::Selector::parse(".content, .text").map_err(|_|Error::new("CONTRACT_ERROR"))?;
    let mut messages=vec![];
    for (i,node) in doc.select(&messages_sel).enumerate(){
        let role=node.select(&author_sel).next().map(|n|n.text().collect::<String>().to_lowercase()).unwrap_or_default();
        let role=if role.contains("user")||role.contains("you"){"human"}else if role.contains("assistant")||role.contains("chatgpt"){"assistant"}else{"unknown"};
        let body=node.select(&content_sel).next();
        let Some(body)=body else{continue;};
        let plain=body.descendants().filter_map(|n|{
            if n.ancestors().any(|a|a.value().as_element().is_some_and(|e|["script","style","iframe","object"].contains(&e.name()))){return None;}
            n.value().as_text().map(|t|t.to_string())
        }).collect::<Vec<_>>().join("");
        if !plain.trim().is_empty(){messages.push(checked_message(format!("html_{i}"),role,plain,None)?);}
        ensure(messages.len()<=MAX_MESSAGES,"TOO_MANY_MESSAGES")?;
    }
    ensure(!messages.is_empty(),"UNSUPPORTED_EXPORT_FORMAT")?;
    Ok(ParsedImport{provider:"chatgpt_export".into(),format_version:"chatgpt_html_message_v1".into(),content_hash:hex::encode(Sha256::digest(raw)),messages,warnings:vec!["HTMLは不活性なテキストとして読み込みました。外部リソースへ接続していません。".into()]})
}
pub fn parse_archive<R:Read+Seek>(reader:R,compressed_size:u64)->Result<ParsedImport>{
    ensure(compressed_size<=MAX_COMPRESSED,"ARCHIVE_TOO_LARGE")?;
    let mut archive=zip::ZipArchive::new(reader).map_err(|_|Error::new("INVALID_ARCHIVE"))?;
    ensure(archive.len()<=10000,"INVALID_ARCHIVE")?;
    let mut total=0u64;let mut names=BTreeSet::new();let mut selected=None;
    for i in 0..archive.len(){
        let entry=archive.by_index(i).map_err(|_|Error::new("INVALID_ARCHIVE"))?;
        let name=entry.name();
        ensure(!name.contains('\\') && !name.contains('\0') && !name.starts_with('/') && !name.split('/').any(|p|p=="..") && entry.enclosed_name().is_some(),"UNSAFE_ARCHIVE_PATH")?;
        ensure(names.insert(name.to_string()),"DUPLICATE_ARCHIVE_ENTRY")?;
        if let Some(mode)=entry.unix_mode(){ensure(mode&0o170000!=0o120000,"UNSAFE_ARCHIVE_LINK")?;}
        ensure(!name.to_ascii_lowercase().ends_with(".zip"),"NESTED_ARCHIVE_UNSUPPORTED")?;
        total=total.checked_add(entry.size()).ok_or_else(||Error::new("ARCHIVE_TOO_LARGE"))?;
        ensure(total<=MAX_EXPANDED,"ARCHIVE_TOO_LARGE")?;
        if entry.size()>1024*1024{ensure(entry.size()/entry.compressed_size().max(1)<=250,"ARCHIVE_COMPRESSION_BOMB")?;}
        let base=Path::new(name).file_name().and_then(|s|s.to_str()).unwrap_or("");
        if ["conversations.json","claude_conversations.json"].contains(&base){
            ensure(selected.is_none(),"AMBIGUOUS_ARCHIVE")?;selected=Some(i);
        }
    }
    let i=selected.ok_or_else(||Error::new("UNSUPPORTED_EXPORT_FORMAT"))?;
    let entry=archive.by_index(i).map_err(|_|Error::new("INVALID_ARCHIVE"))?;
    ensure(entry.size()<=MAX_DOCUMENT as u64,"DOCUMENT_TOO_LARGE")?;
    let mut data=Vec::new();entry.take(MAX_DOCUMENT as u64+1).read_to_end(&mut data)?;
    ensure(data.len()<=MAX_DOCUMENT,"DOCUMENT_TOO_LARGE")?;parse_json(&data)
}
pub fn parse(raw:&[u8],format:&str)->Result<ParsedImport>{
    match format{"json"=>parse_json(raw),"summary"|"txt"|"md"=>parse_summary(raw),"html"=>parse_html(raw),"zip"=>parse_archive(Cursor::new(raw),raw.len() as u64),_=>Err(Error::new("UNSUPPORTED_EXPORT_FORMAT"))}
}
pub fn ingest(store:&mut Store,w:&str,parsed:ParsedImport)->Result<Value>{
    store.workspace(w)?;
    let old:Option<String>=store.conn.query_row("SELECT id FROM imports WHERE workspace_id=?1 AND provider=?2 AND content_hash=?3",params![w,parsed.provider,parsed.content_hash],|r|r.get(0)).optional()?;
    if let Some(id)=old{return Ok(json!({"artifact_id":id,"duplicate":true,"new_cases":0}));}
    let artifact_id=id();let mut pending=vec![];let ts=now();
    let tx=store.conn.transaction()?;
    tx.execute("INSERT INTO imports VALUES(?1,?2,?3,?4,?5)",params![artifact_id,w,parsed.provider,parsed.content_hash,ts])?;
    let mut seen=BTreeSet::new();
    for m in &parsed.messages {
        if !seen.insert(&m.external_id){continue;}
        let mid=id();
        tx.execute("INSERT INTO source_messages VALUES(?1,?2,?3,?4,?5,?6,?7)",params![mid,w,artifact_id,m.external_id,m.role,m.text,m.occurred_at])?;
        if m.role=="human" || parsed.provider=="summary" {pending.push((mid,m.clone()));}
    }
    tx.commit()?;
    let mut created=vec![];
    for (mid,m) in pending {
        let p=ObservationProposal{schema_version:SCHEMA.into(),request_id:id(),workspace_id:w.into(),case_kind:"hypothetical".into(),family_id:artifact_id.clone(),domain:"resource_allocation".into(),
            context:Context{as_of:m.occurred_at.unwrap_or_else(||ts.clone()),summary:m.text.chars().take(8192).collect(),facts:vec![],unknown_fields:crate::features::manifest().context_features.iter().map(|c|c.key.clone()).collect()},
            candidates:vec![],response:None,source:Source{kind:parsed.provider.clone(),reference:format!("cdna-source:{mid}"),quote:m.text.chars().take(8192).collect()},model_exposure:false};
        created.push(store.propose(&p,"imported_statement","owner")?.id);
    }
    Ok(json!({"artifact_id":artifact_id,"duplicate":false,"message_count":parsed.messages.len(),"new_cases":created.len(),"case_ids":created,"warnings":parsed.warnings,"all_observations_pending":true}))
}
