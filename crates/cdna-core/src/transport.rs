//! Strict JSON and authenticated local IPC. No HTTP listener in the product.
use crate::{domain::*,Runtime};
use serde::{Deserialize,Deserializer,de::{self,Visitor,MapAccess,SeqAccess}};
use serde_json::{Value,Map,Number};
use std::{fmt,path::Path,sync::Arc,time::Duration};
use tokio::io::{AsyncReadExt,AsyncWriteExt};

struct StrictValue(Value);
impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D:Deserializer<'de>>(deserializer:D)->std::result::Result<Self,D::Error>{
        struct JsonVisitor;
        impl<'de> Visitor<'de> for JsonVisitor {
            type Value=StrictValue;
            fn expecting(&self,f:&mut fmt::Formatter)->fmt::Result{f.write_str("a JSON value without duplicate keys")}
            fn visit_bool<E:de::Error>(self,v:bool)->std::result::Result<Self::Value,E>{Ok(StrictValue(Value::Bool(v)))}
            fn visit_i64<E:de::Error>(self,v:i64)->std::result::Result<Self::Value,E>{Ok(StrictValue(Value::Number(v.into())))}
            fn visit_u64<E:de::Error>(self,v:u64)->std::result::Result<Self::Value,E>{Ok(StrictValue(Value::Number(v.into())))}
            fn visit_f64<E:de::Error>(self,v:f64)->std::result::Result<Self::Value,E>{Number::from_f64(v).map(|n|StrictValue(Value::Number(n))).ok_or_else(||E::custom("nonfinite"))}
            fn visit_str<E:de::Error>(self,v:&str)->std::result::Result<Self::Value,E>{Ok(StrictValue(Value::String(v.into())))}
            fn visit_string<E:de::Error>(self,v:String)->std::result::Result<Self::Value,E>{Ok(StrictValue(Value::String(v)))}
            fn visit_none<E:de::Error>(self)->std::result::Result<Self::Value,E>{Ok(StrictValue(Value::Null))}
            fn visit_unit<E:de::Error>(self)->std::result::Result<Self::Value,E>{Ok(StrictValue(Value::Null))}
            fn visit_seq<A:SeqAccess<'de>>(self,mut seq:A)->std::result::Result<Self::Value,A::Error>{let mut a=Vec::new();while let Some(StrictValue(v))=seq.next_element()?{a.push(v);}Ok(StrictValue(Value::Array(a)))}
            fn visit_map<A:MapAccess<'de>>(self,mut map:A)->std::result::Result<Self::Value,A::Error>{let mut o=Map::new();while let Some(k)=map.next_key::<String>()?{if o.contains_key(&k){return Err(de::Error::custom("duplicate key"));}let StrictValue(v)=map.next_value()?;o.insert(k,v);}Ok(StrictValue(Value::Object(o)))}
        }
        deserializer.deserialize_any(JsonVisitor)
    }
}
pub fn strict_json(bytes:&[u8])->Result<Value>{
    ensure(bytes.len()<=MAX_FRAME*4,"INPUT_TOO_LARGE")?;
    let mut d=serde_json::Deserializer::from_slice(bytes);
    let StrictValue(value)=StrictValue::deserialize(&mut d)?;d.end()?;finite(&value,0)?;Ok(value)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentEnvelope {connection_id:String,token:String,method:String,params:Value}
#[cfg(unix)]
pub async fn serve(runtime:Arc<Runtime>,path:&Path)->Result<()> {
    use std::os::unix::fs::{FileTypeExt,PermissionsExt};
    use tokio::net::UnixListener;
    if path.exists(){ensure(std::fs::symlink_metadata(path)?.file_type().is_socket(),"UNSAFE_PATH")?;if tokio::net::UnixStream::connect(path).await.is_ok(){return Err(Error::new("APP_ALREADY_RUNNING"));}std::fs::remove_file(path)?;}
    let listener=UnixListener::bind(path)?;std::fs::set_permissions(path,std::fs::Permissions::from_mode(0o600))?;
    let slots=Arc::new(tokio::sync::Semaphore::new(16));
    loop {
        let (mut stream,_)=listener.accept().await?;
        let Ok(permit)=slots.clone().try_acquire_owned()else{continue;};
        let runtime=runtime.clone();
        tokio::spawn(async move {
            let _permit=permit;
            let process=async {
                let mut header=[0u8;4];stream.read_exact(&mut header).await?;
                let n=u32::from_be_bytes(header) as usize;ensure(n<=MAX_REQUEST,"INPUT_TOO_LARGE")?;
                let mut raw=vec![0u8;n];stream.read_exact(&mut raw).await?;
                let envelope:AgentEnvelope=serde_json::from_value(strict_json(&raw)?)?;
                let rt=runtime.clone();
                let identity=(envelope.connection_id.clone(),envelope.token.clone());
                let reply=tokio::task::spawn_blocking(move||rt.agent_call(&envelope.connection_id,&envelope.token,&envelope.method,envelope.params)).await.map_err(|_|Error::new("INTERNAL_ERROR"))?;
                let response=match reply {
                    Ok(ticket)=>match runtime.validate_ticket(&identity.0,&identity.1,&ticket){Ok(())=>serde_json::json!({"ok":true,"result":ticket.value}),Err(e)=>serde_json::json!({"ok":false,"error":e})},
                    Err(e)=>serde_json::json!({"ok":false,"error":e}),
                };
                let bytes=serde_json::to_vec(&response)?;ensure(bytes.len()<=MAX_REQUEST,"OUTPUT_TOO_LARGE")?;
                stream.write_all(&(bytes.len()as u32).to_be_bytes()).await?;stream.write_all(&bytes).await?;stream.shutdown().await?;Ok::<(),Error>(())
            };
            // Malformed or slow connections are simply closed, never echoed/logged.
            let _=tokio::time::timeout(Duration::from_secs(30),process).await;
        });
    }
}
#[cfg(unix)]
pub async fn call(path:&Path,connection_id:&str,token:&str,method:&str,params:Value)->Result<Value>{
    let mut stream=tokio::net::UnixStream::connect(path).await.map_err(|_|Error::new("APP_UNAVAILABLE"))?;
    let raw=serde_json::to_vec(&serde_json::json!({"connection_id":connection_id,"token":token,"method":method,"params":params}))?;ensure(raw.len()<=MAX_REQUEST,"INPUT_TOO_LARGE")?;
    let work=async {
        stream.write_all(&(raw.len()as u32).to_be_bytes()).await?;stream.write_all(&raw).await?;
        let mut header=[0u8;4];stream.read_exact(&mut header).await?;let n=u32::from_be_bytes(header)as usize;ensure(n<=MAX_REQUEST,"OUTPUT_TOO_LARGE")?;
        let mut bytes=vec![0u8;n];stream.read_exact(&mut bytes).await?;let value=strict_json(&bytes)?;
        if value["ok"]==true{Ok(value["result"].clone())}else{Err(serde_json::from_value(value["error"].clone()).unwrap_or_else(|_|Error::new("IPC_ERROR")))}
    };
    tokio::time::timeout(Duration::from_secs(30),work).await.map_err(|_|Error::new("IPC_TIMEOUT"))?
}
