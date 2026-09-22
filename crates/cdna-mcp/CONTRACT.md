# MCP transport boundary

`pub type Dispatcher = Arc<dyn Fn(serde_json::Value) -> Result<serde_json::Value, String> + Send + Sync>`

`pub async fn serve_stdio(dispatch: Dispatcher) -> anyhow::Result<()>`

`McpServer::new(dispatch)` implements the official rmcp ServerHandler for embedded tests.
The parent callback must derive Agent authority from a locally registered grant on every call,
check workspace, scope, expiry, revocation, vault lock and recheck before returning data.
No authority is accepted from MCP arguments. Parent owns durable audit and idempotency.

Mappings: get_capabilities => capabilities; get_profile => profile;
find_decisions => find; propose_observation => propose (proposal wrapper);
rank_options => rank (request wrapper); check_policy => check_policy;
get_learning_gaps => gaps; enqueue_questions => enqueue;
propose_answer => propose_answer; get_job => get_job; cancel_job => cancel_job.
All tool names have `cdna_` prefix. Unsupported parent operations must return errors.
Public non-domain arguments use schema_version/request_id/workspace_id, with query/limit
for find, and job_id for job operations. Missing backend contracts are explicitly unavailable.
Requests are size bounded, closed typed contracts; domain semantics are checked before callback.
Each server has a shared 60/minute limiter with burst 10. Callback is bounded by 30 seconds;
it must itself bound synchronous work and transaction duration because blocking cancellation
cannot stop a transaction. All diagnostics belong on stderr, never stdout.

The stdio wire frame (including the JSON-RPC envelope) is capped at 256 KiB by
rmcp's official codec; malformed or oversized frames close the connection.
At most two callbacks may run simultaneously; a timed-out callback retains its
slot until it actually finishes. Success uses structuredContent and a closed
`{ok,result}` envelope; operation-specific result validation remains the parent's
responsibility. No MCP Tasks capability is advertised. Job tools address only
parent-owned jobs and require owner authorization there.

Verified SDK reference: https://docs.rs/rmcp/3.4.0/rmcp/
