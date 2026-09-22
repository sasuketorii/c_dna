mod mcp_bridge;
use anyhow::{Context, Result, ensure};
use axum::{Router, extract::{DefaultBodyLimit, State}, body::Bytes, http::{HeaderMap, StatusCode}, response::{IntoResponse, Response}, routing::post, Json};
use cdna_app::{Authority, Engine, error_json};
use cdna_store::Store;
use clap::{Parser, Subcommand};
use serde::{Deserialize,Serialize};
use serde_json::{Value,json};
use std::{io::Read, net::Ipv4Addr, path::{Path,PathBuf}, sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}}};
use tower_http::services::ServeDir;
use uuid::Uuid;
use zeroize::Zeroizing;
use sha2::{Digest,Sha256};

#[derive(Parser)]
#[command(name="cdna",about="C-DNA ローカル判断学習基盤",version)]
struct Cli {
    /// Encrypted vault directory (never an encryption key).
    #[arg(long,default_value="data/local/vault")] vault:PathBuf,
    #[arg(long)] learner_dir:Option<PathBuf>,
    /// Host-pinned evaluator public key file, never a request parameter.
    #[arg(long, requires="human_public_key")] evaluator_public_key:Option<PathBuf>,
    /// Host-pinned human approval public key file.
    #[arg(long, requires="evaluator_public_key")] human_public_key:Option<PathBuf>,
    #[command(subcommand)] command:Commands,
}
#[derive(Subcommand)]
enum Commands {
    /// Create an encrypted vault with a macOS Keychain key.
    Init,
    /// Run a signed evolution operation with host-pinned public verification keys.
    Evolution { #[arg(long)] evaluator_key:PathBuf, #[arg(long)] human_key:PathBuf },
    /// Import selected conversation JSON as pending sources, never training labels.
    Import { #[arg(long)] provider:String, #[arg(long)] workspace:Uuid, #[arg(long)] conversation:Vec<String>, #[arg(long)] cursor:Option<String>, #[arg(long,default_value_t=1000)] page_size:usize, #[arg(long)] preview:bool, file:PathBuf },
    /// Bootstrap from the judgment profile your existing ChatGPT or Claude knows.
    Bootstrap { #[command(subcommand)] command:BootstrapCommand },
    /// Execute one owner command read from bounded JSON stdin.
    Exec,
    /// Keep the private local core available to scoped MCP clients, without a UI.
    Serve,
    /// Register a scoped local MCP client; secret is stored in Keychain.
    Grant { #[arg(long)] workspace:Uuid, #[arg(long, value_delimiter=',', default_value="decision:read")] scopes:Vec<String>, #[arg(long,default_value_t=24)] hours:u16 },
    Revoke { id:Uuid },
    /// stdio MCP connecting to an already unlocked local core.
    Mcp { #[arg(long)] grant:Uuid },
    /// Write an encrypted, separately protected recovery backup.
    Backup { output:PathBuf },
    /// Restore to a new vault, reconciling both kinds of known deletion tombstones.
    Restore { backup:PathBuf, destination:PathBuf, #[arg(long)] offline:bool },
    /// Local browser playground; --demo uses a separate disposable encrypted vault.
    Playground { #[arg(long)] demo:bool, #[arg(long,default_value_t=4317)] port:u16, #[arg(long)] assets_dir:Option<PathBuf> },
}
#[derive(Subcommand)]
enum BootstrapCommand {
    /// Print the prompt to paste into your existing ChatGPT or Claude conversation.
    Prompt,
    /// Validate and store an AI-generated profile as hypotheses; --preview writes nothing.
    Import { #[arg(long)] workspace:Uuid, #[arg(long)] preview:bool, file:PathBuf },
    /// Read the stored profile and its current revision.
    Show { #[arg(long)] workspace:Uuid, #[arg(long)] source:Uuid },
    /// Correct a profile at the given revision; changed text becomes local-only again.
    Revise { #[arg(long)] workspace:Uuid, #[arg(long)] source:Uuid, #[arg(long)] revision:u64, file:PathBuf },
    /// Allow or revoke this profile in the context sent to your connected teaching LLM.
    Share { #[arg(long)] workspace:Uuid, #[arg(long)] source:Uuid, #[arg(long)] revision:u64, #[arg(long,action=clap::ArgAction::Set)] enabled:bool },
}
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {schema_version:String,vault_id:Uuid}
#[derive(Clone)]
struct WebState {engine:Arc<Mutex<Engine>>, token:Arc<String>,origin:String, requests:Arc<tokio::sync::Semaphore>, training:Arc<tokio::sync::Semaphore>, cancellation:Arc<Mutex<Option<Arc<AtomicBool>>>>}
#[tokio::main]
async fn main() {
    if let Err(error)=run().await {eprintln!("{}",error_json(&error));std::process::exit(1);}
}
// Reserve the leaf atomically before creating any credentials or database.
fn reserve_vault_directory(path:&Path)->Result<()> {
    if let Some(parent)=path.parent().filter(|p| !p.as_os_str().is_empty()) { std::fs::create_dir_all(parent)?; }
    std::fs::create_dir(path).context("INPUT_INVALID: vault destination already exists or cannot be created")?;
    Ok(())
}
fn open_vault(path:&Path,create:bool)->Result<Store> {
    if create {
        reserve_vault_directory(path)?;
        #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;std::fs::set_permissions(path,std::fs::Permissions::from_mode(0o700))?;}
        let id=Uuid::new_v4();
        #[cfg(target_os="macos")]
        let key=cdna_store::keychain::create(id)?;
        #[cfg(not(target_os="macos"))]
        let key:Zeroizing<String>={anyhow::bail!("PROVIDER_UNAVAILABLE: supported OS credential store required")};
        let store=Store::create(path.join("vault.db"),&key)?;
        let manifest=Manifest{schema_version:"1.0".into(),vault_id:id};
        std::fs::write(path.join("manifest.json"),serde_json::to_vec_pretty(&manifest)?)?;
        Ok(store)
    } else {
        let manifest:Manifest=serde_json::from_slice(&std::fs::read(path.join("manifest.json"))?)?;
        ensure!(manifest.schema_version=="1.0","SCHEMA_UNSUPPORTED");
        #[cfg(target_os="macos")]
        let key=cdna_store::keychain::load(manifest.vault_id)?;
        #[cfg(not(target_os="macos"))]
        let key:Zeroizing<String>={anyhow::bail!("PROVIDER_UNAVAILABLE: supported OS credential store required")};
        Ok(Store::open(path.join("vault.db"),&key)?)
    }
}
async fn run()->Result<()> {
    let cli=Cli::parse();
    let learner=cli.learner_dir.unwrap_or_else(||PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../learner"));
    let configure = |engine: Engine| -> Result<Engine> {
        match (&cli.evaluator_public_key, &cli.human_public_key) {
            (Some(e), Some(h)) => engine.with_evolution_trust(
                ed25519_dalek::VerifyingKey::from_bytes(&read_public_key(e)?)?,
                ed25519_dalek::VerifyingKey::from_bytes(&read_public_key(h)?)?),
            (None, None) => Ok(engine),
            _ => anyhow::bail!("EVOLUTION_UNTRUSTED: both pinned public keys required"),
        }
    };
    match cli.command {
        Commands::Init=>{let mut engine=Engine::new(open_vault(&cli.vault,true)?,learner,false);let result=engine.execute(json!({"operation":"workspace_create","name":"マイワークスペース"}),Authority::Human)?;println!("{}",json!({"ok":true,"result":result}));}
        Commands::Import{provider,workspace,conversation,cursor,page_size,preview,file}=>{
            let provider=match provider.as_str(){"chatgpt"=>cdna_app::importer::Provider::ChatGpt,"claude"=>cdna_app::importer::Provider::Claude,_=>anyhow::bail!("INPUT_INVALID: provider must be chatgpt or claude")};
            let mut bytes=Vec::new();std::fs::File::open(file)?.take((cdna_app::importer::MAX_EXPORT_BYTES+1) as u64).read_to_end(&mut bytes)?;
            let result=cdna_app::import_workflow::import_page(&mut open_vault(&cli.vault,false)?,workspace,provider,&bytes,&conversation,cursor.as_deref(),page_size,preview)?;
            println!("{}",json!({"ok":true,"result":result,"training_eligible":false}));
        }
        Commands::Bootstrap{command}=>{
            use cdna_app::bootstrap;
            if matches!(&command,BootstrapCommand::Prompt) {
                print!("{}",bootstrap::PROMPT);
                return Ok(());
            }
            let mut store=open_vault(&cli.vault,false)?;
            let read_profile=|file:PathBuf|->Result<Vec<u8>> {
                let mut bytes=Vec::new();
                std::fs::File::open(file)?.take((bootstrap::MAX_PROFILE_BYTES+1) as u64).read_to_end(&mut bytes)?;
                ensure!(bytes.len()<=bootstrap::MAX_PROFILE_BYTES,"BOOTSTRAP_SIZE_LIMIT");
                Ok(bytes)
            };
            let result=match command {
                BootstrapCommand::Prompt=>unreachable!(),
                BootstrapCommand::Import{workspace,preview,file}=>{
                    // Stable host namespace makes retry identity independent of provider claims.
                    let namespace=Uuid::from_u128(0x927e57b8_aa2a_4c06_89c4_90c704161a9c);
                    serde_json::to_value(bootstrap::import_profile(&mut store,&Authority::Human,workspace,namespace,&read_profile(file)?,preview)?)?
                }
                BootstrapCommand::Show{workspace,source}=>{
                    let document=store.get_document(cdna_store::DocumentKind::Source,workspace,source)?;
                    ensure!(document.payload["kind"]=="bootstrap_profile","BOOTSTRAP_SOURCE_INVALID");
                    serde_json::to_value(document)?
                }
                BootstrapCommand::Revise{workspace,source,revision,file}=>serde_json::to_value(bootstrap::revise_profile(&mut store,&Authority::Human,workspace,source,revision,&read_profile(file)?)?)?,
                BootstrapCommand::Share{workspace,source,revision,enabled}=>serde_json::to_value(bootstrap::set_teacher_sharing(&mut store,&Authority::Human,workspace,source,revision,enabled)?)?,
            };
            println!("{}",json!({"ok":true,"result":result}));
        }
        Commands::Evolution{evaluator_key,human_key}=>{
            let evaluator=read_public_key(&evaluator_key)?;let human=read_public_key(&human_key)?;
            let mut bytes=Vec::new();std::io::stdin().take(1024*1024+1).read_to_end(&mut bytes)?;
            let command=cdna_domain::parse_json_with_limit(&bytes,1024*1024)?;
            let result=cdna_app::evolution_cli::run(&mut open_vault(&cli.vault,false)?,command,evaluator,human)?;
            println!("{}",json!({"ok":true,"result":result}));
        }
        Commands::Exec=>{
            let mut input=Vec::new();std::io::stdin().take(256*1024+1).read_to_end(&mut input)?;
            ensure!(input.len()<=256*1024,"INPUT_INVALID: request too large");
            let mut engine=configure(Engine::new(open_vault(&cli.vault,false)?,learner,false))?;
            let result=engine.execute(cdna_domain::parse_json(&input)?,Authority::Human)?;
            println!("{}",json!({"ok":true,"result":result}));
        }
        Commands::Serve=>{
            let engine=Arc::new(Mutex::new(configure(Engine::new(open_vault(&cli.vault,false)?,learner,false))?));
            let socket=cli.vault.join("bridge/core.sock");
            tokio::select! {
                result=mcp_bridge::serve_local(engine,socket)=>result?,
                result=tokio::signal::ctrl_c()=>result?,
            }
        }
        Commands::Grant{workspace,scopes,hours}=>{
            ensure!(hours>0&&hours<=720,"INPUT_INVALID: grant validity must be 1–720 hours");
            let mut store=open_vault(&cli.vault,false)?;
            let id=Uuid::new_v4();
            let token=cdna_store::keychain::create(id)?;
            let token_hash=format!("{:x}",Sha256::digest(token.as_bytes()));
            store.create_grant(cdna_store::Grant{id,workspace_id:workspace,scopes,expires_at:chrono::Utc::now().timestamp()+i64::from(hours)*3600,revoked:false,token_hash})?;
            println!("{}",json!({"ok":true,"grant_id":id,"secret_storage":"macos_keychain","mcp_arguments":["--vault",cli.vault,"mcp","--grant",id]}));
        }
        Commands::Revoke{id}=>{open_vault(&cli.vault,false)?.revoke_grant(id)?;println!("{}",json!({"ok":true,"revoked":id}));}
        Commands::Mcp{grant}=>{
            let token=cdna_store::keychain::load(grant)?;
            mcp_bridge::serve_stdio(cli.vault.join("bridge/core.sock"),token).await?;
        }
        Commands::Backup{output}=>{
            let recovery=Zeroizing::new(rpassword::prompt_password("復元用パスフレーズ（32文字以上）: ")?);
            let confirmation=Zeroizing::new(rpassword::prompt_password("復元用パスフレーズを再入力: ")?);
            ensure!(*recovery==*confirmation,"INPUT_INVALID: passphrases differ");
            open_vault(&cli.vault,false)?.backup(output,&recovery)?;
            println!("{}",json!({"ok":true,"format":"sqlcipher-v1","encrypted":true}));
        }
        Commands::Restore{backup,destination,offline}=>{
            ensure!(!destination.exists(),"INPUT_INVALID: destination exists");
            let recovery=Zeroizing::new(rpassword::prompt_password("復元用パスフレーズ: ")?);
            let (tombstones,documents)=if offline{(Vec::new(),Vec::new())}else{let current=open_vault(&cli.vault,false)?;(current.tombstones()?,current.document_tombstones()?)};
            reserve_vault_directory(&destination)?;
            #[cfg(unix)] {use std::os::unix::fs::PermissionsExt;std::fs::set_permissions(&destination,std::fs::Permissions::from_mode(0o700))?;}
            let id=Uuid::new_v4();let key=cdna_store::keychain::create(id)?;
            let restored=Store::restore(backup,&recovery,destination.join("vault.db"),&key,&tombstones,&documents)?;
            drop(restored);
            std::fs::write(destination.join("manifest.json"),serde_json::to_vec_pretty(&Manifest{schema_version:"1.0".into(),vault_id:id})?)?;
            println!("{}",json!({"ok":true,"destination":destination,"unknown_later_deletions":offline,"restored_grants":"revoked"}));
        }
        Commands::Playground{demo,port,assets_dir}=>{
            let demo_dir=if demo {Some(tempfile::tempdir()?)}else{None};
            let store=if let Some(dir)=&demo_dir {Store::create(dir.path().join("demo.db"),&Zeroizing::new(format!("{}{}",Uuid::new_v4(),Uuid::new_v4())))?}else{open_vault(&cli.vault,false)?};
            let mut engine=configure(Engine::new(store,learner,demo))?;
            if demo {engine.execute(json!({"operation":"workspace_create","name":"架空データのプレイグラウンド"}),Authority::Human)?;}
            let token=format!("{}{}",Uuid::new_v4().simple(),Uuid::new_v4().simple());
            let listener=tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST,port)).await.context("local playground port unavailable")?;
            let origin=format!("http://127.0.0.1:{}",listener.local_addr()?.port());
            let state=WebState{engine:Arc::new(Mutex::new(engine)),token:Arc::new(token),origin:origin.clone(),requests:Arc::new(tokio::sync::Semaphore::new(8)),training:Arc::new(tokio::sync::Semaphore::new(1)),cancellation:Arc::new(Mutex::new(None))};
            let assets=assets_dir.unwrap_or_else(||PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/dist"));
            ensure!(assets.join("index.html").is_file(),"PROVIDER_UNAVAILABLE: build apps/desktop first");
            let app=Router::new().route("/api/command",post(command)).fallback_service(ServeDir::new(assets)).layer(DefaultBodyLimit::max(256*1024)).with_state(state.clone());
            eprintln!("C-DNA: {origin}/#session={}",state.token);
            let socket=demo_dir.as_ref().map(|d|d.path().join("bridge/core.sock")).unwrap_or_else(||cli.vault.join("bridge/core.sock"));
            let http=axum::serve(listener,app).with_graceful_shutdown(async{let _=tokio::signal::ctrl_c().await;});
            tokio::select! {result=http=>result?, result=mcp_bridge::serve_local(state.engine.clone(),socket)=>result?};
            drop(state);drop(demo_dir);
        }
    } Ok(())
}
async fn command(State(state):State<WebState>,headers:HeaderMap,body:Bytes)->Response {
    let host=state.origin.trim_start_matches("http://");
    let origin=headers.get("origin").and_then(|s|s.to_str().ok());
    let bearer=headers.get("authorization").and_then(|s|s.to_str().ok()).and_then(|s|s.strip_prefix("Bearer "));
    if headers.get("host").and_then(|s|s.to_str().ok())!=Some(host)||origin!=Some(state.origin.as_str())||bearer!=Some(state.token.as_str()) {
        return (StatusCode::FORBIDDEN,Json(json!({"ok":false,"error":{"code":"SCOPE_DENIED","message":"接続セッションが無効です。"}}))).into_response();
    }
    let input=match cdna_domain::parse_json(&body) {Ok(v)=>v,Err(e)=>return (StatusCode::BAD_REQUEST,Json(error_json(&e.into()))).into_response()};
    let permit=match state.requests.clone().try_acquire_owned(){Ok(p)=>p,Err(_)=>return (StatusCode::TOO_MANY_REQUESTS,Json(json!({"ok":false,"error":{"code":"RATE_LIMITED"}}))).into_response()};
    let result=tokio::task::spawn_blocking(move|| {
        let _permit=permit;
        if input["operation"]=="lock" {
            if let Ok(cancel)=state.cancellation.lock(){if let Some(flag)=cancel.as_ref(){flag.store(true,Ordering::Release);}}
        }
        let execute=||->Result<Value>{
            if input["operation"]=="train" {
                let _train_permit=state.training.clone().try_acquire_owned().map_err(|_|anyhow::anyhow!("RATE_LIMITED: training already running"))?;
                let cancel=Arc::new(AtomicBool::new(false));
                *state.cancellation.lock().map_err(|_|anyhow::anyhow!("INTERNAL_ERROR"))?=Some(cancel.clone());
                // Validate the entire envelope, then release the DB mutex while Python runs.
                #[derive(Deserialize)] #[serde(deny_unknown_fields)] struct TrainInput {operation:String,workspace_id:Uuid, #[serde(default)] domain:Option<cdna_domain::Domain>}
                let command:TrainInput=serde_json::from_value(input.clone())?;
                ensure!(command.operation=="train","INPUT_INVALID");
                let task=state.engine.lock().map_err(|_|anyhow::anyhow!("INTERNAL_ERROR"))?.prepare_train_domain(command.workspace_id,command.domain)?;
                let trained=cdna_app::run_training(&task,cancel.clone());
                let mut engine=state.engine.lock().map_err(|_|anyhow::anyhow!("INTERNAL_ERROR"))?;
                ensure!(!cancel.load(Ordering::Acquire),"CANCELLED");
                engine.finish_train(task,trained?)
            }else{state.engine.lock().map_err(|_|anyhow::anyhow!("INTERNAL_ERROR"))?.execute(input,Authority::Human)}
        };
        match execute(){Ok(result)=>json!({"ok":true,"result":result}),Err(e)=>error_json(&e)}
    }).await;
    match result {Ok(body)=>Json(body).into_response(),Err(_)=>(StatusCode::INTERNAL_SERVER_ERROR,Json(json!({"ok":false}))).into_response()}
}

fn read_public_key(path:&Path)->Result<[u8;32]> {
    let mut bytes=Vec::new();std::fs::File::open(path)?.take(1025).read_to_end(&mut bytes)?;
    ensure!(bytes.len()<=1024,"INPUT_INVALID: public key file too large");
    let text=std::str::from_utf8(&bytes)?.trim();
    ensure!(text.len()==64&&text.bytes().all(|b|b.is_ascii_hexdigit()),"INPUT_INVALID: public key must be 64 hexadecimal characters");
    let mut key=[0u8;32];
    for (i,part) in text.as_bytes().chunks_exact(2).enumerate(){key[i]=u8::from_str_radix(std::str::from_utf8(part)?,16)?;}
    Ok(key)
}

#[cfg(test)]
mod vault_creation_tests {
    use super::*;
    #[test]
    fn concurrent_leaf_reservation_has_exactly_one_owner() {
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("nested/vault");
        let results=std::thread::scope(|scope| {
            let a=scope.spawn(|| reserve_vault_directory(&path).is_ok());
            let b=scope.spawn(|| reserve_vault_directory(&path).is_ok());
            [a.join().unwrap(),b.join().unwrap()]
        });
        assert_eq!(results.into_iter().filter(|ok|*ok).count(),1);
        std::fs::write(path.join("marker"),b"preserve").unwrap();
        assert!(reserve_vault_directory(&path).is_err());
        assert_eq!(std::fs::read(path.join("marker")).unwrap(),b"preserve");
    }
    #[cfg(unix)]
    #[test]
    fn dangling_leaf_is_never_followed() {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("vault");
        std::os::unix::fs::symlink(dir.path().join("absent"),&path).unwrap();
        assert!(reserve_vault_directory(&path).is_err());
        assert!(!dir.path().join("absent").exists());
    }
}
