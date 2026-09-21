# C-DNA development contract

Read `exec_plan/C-DNA_Requirements_v1.0.0_2026-09-21.md` before changing behavior. Preserve existing documentation, artwork, and the AGPL-3.0 license.

Deliver working vertical paths, not disconnected mock screens. Keep Rust as the only vault writer. Never treat imported or agent-generated content as owner-confirmed. Never turn raw scores into probabilities. Predictions never authorize business execution.

All public fixtures must be wholly fictional. Do not copy private conversations, credentials, personal characteristics, or real client/company identifiers into source, tests, screenshots, logs, or artifacts. Browser demonstration data and personal vault data must never mix.

Validate boundaries: workspace authorization, confirmation, revisions, idempotency, cancellation, deletion epoch, finite numeric inputs, and immutable training snapshots. Corrections and deletions invalidate derived models; negative examples do not undo training.

Use existing libraries. Keep core, learner, UI, MCP, and provider adapters separate without unnecessary services. No arbitrary shell command or file-access API exposed to agents. No network request in local-only mode.

Report actual command results and environments. `blocked` and `not_run` are not passes. Unit tests are not proof of actual Codex/Claude sessions, Apple signing, notarization, clean-device installation, Mojo performance, or personal decision accuracy. Do not weaken acceptance criteria merely to obtain green checks. Keep the requirements/acceptance ledger current.
