//! Explicitly opt-in developer/test bridge; never bundled in production.
//! One JSON line in, one JSON line out, on private inherited pipes only.
use cdna_core::{Runtime,keys::MemoryKeys,jobs::LearnerCommand,transport::strict_json,domain::Error};
use serde_json::{json,Value};
use std::{path::PathBuf,sync::Arc};
use tokio::io::{AsyncBufReadExt,AsyncWriteExt,BufReader};
#[tokio::main]
async fn main(){
    let mut args=std::env::args().skip(1);
    let Some(root)=args.next().map(PathBuf::from)else{eprintln!("TEST_ROOT_REQUIRED");return;};
    let Some(source)=args.next().map(PathBuf::from)else{eprintln!("SOURCE_ROOT_REQUIRED");return;};
    let Some(python)=args.next().map(PathBuf::from)else{eprintln!("PYTHON_REQUIRED");return;};
    if !root.is_absolute()||!source.is_absolute()||!python.is_absolute(){eprintln!("ABSOLUTE_PATH_REQUIRED");return;}
    let rt=match Runtime::new(root,Arc::new(MemoryKeys::default()),LearnerCommand::development(python,source)){Ok(rt)=>rt,Err(e)=>{eprintln!("{}",e.code);return;}};
    #[cfg(unix)]{let server=rt.clone();let socket=rt.socket_path();tokio::spawn(async move{let _=cdna_core::transport::serve(server,&socket).await;});}
    let mut reader=BufReader::new(tokio::io::stdin());let mut stdout=tokio::io::stdout();
    loop{
        let mut raw=vec![];let n=reader.read_until(b'\n',&mut raw).await.unwrap_or(0);if n==0{break;}
        if raw.len()>256*1024*1024{break;}
        let value=strict_json(&raw);let request_id=value.as_ref().ok().and_then(|v|v.get("id")).cloned().unwrap_or(Value::Null);
        let result=match value{Ok(v)=>{
            let runtime=rt.clone();tokio::task::spawn_blocking(move||runtime.owner_call(v["method"].as_str().unwrap_or(""),v["params"].clone())).await.unwrap_or_else(|_|Err(Error::new("INTERNAL_ERROR")))
        },Err(e)=>Err(e)};
        let answer=match result{Ok(result)=>json!({"id":request_id,"ok":true,"result":result}),Err(error)=>json!({"id":request_id,"ok":false,"error":error})};
        if stdout.write_all(format!("{answer}\n").as_bytes()).await.is_err(){break;}let _=stdout.flush().await;
    }
    let _=rt.owner_call("lock",json!({}));
}
