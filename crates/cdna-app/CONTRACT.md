# Application boundary

`Engine` is the single application owner of `Store`. It exposes
`execute(command: serde_json::Value, authority: Authority) -> anyhow::Result<Value>`.
`Authority::Human` is only supplied by CLI / desktop user entrypoints.
External callers receive `Authority::Agent { workspace_id: Uuid, scopes: Vec<String> }`
derived from locally registered grants, never from request JSON.

Command envelope: `{ "operation": "...", ... }`.
Operations: `status`, `workspace_create` (name), `list` (workspace_id),
`propose` (proposal matching cdna-domain), `confirm` (workspace_id, id,
expected_revision, request_id), `revise` (workspace_id,id,expected_revision,
request_id,proposal), `delete` (workspace_id,id,expected_revision),
`rank` (request matching RankRequest), `train` (workspace_id),
`models` (workspace_id), `activate_model` (workspace_id,id),
`gaps` (workspace_id), `export` (workspace_id), `lock`.

All JSON results are wrapped by transports as `{ok:true,result:...}` or
`{ok:false,error:{code,message}}`. UI renders errors and preserves unsaved input.
`status` reports workspace IDs, lock state and implementation capabilities;
`list` returns records, each with id/revision/status/payload (the proposal).
Training returns provisional model and evaluation, never a validated-accuracy claim.
Rank returns ranking, evidence, abstention_reasons and execution_authorization none.
Exact confirmed memory takes precedence. With host-pinned evolution trust, an approved,
current signed model may rank unseen inputs in its supported numerical scope; only
calibrated two-choice outputs include pairwise probabilities and model raw scores. Demo data is explicitly hypothetical and uses its
own vault. Request inputs are bounded by the transport and domain validation.

The browser playground uses a loopback-only HTTP server with a startup-generated
session secret, Host/Origin validation and no public unauthenticated mutations.
Production desktop calls the same Engine through Tauri commands.

Construct with `Engine::new(store, learner, demo)`; learned trust is disabled by default.
A trusted host can apply `with_evolution_trust(evaluator_key, human_key)` using distinct
pinned public keys. No action JSON may supply keys. Owner `activate_model` requires a
registered, independently evaluated and explicitly human-signed evolution candidate;
legacy provisional models cannot activate. Every learned rank reloads and validates the
signed state against the current vault epoch, and rechecks state revision and epoch
before returning. Changes to confirmed sources, policies and deletion state invalidate
model use through existing Store invalidation.

CLI owner execution can configure `--evaluator-public-key FILE --human-public-key FILE`.
The separate `evolution --evaluator-key FILE --human-key FILE` adapter handles register,
evaluate, approve, activate, rollback and invalidate. `prepare_candidate` takes a current
provisional `model_id`, a bound `dataset` and explicit evaluator `inference` support;
it returns canonical unsigned candidate/artifact bytes without writing or promoting.
The numerical metadata and its signature semantics are in `../cdna-evolution/CONTRACT.md`.
