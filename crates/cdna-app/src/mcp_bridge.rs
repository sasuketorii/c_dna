//! Private local bridge to the desktop's already unlocked Engine. Never opens a vault.
use anyhow::{Result, bail, ensure};
use cdna_app::{Authority, Engine};
use cdna_domain::{MAX_REQUEST_BYTES, ObservationProposal, RankRequest};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
    sync::Semaphore,
};
use uuid::Uuid;
use zeroize::Zeroizing;
const DEADLINE: Duration = Duration::from_secs(30);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    token: String,
    command: Value,
}
fn uid() -> u32 {
    rustix::process::geteuid().as_raw()
}
fn private_directory(path: &Path, create: bool) -> Result<()> {
    if create && !path.exists() {
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(path)?;
    }
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == uid()
            && metadata.mode() & 0o777 == 0o700,
        "SCOPE_DENIED: socket directory must be private"
    );
    Ok(())
}
fn private_socket(path: &Path) -> Result<()> {
    private_directory(
        path.parent()
            .ok_or_else(|| anyhow::anyhow!("INPUT_INVALID"))?,
        false,
    )?;
    let m = fs::symlink_metadata(path)?;
    ensure!(
        m.file_type().is_socket() && m.uid() == uid() && m.mode() & 0o777 == 0o600,
        "SCOPE_DENIED: invalid socket"
    );
    Ok(())
}
struct SocketCleanup {
    path: PathBuf,
    inode: u64,
}
impl Drop for SocketCleanup {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path)
            .is_ok_and(|m| m.ino() == self.inode && m.file_type().is_socket() && m.uid() == uid())
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}
fn code(error: &anyhow::Error) -> String {
    if matches!(
        error.downcast_ref::<cdna_store::StoreError>(),
        Some(cdna_store::StoreError::Locked)
    ) {
        return "VAULT_LOCKED".into();
    }
    let message = error.to_string();
    for c in [
        "SCOPE_DENIED",
        "VAULT_LOCKED",
        "PROVIDER_UNAVAILABLE",
        "BUDGET_EXCEEDED",
        "SCHEMA_UNSUPPORTED",
        "CONSENT_REQUIRED",
        "REVISION_CONFLICT",
        "RATE_LIMITED",
    ] {
        if message.contains(c) {
            return c.into();
        }
    }
    "INPUT_INVALID".into()
}
fn failure(c: &str) -> Value {
    json!({"ok":false,"error":{"code":c,"message":c}})
}
fn grant(engine: &Engine, hash: &str) -> Result<cdna_store::Grant> {
    engine.store.authorize_grant(hash).map_err(|e| match e {
        cdna_store::StoreError::Locked => anyhow::Error::new(e),
        _ => anyhow::anyhow!("SCOPE_DENIED"),
    })
}
fn keys(v: &Value, allowed: &[&str]) -> Result<()> {
    let m = v
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("INPUT_INVALID"))?;
    ensure!(
        m.keys().all(|k| allowed.contains(&k.as_str())),
        "INPUT_INVALID"
    );
    Ok(())
}
fn uuid(v: &Value, key: &str) -> Result<Uuid> {
    Ok(Uuid::parse_str(
        v[key]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("INPUT_INVALID"))?,
    )?)
}
fn take_proposal_rate(hash: &str) -> Result<()> {
    use std::{collections::HashMap, sync::OnceLock, time::Instant};
    static RATE: OnceLock<Mutex<HashMap<String, (f64, Instant)>>> = OnceLock::new();
    let mut rates = RATE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| anyhow::anyhow!("INTERNAL_ERROR"))?;
    let now = Instant::now();
    rates.retain(|_, (_, at)| now.duration_since(*at) < Duration::from_secs(60));
    ensure!(
        rates.contains_key(hash) || rates.len() < 128,
        "BUDGET_EXCEEDED"
    );
    let rate = rates.entry(hash.to_owned()).or_insert((10., now));
    rate.0 = (rate.0 + now.duration_since(rate.1).as_secs_f64()).min(10.);
    rate.1 = now;
    ensure!(rate.0 >= 1., "RATE_LIMITED");
    rate.0 -= 1.;
    Ok(())
}
pub(crate) fn execute(engine: &mut Engine, hash: &str, command: Value) -> Result<Value> {
    ensure!(
        serde_json::to_vec(&command)?.len() <= MAX_REQUEST_BYTES,
        "BUDGET_EXCEEDED"
    );
    let granted = grant(engine, hash)?;
    let auth = Authority::Agent {
        workspace_id: granted.workspace_id,
        scopes: granted.scopes.clone(),
    };
    let op = command["operation"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("INPUT_INVALID"))?;
    let result = match op {
        "rank" | "check_policy" => {
            keys(&command, &["operation", "request"])?;
            let request = RankRequest::from_json(&serde_json::to_vec(&command["request"])?)?;
            engine.execute(json!({"operation":op,"request":request}), auth)?
        }
        "enqueue" => {
            keys(&command, &["operation", "request"])?;
            take_proposal_rate(hash)?;
            engine.execute(
                json!({"operation":"with_ai_enqueue","request":command["request"]}),
                auth,
            )?
        }
        "propose" => {
            keys(&command, &["operation", "proposal"])?;
            let proposal =
                ObservationProposal::from_json(&serde_json::to_vec(&command["proposal"])?)?;
            engine.execute(json!({"operation":"propose","proposal":proposal}), auth)?
        }
        "capabilities" | "find" | "gaps" | "policies" | "profile" | "get_job" | "cancel_job" => {
            let allowed = if op == "find" {
                vec![
                    "operation",
                    "schema_version",
                    "request_id",
                    "workspace_id",
                    "query",
                    "limit",
                ]
            } else if op.ends_with("job") {
                vec![
                    "operation",
                    "schema_version",
                    "request_id",
                    "workspace_id",
                    "job_id",
                ]
            } else {
                vec!["operation", "schema_version", "request_id", "workspace_id"]
            };
            keys(&command, &allowed)?;
            ensure!(command["schema_version"] == "1.0", "SCHEMA_UNSUPPORTED");
            uuid(&command, "request_id")?;
            let workspace = uuid(&command, "workspace_id")?;
            ensure!(workspace == granted.workspace_id, "SCOPE_DENIED");
            match op {
                "capabilities" => {
                    let mut status = engine.execute(json!({"operation":"status"}), auth)?;
                    status["capabilities"] = json!([
                        "scoped_decision_search",
                        "observation_proposals",
                        "memory_ranking",
                        "learning_gaps",
                        "policy_check",
                        "train_only_coaching_profile",
                        "question_proposals"
                    ]);
                    status["unsupported_tools"] =
                        json!(["propose_answer", "get_job", "cancel_job"]);
                    status
                }
                "profile" => engine.execute(
                    json!({"operation":"with_ai_profile","workspace_id":workspace}),
                    auth,
                )?,
                "gaps" | "policies" => {
                    engine.execute(json!({"operation":op,"workspace_id":workspace}), auth)?
                }
                "find" => {
                    let query = command["query"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("INPUT_INVALID"))?;
                    let limit = match command.get("limit") {
                        None => 5,
                        Some(value) => value
                            .as_u64()
                            .ok_or_else(|| anyhow::anyhow!("INPUT_INVALID"))?,
                    };
                    ensure!(
                        !query.trim().is_empty()
                            && query.len() <= 4096
                            && (1..=20).contains(&limit),
                        "INPUT_INVALID"
                    );
                    let page = engine
                        .execute(json!({"operation":"list","workspace_id":workspace}), auth)?;
                    let records = page["records"]
                        .as_array()
                        .ok_or_else(|| anyhow::anyhow!("INTERNAL_ERROR"))?;
                    let query = query.to_lowercase();
                    let mut matches = Vec::new();
                    let mut truncated = false;
                    for record in records {
                        if record["status"] != "confirmed" {
                            continue;
                        }
                        let text = record["payload"]["context"]["summary"]
                            .as_str()
                            .unwrap_or("");
                        if text.to_lowercase().contains(&query) {
                            if matches.len() < (limit as usize) {
                                matches.push(record.clone())
                            } else {
                                truncated = true
                            }
                        }
                    }
                    json!({"records":matches,"limit":limit,"search_kind":"case_insensitive_summary_substring","search_window":100,"truncated":truncated||records.len()==100})
                }
                _ => bail!("PROVIDER_UNAVAILABLE"),
            }
        }
        _ => bail!("SCOPE_DENIED"),
    };
    // Durable expiry/revocation and vault lock are checked again before disclosing output.
    grant(engine, hash)?;
    Ok(result)
}
async fn connection(
    mut stream: tokio::net::UnixStream,
    engine: Arc<Mutex<Engine>>,
    permit: tokio::sync::OwnedSemaphorePermit,
) -> Result<()> {
    ensure!(stream.peer_cred()?.uid() == uid(), "SCOPE_DENIED");
    let n = stream.read_u32().await? as usize;
    ensure!(n > 0 && n <= MAX_REQUEST_BYTES, "INPUT_INVALID");
    let mut bytes = Zeroizing::new(vec![0; n]);
    stream.read_exact(&mut bytes).await?;
    let mut envelope: Envelope = serde_json::from_value(cdna_domain::parse_json(&bytes)?)?;
    let token = Zeroizing::new(std::mem::take(&mut envelope.token));
    ensure!((32..=256).contains(&token.len()), "SCOPE_DENIED");
    let hash = format!("{:x}", Sha256::digest(token.as_bytes()));
    let (response, _permit) = tokio::task::spawn_blocking(move || {
        let mut engine = engine
            .lock()
            .map_err(|_| anyhow::anyhow!("INTERNAL_ERROR"))?;
        let value = match execute(&mut engine, &hash, envelope.command) {
            Ok(result) => json!({"ok":true,"result":result}),
            Err(e) => failure(&code(&e)),
        };
        let mut bytes = serde_json::to_vec(&value)?;
        if bytes.len() > MAX_REQUEST_BYTES {
            bytes = serde_json::to_vec(&failure("BUDGET_EXCEEDED"))?;
        }
        // Recheck after output serialization, immediately before passing bytes to the socket writer.
        if let Err(error) = grant(&engine, &hash) {
            bytes = serde_json::to_vec(&failure(&code(&error)))?;
        }
        Ok::<_, anyhow::Error>((bytes, permit))
    })
    .await??;
    stream.write_u32(response.len() as u32).await?;
    stream.write_all(&response).await?;
    stream.shutdown().await?;
    Ok(())
}
pub async fn serve_local(engine: Arc<Mutex<Engine>>, socket_path: PathBuf) -> Result<()> {
    let parent = socket_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("INPUT_INVALID"))?;
    private_directory(parent, true)?;
    ensure!(
        fs::symlink_metadata(&socket_path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "SCOPE_DENIED: socket path already exists"
    );
    let listener = UnixListener::bind(&socket_path)?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
    let _cleanup = SocketCleanup {
        inode: fs::symlink_metadata(&socket_path)?.ino(),
        path: socket_path,
    };
    let slots = Arc::new(Semaphore::new(8));
    loop {
        let permit = slots.clone().acquire_owned().await?;
        let (stream, _) = listener.accept().await?;
        let engine = engine.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(DEADLINE, connection(stream, engine, permit)).await;
        });
    }
}
fn request(socket: &Path, token: &str, command: Value) -> Result<Value> {
    private_socket(socket)?;
    #[derive(serde::Serialize)]
    struct Request<'a> {
        token: &'a str,
        command: Value,
    }
    let bytes = Zeroizing::new(serde_json::to_vec(&Request { token, command })?);
    ensure!(bytes.len() <= MAX_REQUEST_BYTES, "INPUT_INVALID");
    let stream = tokio::runtime::Handle::try_current()?.block_on(async {
        tokio::time::timeout(DEADLINE, tokio::net::UnixStream::connect(socket)).await?
    })?;
    let mut stream: UnixStream = stream.into_std()?;
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(DEADLINE))?;
    stream.set_write_timeout(Some(DEADLINE))?;
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)?;
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let n = u32::from_be_bytes(length) as usize;
    ensure!(n > 0 && n <= MAX_REQUEST_BYTES, "INPUT_INVALID");
    let mut reply = vec![0; n];
    stream.read_exact(&mut reply)?;
    let response: Value = serde_json::from_slice(&reply)?;
    if response["ok"] == true {
        Ok(response["result"].clone())
    } else {
        bail!(
            "{}",
            response["error"]["code"]
                .as_str()
                .unwrap_or("INTERNAL_ERROR")
        )
    }
}
pub async fn serve_stdio(socket_path: PathBuf, token: Zeroizing<String>) -> Result<()> {
    private_socket(&socket_path)?;
    cdna_mcp::serve_stdio(Arc::new(move |command| {
        request(&socket_path, &token, command).map_err(|e| code(&e))
    }))
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(dir: &Path) -> (Arc<Mutex<Engine>>, String, Uuid, Uuid) {
        let mut store =
            cdna_store::Store::create(dir.join("vault.db"), &Zeroizing::new("a".repeat(64)))
                .unwrap();
        let workspace = Uuid::new_v4();
        store.create_workspace(workspace).unwrap();
        let token = "b".repeat(64);
        let id = Uuid::new_v4();
        store
            .create_grant(cdna_store::Grant {
                id,
                workspace_id: workspace,
                scopes: vec![
                    "decision:read".into(),
                    "decision:propose".into(),
                    "judgment:infer".into(),
                ],
                expires_at: chrono::Utc::now().timestamp() + 300,
                revoked: false,
                token_hash: format!("{:x}", Sha256::digest(token.as_bytes())),
            })
            .unwrap();
        (
            Arc::new(Mutex::new(Engine::new(
                store,
                PathBuf::from("unused"),
                true,
            ))),
            token,
            workspace,
            id,
        )
    }
    fn common(operation: &str, workspace: Uuid) -> Value {
        json!({"operation":operation,"workspace_id":workspace,"schema_version":"1.0","request_id":Uuid::new_v4()})
    }
    async fn call(path: PathBuf, token: String, command: Value) -> Result<Value> {
        tokio::task::spawn_blocking(move || request(&path, &token, command)).await?
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn socket_grants_revocation_lock_and_framing() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (engine, token, workspace, id) = fixture(dir.path());
        let path = dir.path().join("bridge").join("core.sock");
        let server = tokio::spawn(serve_local(engine.clone(), path.clone()));
        for _ in 0..100 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        private_socket(&path)?;
        let result = call(
            path.clone(),
            token.clone(),
            common("capabilities", workspace),
        )
        .await?;
        assert_eq!(result["workspaces"], json!([workspace]));
        assert!(
            call(
                path.clone(),
                token.clone(),
                common("capabilities", Uuid::new_v4())
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("SCOPE_DENIED")
        );
        assert!(
            call(
                path.clone(),
                token.clone(),
                json!({"operation":"confirm","workspace_id":workspace})
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("SCOPE_DENIED")
        );
        let mut privileged = common("capabilities", workspace);
        privileged["authority"] = "human".into();
        assert!(call(path.clone(), token.clone(), privileged).await.is_err());
        assert!(
            call(
                path.clone(),
                "x".repeat(64),
                common("capabilities", workspace)
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("SCOPE_DENIED")
        );
        // Real framing: duplicate keys and oversized lengths close the connection, never execute.
        for frame in [
            b"{\"token\":\"x\",\"token\":\"y\",\"command\":{}}".to_vec(),
            vec![],
        ] {
            let mut stream = tokio::net::UnixStream::connect(&path).await?;
            stream
                .write_u32(if frame.is_empty() {
                    MAX_REQUEST_BYTES as u32 + 1
                } else {
                    frame.len() as u32
                })
                .await?;
            if !frame.is_empty() {
                stream.write_all(&frame).await?;
            }
            assert!(
                tokio::time::timeout(Duration::from_secs(1), stream.read_u32())
                    .await?
                    .is_err()
            );
        }
        let mut rank: Value =
            serde_json::from_str(include_str!("../../../contracts/fixtures/rank-valid.json"))?;
        rank["workspace_id"] = json!(workspace);
        rank["request_id"] = json!(Uuid::new_v4());
        let proposal = json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":workspace,"family_id":Uuid::new_v4(),"case_kind":"actual","domain":rank["domain"],"context":rank["context"],"candidates":rank["candidates"],"response":{"response_type":"choose_one","candidate_id":rank["candidates"][0]["id"]},"source":{"artifact_id":Uuid::new_v4(),"source_kind":"agent_proposal","occurred_at":null,"observed_at":"2026-09-21T10:00:00Z"},"model_exposure":false});
        let proposed = call(
            path.clone(),
            token.clone(),
            json!({"operation":"propose","proposal":proposal}),
        )
        .await?;
        assert_eq!(proposed["status"], "pending");
        engine.lock().unwrap().execute(json!({"operation":"confirm","workspace_id":workspace,"id":proposed["id"],"expected_revision":proposed["revision"],"request_id":Uuid::new_v4()}),Authority::Human)?;
        let mut search = common("find", workspace);
        search["query"] = rank["context"]["summary"].clone();
        search["limit"] = 5.into();
        let found = call(path.clone(), token.clone(), search).await?;
        assert_eq!(found["records"].as_array().unwrap().len(), 1);
        let ranked = call(
            path.clone(),
            token.clone(),
            json!({"operation":"rank","request":rank}),
        )
        .await?;
        assert_eq!(ranked["execution_authorization"], "none");
        assert!(
            call(path.clone(), token.clone(), common("gaps", workspace))
                .await
                .unwrap_err()
                .to_string()
                .contains("SCOPE_DENIED")
        );
        engine.lock().unwrap().store.revoke_grant(id)?;
        assert!(
            call(
                path.clone(),
                token.clone(),
                common("capabilities", workspace)
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("SCOPE_DENIED")
        );
        engine.lock().unwrap().store.lock();
        assert!(
            call(path.clone(), token, common("capabilities", workspace))
                .await
                .unwrap_err()
                .to_string()
                .contains("VAULT_LOCKED")
        );
        server.abort();
        let _ = server.await;
        assert!(!path.exists());
        Ok(())
    }
    #[tokio::test]
    async fn never_overwrite_existing_path() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (engine, _, _, _) = fixture(dir.path());
        let parent = dir.path().join("bridge");
        private_directory(&parent, true)?;
        let path = parent.join("core.sock");
        fs::write(&path, b"preserve")?;
        assert!(serve_local(engine, path.clone()).await.is_err());
        assert_eq!(fs::read(path)?, b"preserve");
        Ok(())
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn policy_bridge_requires_read_scope_and_denies_mutation() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let (engine, old_token, workspace, _) = fixture(dir.path());
        let token = "c".repeat(64);
        let grant_id = Uuid::new_v4();
        let policy = json!({"statement":"Prefer repair when there is a deadline","when":{"op":"exists","field":"deadline_days"},"requires":{"op":"compare","field":"delivery_strategy","comparison":"eq","value":"repair_first","unit":null},"effective_from":"2020-01-01T00:00:00Z","expires_at":null});
        let approved = {
            let mut e = engine.lock().unwrap();
            e.store.create_grant(cdna_store::Grant {
                id: grant_id,
                workspace_id: workspace,
                scopes: vec!["policy:read".into()],
                expires_at: chrono::Utc::now().timestamp() + 300,
                revoked: false,
                token_hash: format!("{:x}", Sha256::digest(token.as_bytes())),
            })?;
            let doc = e.execute(
                json!({"operation":"propose_policy","workspace_id":workspace,"policy":policy}),
                Authority::Human,
            )?;
            e.execute(json!({"operation":"approve_policy","workspace_id":workspace,"id":doc["id"],"expected_revision":doc["revision"]}),Authority::Human)?
        };
        let path = dir.path().join("policy-bridge").join("core.sock");
        let server = tokio::spawn(serve_local(engine.clone(), path.clone()));
        for _ in 0..100 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut request: Value =
            serde_json::from_str(include_str!("../../../contracts/fixtures/rank-valid.json"))?;
        request["workspace_id"] = json!(workspace);
        let command = json!({"operation":"check_policy","request":request});
        assert!(
            call(path.clone(), old_token, command.clone())
                .await
                .unwrap_err()
                .to_string()
                .contains("SCOPE_DENIED")
        );
        let checked = call(path.clone(), token.clone(), command.clone()).await?;
        assert_eq!(checked["candidates"][0]["status"], "compliant");
        assert_eq!(checked["candidates"][1]["status"], "violated");
        assert_eq!(checked["execution_authorization"], "none");
        let listed = call(path.clone(), token.clone(), common("policies", workspace)).await?;
        assert_eq!(listed["policies"].as_array().unwrap().len(), 1);
        let mut other = command.clone();
        other["request"]["workspace_id"] = json!(Uuid::new_v4());
        assert!(
            call(path.clone(), token.clone(), other)
                .await
                .unwrap_err()
                .to_string()
                .contains("SCOPE_DENIED")
        );
        assert!(call(path.clone(),token.clone(),json!({"operation":"revoke_policy","workspace_id":workspace,"id":approved["id"],"expected_revision":approved["revision"]})).await.unwrap_err().to_string().contains("SCOPE_DENIED"));
        engine.lock().unwrap().store.revoke_grant(grant_id)?;
        assert!(
            call(path.clone(), token, command)
                .await
                .unwrap_err()
                .to_string()
                .contains("SCOPE_DENIED")
        );
        server.abort();
        let _ = server.await;
        Ok(())
    }
}
