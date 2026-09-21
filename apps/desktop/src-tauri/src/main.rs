#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use cdna_core::{Runtime,domain::{Error,Result},jobs::LearnerCommand,keys};
use serde_json::Value;
use std::sync::Arc;
use tauri::{Manager,State};

#[tauri::command]
async fn owner_call(window:tauri::WebviewWindow,runtime:State<'_,Arc<Runtime>>,method:String,params:Value)->Result<Value>{
    if window.label()!="main"{return Err(Error::new("SCOPE_DENIED"));}
    let runtime=runtime.inner().clone();
    tauri::async_runtime::spawn_blocking(move||runtime.owner_call(&method,params)).await.map_err(|_|Error::new("INTERNAL_ERROR"))?
}
#[tauri::command]
async fn import_file(window:tauri::WebviewWindow,runtime:State<'_,Arc<Runtime>>,workspace_id:String)->Result<Value>{
    if window.label()!="main"{return Err(Error::new("SCOPE_DENIED"));}
    let file=rfd::AsyncFileDialog::new().add_filter("会話エクスポート",&["json","html","zip"]).pick_file().await.ok_or_else(||Error::new("CANCELLED"))?;
    let path=file.path().to_owned();let runtime=runtime.inner().clone();
    tauri::async_runtime::spawn_blocking(move||{
        let size=std::fs::metadata(&path)?.len();if size>cdna_core::imports::MAX_COMPRESSED{return Err(Error::new("DOCUMENT_TOO_LARGE"));}
        let format=path.extension().and_then(|x|x.to_str()).ok_or_else(||Error::new("UNSUPPORTED_EXPORT_FORMAT"))?;
        runtime.import_bytes(&workspace_id,format,&std::fs::read(&path)?)
    }).await.map_err(|_|Error::new("INTERNAL_ERROR"))?
}
#[tauri::command]
async fn save_export(window:tauri::WebviewWindow,data:String,kind:String)->Result<bool>{
    if window.label()!="main"||data.len()>256*1024*1024{return Err(Error::new("SCOPE_DENIED"));}
    let name=match kind.as_str(){"backup"=>"c-dna-backup.cdna","diagnostic"=>"c-dna-diagnostic.json","workspace"=>"c-dna-workspace.json",_=>return Err(Error::new("INPUT_INVALID"))};
    let Some(file)=rfd::AsyncFileDialog::new().set_file_name(name).save_file().await else{return Ok(false);};
    file.write(data.as_bytes()).await?;Ok(true)
}
fn main(){
    tauri::Builder::default().setup(|app|{
        let root=app.path().app_data_dir()?;
        let resources=app.path().resource_dir()?;
        // Installed applications use a bundled, hash-checked learner. The dev
        // launch script supplies paths only in debug builds, never through IPC.
        let learner={
            #[cfg(debug_assertions)]{match(std::env::var("CDNA_DEV_PYTHON"),std::env::var("CDNA_SOURCE_ROOT")){(Ok(p),Ok(s))=>LearnerCommand::development(p.into(),s.into()),_=>LearnerCommand::packaged(&resources)}}
            #[cfg(not(debug_assertions))]{LearnerCommand::packaged(&resources)}
        };
        let runtime=Runtime::new(root,keys::system(),learner)?;
        #[cfg(unix)]{let server=runtime.clone();let path=runtime.socket_path();tauri::async_runtime::spawn(async move{let _=cdna_core::transport::serve(server,&path).await;});}
        app.manage(runtime);Ok(())
    }).invoke_handler(tauri::generate_handler![owner_call,import_file,save_export])
      .on_window_event(|window,event|{if matches!(event,tauri::WindowEvent::Destroyed){if let Some(rt)=window.app_handle().try_state::<Arc<Runtime>>(){let _=rt.owner_call("lock",serde_json::json!({}));}}})
      .run(tauri::generate_context!()).expect("C-DNA desktop runtime could not start");
}
