# withAI: human answers, engine and external LLM teaching loop

The product premise is **human answers + the existing learning engine + Codex or
Claude Code as the teacher**. The teacher asks questions, identifies uncertainties,
and proposes changes; the human supplies the answer and its reasoning. This slice
implements the public MCP/backend transport and safe proposal lifecycle. It does
not establish teaching quality, representation distillation quality or measured
accuracy improvements; those require the separate teaching-loop design and real
user evaluation. There is no second inference engine or agent execution framework.

## Run with an official external client

Build/install `cdna` and place it on PATH, initialize a real vault using the normal
CLI, then create a short-lived dedicated grant for the intended workspace:

```sh
cdna --vault ./vault grant --workspace WORKSPACE_UUID --scopes profile:read,learning:read,question:propose --hours 1
cdna --vault ./vault serve
```

Keep `serve` running as the unlocked private local backend. `cdna mcp` only bridges
to that backend; it never opens the vault. The returned grant UUID identifies an OS
keychain credential; no bearer token is copied into the client configuration.
Choose one official client registration from the same working directory:

```sh
codex mcp add cdna -- cdna --vault ./vault mcp --grant GRANT_UUID
claude mcp add --transport stdio cdna -- cdna --vault ./vault mcp --grant GRANT_UUID
```

Paths above are relative to the client's launch directory; keep it stable.
These command shapes follow the official [Codex MCP documentation](https://developers.openai.com/codex/mcp)
and [Claude Code MCP documentation](https://code.claude.com/docs/en/mcp), checked
2026-09-22. The external client authenticates to its own provider. The engine never
extracts provider credentials or invokes either agent CLI. Granting the dedicated
scopes authorizes disclosure of the bounded TRAIN evidence to that client; review
the chosen client's provider/data settings before using private real data.

Do not add `decision:read`, `decision:propose`, or shell execution permissions to a
teaching-only client. Those are separate general-purpose capabilities and are not
needed by this workflow. In particular, the dedicated grant cannot call broad
historical decision search or write observation labels.

## Public tools and human commands

`cdna_get_profile` accepts `schema_version: "1.0"`, `request_id`, and `workspace_id`.
It requires `profile:read` and `learning:read`. It returns the current vault epoch,
latest current model ID and at most 32 confirmed evidence records whose families
are explicitly in that model's `split_family_ids.train` and whose revision matches
its lineage. Model-exposed and AI-generated observations are excluded. Without a
current model it returns an empty evidence list, allowing a clearly ungrounded
cold-start question proposal. For existing confirmed answers, the owner can call
`with_ai_bootstrap` (the existing human `train` action alias, with optional domain)
to establish a current family split before any label is disclosed. Validation/test family labels, complete model
artifacts, source documents and personality assessments are never returned by this
tool. Personality remains null, including when assessments exist.

For initial ChatGPT/Claude history, use the separate [portable bootstrap prompt](../prompts/bootstrap-profile-ja.md)
and human bootstrap import/correction/sharing CLI. `bootstrap_hypotheses` is a
separate, explicitly shared Source channel available even before a model exists.
Its `ai_generated_hypothesis`, `training_eligible: false`, and
`heldout_eligible: false` markers remain unchanged by human correction. The raw
source artifact is not exported; only its bounded typed hypothesis profile is
returned with source ID/revision. Local-only, revoked, deleted and other-workspace
profiles are excluded. Source revision/sharing changes invalidate existing drafts.

`cdna_enqueue_questions` additionally requires `question:propose`. Its discovered
closed schema accepts one batch containing:

- `schema_version`, `request_id`, `workspace_id`, the profile's `epoch`, and provider
  `codex` or `claude_code` (an untrusted attribution, not authenticated identity).
- `evidence_refs`: up to 32 `{record_id, revision}` references from that profile.
- Nonempty `observed_issue`, `hypothesis`, `proposed_change`, `evaluation_plan`,
  `risks`, and `rollback`, each at most 4096 bytes.
- `measured_effect: null`. The teacher cannot claim independently measured effects.
- `questions`: 1–8 existing `QuestionCandidate` objects. Each has UUID `id` and
  `family_id`, a valid same-workspace `RankRequest` in `question`,
  `synthetic_scenario: true`, `source: "llm_proposal"`, category `gap`,
  `representative`, or `exploration`, and `estimated_answer_seconds` from 1–600.
  `owner_declared_importance` and `observed_frequency` must be null. The request
  cannot contain a response, authority, confirmation, shell action or trusted source.

Every question also needs a matching `question_origins` entry:
`{question_id, derived_from: null}` explicitly declares an independent case;
`{question_id, derived_from: {record_id, revision}}` declares a derivative of
allowlisted TRAIN evidence. For bootstrap hypotheses use
`{question_id, derived_from: null, derived_source: {source_id, revision}}` and set
`family_id` to the shared source ID. Both origin kinds cannot be specified together.
All questions derived from one bootstrap profile conservatively share one family;
they are not counted as independent anchors. Their human answers link to that
source, so source deletion also erases the descendant observations. Derivatives must retain that evidence's family and
retain its source artifact/exposure when the human answers. Independent families
must be new relative to current observation history; an exact copied case cannot
be relabeled as independent. Families must be distinct within a batch. Semantic
paraphrases cannot be proven independent mechanically and require human review. The engine validates the existing question scheduler contract;
it does not claim that the teacher's question is neutral or its hypothesis is true.
Prose is untrusted data. Human review must evaluate leading wording and relevance.

The entire batch is one atomic encrypted `Coaching` document keyed by `request_id`.
Retry an identical request after an ambiguous response; do not generate another ID.
Reusing the same ID with changed content fails. There are at most 32 drafts per
workspace, 8 questions per batch and the existing 256 KiB request limit. MCP has
its existing request rate/concurrency limits; the private bridge also limits
proposal bursts to 10 and replenishes one token per second, with a bounded map.
Durable grant expiry/revocation and workspace are checked again on each request and
before output disclosure. Unsupported tools still return errors, never fake success.

The owner uses `cdna --vault ./vault exec` with JSON stdin (or the same authenticated
human backend command endpoint). These are backend actions, not a required UI:

```json
{"operation":"with_ai_list","workspace_id":"WORKSPACE_UUID"}
```

```json
{"operation":"with_ai_adopt","workspace_id":"WORKSPACE_UUID","id":"BATCH_REQUEST_UUID","epoch":42}
```

```json
{"operation":"with_ai_respond","workspace_id":"WORKSPACE_UUID","id":"BATCH_REQUEST_UUID","question_id":"QUESTION_UUID","request_id":"ANSWER_REQUEST_UUID","response":{"response_type":"choose_one","candidate_id":"a"},"rationale_explicit":"I prioritize the delivery deadline here","reversal_conditions":["Delay if safety verification is incomplete"],"model_exposure":false}
```

Adoption means accepting the draft for questioning; it does not approve code,
execute an improvement plan, manufacture labels or promote a model. Responding is
human-only, creates a **pending** observation through the existing `propose` action,
and retains the question family, question ID (or derivative origin artifact) as source, hypothetical case
kind, human rationale/reversal conditions and declared exposure. Set
`model_exposure: true` if an engine's suggested answer was shown; the existing
training snapshot excludes it. All existing response semantics remain available,
including skip, defer, insufficient information and conditional acceptability.

The owner then uses existing `confirm` (returned observation ID, expected revision,
new request ID), followed by `train` (workspace and optional domain). Training remains
provisional. Independent evaluation and the existing signed promotion gate remain
necessary. The external teacher cannot perform those actions using its grant.

## Invalidation and deletion

Draft creation and adoption do not advance the input epoch or purge the active
model/evolution state. Observation creation, correction, confirmation, deletion,
source erasure, and other input changes advance the epoch and physically purge
copied draft text. Restore always drops coaching documents. Input epochs are
vault-wide, so draft invalidation also covers another workspace's input change.
An old batch cannot be adopted or submitted again after an input change.

A human answer is itself an input change: it consumes the current draft snapshot
and invalidates the rest of that batch and other drafts. For another answer, refresh
the profile and propose a new batch against the new epoch. An identical human
answer retry uses an atomic metadata-only receipt bound to batch/question/request
IDs and returns the already-written observation without resurrecting a draft.
Receipts cascade with record deletion and contain no copied source text.
This deliberately requires refreshing potentially stale teacher hypotheses.

## Evidence and remaining acceptance

`crates/cdna-app/tests/with_ai.rs` covers real encrypted Store + Engine + private
bridge, official SDK discovery/call routing, proposal/adopt/human answer/confirm/
training, exact request retry, denied scope/workspace, malformed batch atomicity,
AI authority rejection, TRAIN allowlisting, consent-bound no-model bootstrap hypotheses and descendant source erasure, raw-source non-disclosure, input
revision/deletion/source erasure/restore purge, and model survival during drafts.
The existing MCP SDK and private socket suites cover framing, locking and grant
revocation. These are local automated checks, not a live Codex/Claude Code session
or proof of LLM question quality, user benefit, or improved model accuracy.

The existing tree-sitter graph found Store invalidation call candidates; its Rust
trace reported `language_not_evaluated` and unverified freshness. Source inspection
therefore established mutation/restore coverage, rather than claiming graph
completeness. Root final freeze/testing is required after concurrent edits settle.

### Local implementation checkpoint (2026-09-22)

The focused command `cargo test -p cdna-app --test with_ai -p cdna-mcp --lib`
completed with 36 app unit tests, 11 withAI/private-bridge tests and 3 MCP tests
passing. The preceding lineage run also passed all 17 Store tests. The required
RevHarness wrapper reported exit 50 because concurrent source changes were detected;
these are successful child test runs, not a clean final frozen-tree acceptance.
The coordinator must run the final combined gate after all writers settle. Real
Codex/Claude Code connection, OS keychain prompts and human teaching outcomes were
not exercised by these automated tests.
