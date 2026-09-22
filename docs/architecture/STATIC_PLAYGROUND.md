# Public static playground

The public build uses `VITE_CDNA_STATIC=true` and the command adapter in
`apps/desktop/src/static/adapter.ts`. It requires no API server, Functions,
paid model API, analytics key or uploaded observations. Cloudflare serves only
static application and runtime assets. See `STATIC_RUNTIME.md` for packaging
the shared learner and domain validator.

The browser session is an ephemeral demonstration vault. Only hypothetical
cases are accepted. Choosing and confirming an answer records a real interaction
with that hypothetical case; it does not establish the identity of the person
answering or prove accuracy on their real decisions. The browser stores no
persistent database. Reloading discards data; locking clears the session and
terminates its learning worker. JavaScript memory clearing is not a claim of
cryptographic erasure. Export is an explicit local download initiated by the UI.
Do not enter secrets or personal case information into the public demonstration.

Each workspace has a random UUID, with at most 16 workspaces and 500 current
observations across the session. Requests are bounded to 256 KiB. Lists and
exports use 100-record pages. Revision checks prevent stale changes. Request
identities make retries idempotent and reject conflicting reuse. Deleted records
cannot be revived by replaying old requests. The session permits at most 5,000
retained write requests and 8 MiB of serialized mutation history, preventing
unbounded retry history growth. Deleted identity tombstones are bounded to 5,000.

Domain proposal and ranking validation executes the same Rust domain code in
WASM. Training executes the shared Python learner inside a Web Worker using
self-hosted Pyodide. It is limited to one training job; confirmed, non-exposed,
non-AI-generated observations form the snapshot. Every data mutation invalidates
models. A generation check rejects results from a snapshot changed during
training, and lock terminates the worker. Models remain provisional. Experimental
preview uses shared learner scoring, returns no calibrated probability and no
execution authorization. Activation remains unavailable without independent
acceptance evidence. Memory ranking matches confirmed cases and abstains on
missing information or conflicting evidence; it does not invent model scores.

`apps/desktop/public/_headers` defines same-origin runtime loading, CSP,
frame embedding restrictions, MIME sniffing protection and disabled sensor
permissions. Runtime resources are static same-origin fetches. There is no
cross-origin data transmission or server-side secret. The static build must be
checked against Cloudflare's per-asset size limits after runtime packaging.
Local builds and source checks do not constitute successful public deployment
or real-person accuracy acceptance.
