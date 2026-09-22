//! Official SDK stdio transport. Application authorization remains in the supplied dispatcher.
use cdna_domain::{MAX_REQUEST_BYTES, ObservationProposal, RankRequest, SourceKind};
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt, model::*, service::RequestContext};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

pub type Dispatcher = Arc<dyn Fn(Value) -> Result<Value, String> + Send + Sync>;
#[derive(Clone)]
pub struct McpServer {
    dispatch: Dispatcher,
    rate: Arc<Mutex<Rate>>,
    slots: Arc<tokio::sync::Semaphore>,
}
struct Rate {
    tokens: f64,
    updated: Instant,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Common {
    schema_version: String,
    request_id: Uuid,
    workspace_id: Uuid,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Find {
    schema_version: String,
    request_id: Uuid,
    workspace_id: Uuid,
    query: String,
    #[serde(default = "five")]
    limit: u8,
}
fn five() -> u8 {
    5
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Job {
    schema_version: String,
    request_id: Uuid,
    workspace_id: Uuid,
    job_id: Uuid,
}
/// External clients propose a bounded batch; no answer or execution authority exists here.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub record_id: Uuid,
    pub revision: u64,
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceEvidenceRef {
    pub source_id: Uuid,
    pub revision: u64,
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QuestionOrigin {
    pub question_id: Uuid,
    pub derived_from: Option<EvidenceRef>,
    pub derived_source: Option<SourceEvidenceRef>,
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnqueueRequest {
    pub schema_version: String,
    pub request_id: Uuid,
    pub workspace_id: Uuid,
    pub epoch: u64,
    pub provider: String,
    #[schemars(length(max = 32))]
    pub evidence_refs: Vec<EvidenceRef>,
    #[schemars(length(min = 1, max = 4096))]
    pub observed_issue: String,
    #[schemars(length(min = 1, max = 4096))]
    pub hypothesis: String,
    #[schemars(length(min = 1, max = 4096))]
    pub proposed_change: String,
    #[schemars(length(min = 1, max = 4096))]
    pub evaluation_plan: String,
    #[schemars(length(min = 1, max = 4096))]
    pub risks: String,
    #[schemars(length(min = 1, max = 4096))]
    pub rollback: String,
    pub measured_effect: Option<serde_json::Value>,
    #[schemars(length(min = 1, max = 8))]
    pub questions: Vec<serde_json::Value>,
    #[schemars(length(min = 1, max = 8))]
    pub question_origins: Vec<QuestionOrigin>,
}
fn enqueue_schema() -> serde_json::Map<String, Value> {
    let mut schema = schema::<EnqueueRequest>();
    let mut rank = serde_json::to_value(schemars::schema_for!(RankRequest)).unwrap();
    let definitions = rank.as_object_mut().unwrap().remove("$defs");
    if let Some(definitions) = definitions {
        schema
            .entry("$defs")
            .or_insert(json!({}))
            .as_object_mut()
            .unwrap()
            .extend(definitions.as_object().unwrap().clone());
    }
    schema["properties"]["schema_version"] = json!({"const":"1.0"});
    schema["properties"]["measured_effect"] = json!({"type":"null"});
    schema["properties"]["provider"] = json!({"type":"string","enum":["codex","claude_code"]});
    schema["properties"]["questions"]["items"] = json!({"type":"object","additionalProperties":false,"required":["id","family_id","question","synthetic_scenario","source","category","estimated_answer_seconds"],"properties":{
        "id":{"type":"string","format":"uuid"},"family_id":{"type":"string","format":"uuid"},"question":rank,
        "synthetic_scenario":{"const":true},"source":{"const":"llm_proposal"},"category":{"enum":["gap","representative","exploration"]},
        "owner_declared_importance":{"type":"null"},"observed_frequency":{"type":"null"},"estimated_answer_seconds":{"type":"integer","minimum":1,"maximum":600}}});
    schema
}
const TOOLS: &[(&str, &str)] = &[
    ("cdna_get_capabilities", "capabilities"),
    ("cdna_get_profile", "profile"),
    ("cdna_find_decisions", "find"),
    ("cdna_propose_observation", "propose"),
    ("cdna_rank_options", "rank"),
    ("cdna_check_policy", "check_policy"),
    ("cdna_get_learning_gaps", "gaps"),
    ("cdna_enqueue_questions", "enqueue"),
    ("cdna_propose_answer", "propose_answer"),
    ("cdna_get_job", "get_job"),
    ("cdna_cancel_job", "cancel_job"),
];
fn schema<T: JsonSchema>() -> serde_json::Map<String, Value> {
    serde_json::to_value(schemars::schema_for!(T))
        .expect("schema")
        .as_object()
        .expect("object schema")
        .clone()
}
fn tool(name: &str, op: &str) -> Tool {
    let s = match op {
        "rank" | "check_policy" => schema::<RankRequest>(),
        "propose" => schema::<ObservationProposal>(),
        "find" => schema::<Find>(),
        "enqueue" => enqueue_schema(),
        "get_job" | "cancel_job" => schema::<Job>(),
        _ => schema::<Common>(),
    };
    let mut t = Tool::new(
        name.to_owned(),
        format!("C-DNA {op}; server-side grant required; proposals never confirm human authority."),
        s,
    );
    t.output_schema=Some(Arc::new(json!({"type":"object","additionalProperties":false,"required":["ok","result"],"properties":{"ok":{"const":true},"result":{}}}).as_object().unwrap().clone()));
    t
}
fn invalid(s: impl Into<String>) -> ErrorData {
    ErrorData::invalid_params(s.into(), None)
}
fn decode<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, ErrorData> {
    serde_json::from_value(v).map_err(|_| invalid("INPUT_INVALID: closed input contract violation"))
}
fn version(v: &str) -> Result<(), ErrorData> {
    if v == "1.0" {
        Ok(())
    } else {
        Err(invalid("SCHEMA_UNSUPPORTED"))
    }
}
impl McpServer {
    pub fn new(dispatch: Dispatcher) -> Self {
        Self {
            dispatch,
            rate: Arc::new(Mutex::new(Rate {
                tokens: 10.,
                updated: Instant::now(),
            })),
            slots: Arc::new(tokio::sync::Semaphore::new(2)),
        }
    }
    fn take(&self) -> Result<(), ErrorData> {
        let mut r = self.rate.lock().map_err(|_| invalid("INTERNAL_ERROR"))?;
        let now = Instant::now();
        r.tokens = (r.tokens + now.duration_since(r.updated).as_secs_f64()).min(10.);
        r.updated = now;
        if r.tokens < 1. {
            return Err(invalid("RATE_LIMITED"));
        }
        r.tokens -= 1.;
        Ok(())
    }
    async fn invoke(&self, name: &str, args: Value) -> Result<CallToolResult, ErrorData> {
        self.take()?;
        let op = TOOLS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, o)| *o)
            .ok_or_else(|| invalid("unknown tool"))?;
        let bytes = serde_json::to_vec(&args).map_err(|_| invalid("INPUT_INVALID"))?;
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(invalid("INPUT_INVALID: request exceeds byte limit"));
        }
        let command = match op {
            "rank" | "check_policy" => {
                let r = RankRequest::from_json(&bytes).map_err(|e| invalid(e.to_string()))?;
                if r.allow_cloud {
                    return Err(invalid("CONSENT_REQUIRED: cloud unavailable over MCP"));
                }
                json!({"operation":op,"request":r})
            }
            "propose" => {
                let p =
                    ObservationProposal::from_json(&bytes).map_err(|e| invalid(e.to_string()))?;
                if !matches!(
                    p.source.source_kind,
                    SourceKind::AgentProposal | SourceKind::AiGenerated
                ) {
                    return Err(invalid("SCOPE_DENIED: untrusted source authority"));
                }
                json!({"operation":op,"proposal":p})
            }
            "enqueue" => {
                let request: EnqueueRequest = decode(args)?;
                version(&request.schema_version)?;
                if !(1..=8).contains(&request.questions.len())
                    || request.evidence_refs.len() > 32
                    || request.measured_effect.is_some()
                {
                    return Err(invalid("INPUT_INVALID: proposal bounds"));
                }
                json!({"operation":"enqueue","request":request})
            }
            "find" => {
                let f: Find = decode(args.clone())?;
                version(&f.schema_version)?;
                if f.query.is_empty() || f.query.len() > 4096 || f.limit == 0 || f.limit > 20 {
                    return Err(invalid("INPUT_INVALID: search limit"));
                }
                json!({"operation":op,"schema_version":f.schema_version,"request_id":f.request_id,"workspace_id":f.workspace_id,"query":f.query,"limit":f.limit})
            }
            "get_job" | "cancel_job" => {
                let j: Job = decode(args)?;
                version(&j.schema_version)?;
                json!({"operation":op,"schema_version":j.schema_version,"request_id":j.request_id,"workspace_id":j.workspace_id,"job_id":j.job_id})
            }
            _ => {
                let c: Common = decode(args)?;
                version(&c.schema_version)?;
                json!({"operation":op,"schema_version":c.schema_version,"request_id":c.request_id,"workspace_id":c.workspace_id})
            }
        };
        // These contracts are intentionally unavailable until a validated backend exists.
        if op == "propose_answer" {
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                "PROVIDER_UNAVAILABLE: operation not implemented",
            )]));
        }
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| invalid("BUDGET_EXCEEDED: concurrent request limit"))?;
        let dispatch = self.dispatch.clone();
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                (dispatch)(command)
            }),
        )
        .await;
        let envelope = match result {
            Ok(Ok(Ok(value))) => json!({"ok":true,"result":value}),
            Ok(Ok(Err(message))) => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(
                    json!({"ok":false,"error":{"code":"OPERATION_FAILED","message":message}})
                        .to_string(),
                )]));
            }
            _ => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(
                    "INTERNAL_ERROR: operation unavailable or deadline exceeded",
                )]));
            }
        };
        let output = envelope.to_string();
        if output.len() > MAX_REQUEST_BYTES {
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                "BUDGET_EXCEEDED: response byte limit",
            )]));
        }
        Ok(CallToolResult::structured(envelope))
    }
}
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: TOOLS.iter().map(|(n, o)| tool(n, o)).collect(),
            ..Default::default()
        })
    }
    async fn call_tool(
        &self,
        p: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.invoke(&p.name, Value::Object(p.arguments.unwrap_or_default()))
            .await
            .map(Into::into)
    }
}
pub async fn serve_stdio(dispatch: Dispatcher) -> anyhow::Result<()> {
    use futures_util::StreamExt;
    use rmcp::{
        service::{RxJsonRpcMessage, TxJsonRpcMessage},
        transport::async_rw::JsonRpcMessageCodec,
    };
    use tokio_util::codec::{FramedRead, FramedWrite};
    let (input, output) = rmcp::transport::stdio();
    // The SDK's plain async-RW adapter defaults to an unlimited line buffer.
    // Use the SDK codec with an explicit wire-frame cap; malformed frames end the connection.
    let read = FramedRead::new(
        input,
        JsonRpcMessageCodec::<RxJsonRpcMessage<RoleServer>>::new_with_max_length(MAX_REQUEST_BYTES),
    )
    .scan((), |_, item| std::future::ready(item.ok()));
    let write = FramedWrite::new(
        output,
        JsonRpcMessageCodec::<TxJsonRpcMessage<RoleServer>>::new_with_max_length(MAX_REQUEST_BYTES),
    );
    McpServer::new(dispatch)
        .serve((write, read))
        .await?
        .waiting()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[tokio::test]
    async fn real_sdk_discovery_validation_and_dispatch() -> anyhow::Result<()> {
        let count = Arc::new(AtomicUsize::new(0));
        let calls = count.clone();
        let server = McpServer::new(Arc::new(move |v| {
            calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(v["operation"], "capabilities");
            Ok(json!({"schema_version":"1.0"}))
        }));
        let (a, b) = tokio::io::duplex(65536);
        let handle = tokio::spawn(async move {
            server.serve(a).await.unwrap().waiting().await.unwrap();
        });
        let client = ().serve(b).await?;
        let listed = client.list_tools(None).await?;
        assert_eq!(listed.tools.len(), 11);
        for t in listed.tools {
            assert_eq!(
                t.input_schema.get("additionalProperties"),
                Some(&Value::Bool(false))
            );
            assert!(!t.name.contains("confirm"));
        }
        let args = json!({"schema_version":"1.0","request_id":"00000000-0000-4000-8000-000000000001","workspace_id":"00000000-0000-4000-8000-000000000002"});
        let good = client
            .call_tool(
                CallToolRequestParams::new("cdna_get_capabilities")
                    .with_arguments(args.as_object().unwrap().clone()),
            )
            .await?;
        assert_ne!(good.is_error, Some(true));
        for flag in ["confirmed_by", "training_eligible", "scope", "authority"] {
            let mut bad = args.clone();
            bad[flag] = true.into();
            assert!(
                client
                    .call_tool(
                        CallToolRequestParams::new("cdna_get_capabilities")
                            .with_arguments(bad.as_object().unwrap().clone())
                    )
                    .await
                    .is_err()
            );
        }
        let mut bad = args.clone();
        bad["schema_version"] = "9.0".into();
        assert!(
            client
                .call_tool(
                    CallToolRequestParams::new("cdna_get_profile")
                        .with_arguments(bad.as_object().unwrap().clone())
                )
                .await
                .is_err()
        );
        let mut rank: Value =
            serde_json::from_str(include_str!("../../../contracts/fixtures/rank-valid.json"))?;
        rank["training_eligible"] = true.into();
        assert!(
            client
                .call_tool(
                    CallToolRequestParams::new("cdna_rank_options")
                        .with_arguments(rank.as_object().unwrap().clone())
                )
                .await
                .is_err()
        );
        rank.as_object_mut().unwrap().remove("training_eligible");
        rank["candidates"][1]["id"] = rank["candidates"][0]["id"].clone();
        assert!(
            client
                .call_tool(
                    CallToolRequestParams::new("cdna_rank_options")
                        .with_arguments(rank.as_object().unwrap().clone())
                )
                .await
                .is_err()
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
        client.cancel().await?;
        handle.await?;
        Ok(())
    }
    #[tokio::test]
    async fn burst_and_domain_validation() {
        let server = McpServer::new(Arc::new(|_| {
            panic!("invalid requests cannot reach application")
        }));
        let mut rank: Value =
            serde_json::from_str(include_str!("../../../contracts/fixtures/rank-valid.json"))
                .unwrap();
        rank["candidates"][1]["id"] = rank["candidates"][0]["id"].clone();
        assert!(server.invoke("cdna_rank_options", rank).await.is_err());
        for _ in 0..9 {
            assert!(server.invoke("unknown", json!({})).await.is_err());
        }
        assert!(server.take().unwrap_err().message.contains("RATE_LIMITED"));
    }
    #[tokio::test]
    async fn policy_tool_uses_domain_schema_and_dispatches_without_authority_flags()
    -> anyhow::Result<()> {
        let count = Arc::new(AtomicUsize::new(0));
        let calls = count.clone();
        let server = McpServer::new(Arc::new(move |v| {
            calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(v["operation"], "check_policy");
            assert_eq!(v["request"]["schema_version"], "1.0");
            Ok(json!({"mode":"policy_check","candidates":[],"execution_authorization":"none"}))
        }));
        let (a, b) = tokio::io::duplex(65536);
        let handle = tokio::spawn(async move {
            server.serve(a).await.unwrap().waiting().await.unwrap();
        });
        let client = ().serve(b).await?;
        let tools = client.list_tools(None).await?;
        let schema = &tools
            .tools
            .iter()
            .find(|t| t.name == "cdna_check_policy")
            .unwrap()
            .input_schema;
        assert!(schema["properties"].get("candidates").is_some());
        let args: Value =
            serde_json::from_str(include_str!("../../../contracts/fixtures/rank-valid.json"))?;
        let good = client
            .call_tool(
                CallToolRequestParams::new("cdna_check_policy")
                    .with_arguments(args.as_object().unwrap().clone()),
            )
            .await?;
        assert_ne!(good.is_error, Some(true));
        let mut invalid = args;
        invalid["scope"] = json!("policy:read");
        assert!(
            client
                .call_tool(
                    CallToolRequestParams::new("cdna_check_policy")
                        .with_arguments(invalid.as_object().unwrap().clone())
                )
                .await
                .is_err()
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
        client.cancel().await?;
        handle.await?;
        Ok(())
    }
}
