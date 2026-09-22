# Reusable measured improvement core

The `cdna-evolution` crate separates proposed model changes, independent evaluation, human approval and active selection. It has no database or executor. The application stores its state in the encrypted vault and commits a state change and active pointer together, using an expected revision. Its initial supported change is a 17-weight feature-version-1.0 model. Arbitrary code, feature dictionaries, permissions, cloud destinations and dependency changes are not accepted by this path.

## Evidence and trust

Candidate bytes are canonical JSON `ModelConfig`; their SHA256 binds the artifact and decoded configuration. A split manifest binds dataset UUID, workspace, input revision, deletion epoch, each case/family, input hash, timestamp, answer and domain. Families cannot span splits, and every training/validation/calibration case precedes final test cases. Test cases must have independent human answers and no model exposure. These declarations are authenticated by a host-trusted evaluator signing the complete evaluation receipt, including the manifest hash and paired results. A self-reported `promotion_eligible` boolean is not a contract field.

Evaluation verifies Ed25519 signatures, content hashes, exact paired case membership and identical input hashes. It recomputes agreement, coverage, Wilson lower bound, policy violations, p95 latency and peak memory. Initial absolute gates are 200 cases, 100 answered, 60% coverage, 90% agreement, 85% Wilson lower bound and no policy violations. Aggregate and per-domain comparisons must not regress against the same-input baseline; measured learning seconds must not increase. These are deliberately conservative gates, not an assertion that a statistically significant improvement was demonstrated. All measurements require positive measured resource values; absent measurements cannot be replaced with zero.

A separate human signing key authorizes the exact candidate hash, receipt hash, workspace, input revision and deletion epoch. The application must authenticate the owner and obtain an explicit activation decision before signing. An experiment authorization is not this activation approval. This crate does not provision or expose signing keys, authenticate a person, or independently observe whether the evaluator used genuine human answers. The host controls trusted keys, data access, final-test exposure accounting and evaluator isolation. Signatures protect provenance and integrity, not truthfulness of a compromised evaluator.

## State and deletion

The normal path is candidate → evaluated → approved → active. Replacing the active candidate retires it. A previously evaluated and approved retired artifact can be selected for rollback only within the same workspace/revision/epoch. Advancing revision or deletion epoch quarantines all candidates and clears the active pointer. Re-evaluation requires a fresh candidate and current data.

Serialized state cannot activate directly. After loading it, the host must call `validate_restored` with the store's current workspace/revision/epoch and trusted evaluator/human public keys. This recomputes evidence and signature checks. Failed validation leaves the active accessor unavailable. The host must also verify the model bytes against the approved hash when loading them for inference and enforce a bounded serialized-state size before deserialization.

## Integration and limits

The host signs evaluator receipts only after a real isolated paired evaluation and records final-test exposure durably; the `test_previously_exposed` signed declaration is not a substitute for that ledger. The dataset manifest describes all splits, but only train members are allowed in model fitting, with validation/calibration used for their declared purposes. External memory, prompts and feature preprocessing must obey the same cutoff. Resource measurements need a comparable harness and identical hardware/process conditions; signed timing values alone cannot ensure that.

Tests verify signature tampering, stale epochs, unsupported artifacts, wrong human/workspace, state restoration and rollback. They prove software behavior on synthetic fixtures, not personal predictive accuracy, production promotion readiness or superiority to another product. “Jev” remains an unidentified comparison target; this module makes no Jev performance claim. Other use cases may reuse the evidence and approval flow after supplying their own versioned feature contract and independently reviewed metric policy; agents cannot lower the current gates automatically.

Paired accuracy additionally uses the mean of per-case candidate-minus-baseline correctness indicators with a normal-approximation 95% lower bound and requires that bound to remain above -0.02. This is a non-inferiority screen on paired case outcomes, not a superiority test, causal estimate or permission to advertise an improvement. Domain-level coverage/agreement/resource checks remain mandatory separately.

## Explicit trusted-host CLI

`cdna evolution --evaluator-key evaluator-public.hex --human-key human-public.hex`
reads one structured JSON command from stdin, bounded to 1 MiB before decoding.
The key files contain public verification keys only: exactly 32 bytes encoded as
64 hex characters (surrounding whitespace is allowed).
The host pins these files; command JSON cannot replace either key. The two keys
must differ. No signing key is created, no approval is inferred from a boolean,
and no synthetic answer is converted into human evidence. This entry point is
for explicit local owner invocation and is not exposed through MCP or playground
commands.

The host adapter is `evolution_cli::run(&mut Store, Value, [u8; 32], [u8; 32])`.
It delegates every transition and cryptographic check to `EvolutionBridge` and
`cdna-evolution`. All commands require an existing `workspace_id`; the store epoch
supplies both the current `input_revision` and deletion epoch. This conservative
binding means any vault input mutation can stale prior evidence, even in another
workspace. Evolution state writes advance their own document revision without
advancing the input epoch. The adapter never rewrites signed evidence to make its
workspace, revision or epoch match.

Example (replace the UUID with an existing workspace):

```sh
printf '%s\n' '{"operation":"status","workspace_id":"00000000-0000-0000-0000-000000000001"}' |
  cdna evolution --evaluator-key evaluator-public.hex --human-key human-public.hex
```

Every mutation also requires `expected_revision` from the last successful result
(initially `0`). Unknown fields are rejected throughout typed command/evidence
objects. Operation-specific fields are:

| Operation | Additional fields | Result |
| --- | --- | --- |
| `status` | None; no `expected_revision` | Validated redacted summary |
| `register` | `candidate`, `artifact` (JSON byte array) | Hash-verified candidate |
| `evaluate` | `receipt` (signed evaluation receipt), `dataset` | Verified paired evaluation |
| `approve` | `approval` (signed human approval) | Separately authorized approval |
| `activate` | `candidate_id` | Explicit active selection |
| `rollback` | `candidate_id` | Previously approved retired selection |
| `invalidate` | None | Quarantine after host input epoch advances |

`candidate`, `dataset`, receipt payload and approval payload use the strict types
in `crates/cdna-evolution/src/lib.rs`. Signed values have the exact envelope
`{"payload":{...},"signature":[...]}`, with a 64-byte Ed25519 signature over
`cdna_evolution::signing_bytes(payload)`. The evaluator signs receipts and the
separate authenticated human signs approvals outside this application. Trust
files supplied to the CLI must be managed by that owner; choosing attacker-owned
files is not a valid production trust configuration.

Responses include only workspace/revision/epoch, active candidate ID and, for
candidate mutations, candidate ID/state. They omit case answers, receipt rows,
signatures, feature weights and domain names. Input changes in the same workspace purge its derived evolution document, so
`status` returns an empty state at revision 0 and new evidence is required.
A retained document made stale by another workspace changing the global epoch
fails closed, including `status`; use `invalidate` with the last known document
revision in that case. Invalidation authenticates the historical state before quarantine and
does not salvage corrupt evidence. These commands persist and select approved
artifacts. With host-pinned evolution trust, the application applies the active
artifact through `inference_runtime.rs` after domain, calibration, input-support,
policy and epoch checks. These commands do not automatically run training or
obtain human judgments. Synthetic tests establish adapter
and bridge behavior only, not real evaluation or human acceptance.
