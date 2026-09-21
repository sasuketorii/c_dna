//! A bounded, cancellable sidecar process. No DB handle or vault key crosses IPC.
use crate::domain::*;
use serde_json::{json,Value};
use std::{path::{Path,PathBuf},sync::{Arc,atomic::{AtomicBool,Ordering}},time::Duration};
use tokio::{io::{AsyncReadExt,AsyncWriteExt},process::Command};

#[derive(Clone,Debug)]
pub struct LearnerCommand {pub executable:PathBuf,pub args:Vec<String>,pub python_path:Option<PathBuf>}
impl LearnerCommand {
    pub fn packaged(resource_dir:&Path)->Self {
        Self{executable:resource_dir.join("runtime/cdna-learner/cdna-learner"),args:vec![],python_path:None}
    }
    /// This constructor is only used by the developer bridge/test harness.
    pub fn development(python:PathBuf,source_root:PathBuf)->Self {
        Self{executable:python,args:vec!["-m".into(),"cdna_learner".into()],python_path:Some(source_root.join("learner/src"))}
    }
    pub fn available(&self)->bool {self.executable.is_file()}
}
pub async fn learn(config:LearnerCommand,snapshot:Value,cancelled:Arc<AtomicBool>)->Result<Value>{
    ensure(config.available(),"LEARNER_UNAVAILABLE")?;
    let input=serde_json::to_vec(&json!({"operation":"train","snapshot":snapshot}))?;
    ensure(input.len()<=MAX_FRAME,"INPUT_TOO_LARGE")?;
    let mut command=Command::new(&config.executable);
    command.args(&config.args).kill_on_drop(true).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    command.env_clear().env("PYTHONNOUSERSITE","1").env("PYTHONDONTWRITEBYTECODE","1").env("PYTHONUTF8","1").env("LC_ALL","C.UTF-8").env("OMP_NUM_THREADS","2").env("OPENBLAS_NUM_THREADS","2").env("MKL_NUM_THREADS","2").env("POLARS_MAX_THREADS","2").env("MODULAR_TELEMETRY_ENABLED","false");
    if let Some(path)=config.python_path{command.env("PYTHONPATH",path);}
    #[cfg(unix)] {use std::os::unix::process::CommandExt;command.as_std_mut().process_group(0);}
    let mut child=command.spawn().map_err(|_|Error::new("LEARNER_UNAVAILABLE"))?;
    let pid=child.id();
    let mut stdin=child.stdin.take().ok_or_else(||Error::new("WORKER_PROTOCOL_ERROR"))?;
    let mut stdout=child.stdout.take().ok_or_else(||Error::new("WORKER_PROTOCOL_ERROR"))?;
    let worker=async {
        stdin.write_all(&(input.len() as u32).to_be_bytes()).await?;stdin.write_all(&input).await?;stdin.shutdown().await?;drop(stdin);
        let mut header=[0u8;4];stdout.read_exact(&mut header).await?;
        let size=u32::from_be_bytes(header) as usize;ensure(size<=MAX_FRAME,"OUTPUT_TOO_LARGE")?;
        let mut raw=vec![0u8;size];stdout.read_exact(&mut raw).await?;
        let mut extra=[0u8;1];ensure(stdout.read(&mut extra).await?==0,"WORKER_PROTOCOL_ERROR")?;
        let output=crate::transport::strict_json(&raw)?;
        if output["ok"]==true{Ok(output["result"].clone())}else{Err(Error::new(output["error"]["code"].as_str().filter(|s|s.len()<64&&s.bytes().all(|b|b.is_ascii_uppercase()||b==b'_')).unwrap_or("TRAINING_FAILED")))}
    };
    let cancellation=async {while !cancelled.load(Ordering::SeqCst){tokio::time::sleep(Duration::from_millis(30)).await;}};
    let result=tokio::select! {
        result=worker=>result,
        _=cancellation=>Err(Error::new("CANCELLED")),
        _=tokio::time::sleep(Duration::from_secs(180))=>Err(Error::new("WORKER_TIMEOUT")),
    };
    if result.is_err()||cancelled.load(Ordering::SeqCst){
        #[cfg(unix)] if let Some(pid)=pid {unsafe{libc::kill(-(pid as i32),libc::SIGKILL);}}
        let _=child.kill().await;
    }
    let status=child.wait().await?;
    ensure(!cancelled.load(Ordering::SeqCst),"CANCELLED")?;
    if !status.success()&&result.is_ok(){return Err(Error::new("TRAINING_FAILED"));}
    result
}
