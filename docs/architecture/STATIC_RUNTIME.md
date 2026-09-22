# Static browser execution decision

The public playground runs without an application server. Cloudflare serves static
files; no Pages Function, Worker API, remote inference endpoint, account secret or
cloud model key is needed. Browser memory is an ephemeral playground, not the
native encrypted vault. Closing/resetting the runtime destroys its worker memory.
The frontend owns visible memory lifecycle and export disclosure.

## Reuse, not a second learner

The small `cdna-browser` WASM facade calls the existing Rust domain
`ObservationProposal::from_json` and `RankRequest::from_json`. Duplicate JSON keys,
limits, ratio units and schema semantics therefore retain their native authority.

Training and ranking execute the **same bytes** as
`learner/src/cdna_learner/worker.py` under Pyodide. `scripts/prepare.py` copies the
source and records SHA-256 in the generated manifest. The browser wrapper only
validates request size, disables optional LightGBM, applies a 500-record ceiling,
and dispatches the existing Python `Request`/`run`. It does not implement a second
optimizer, feature extractor, calibration calculation or split algorithm.

Mojo is a native comparison runtime; browser execution does not pretend to run it.
Rust is reused for domain validation, while Python/scikit-learn performs actual
browser WASM learning and ranking.

## Pinned compatibility exception

Official stable Pyodide **314.0.7** supplies CPython **3.14.2** and supported
WebAssembly wheels: scikit-learn **1.8.0**, NumPy **2.4.6**, SciPy **1.18.0**,
Pydantic **2.12.5** / pydantic-core **2.41.5**. These are the latest packages in this
stable Pyodide distribution, not the newer native lock versions. Native binary
wheels cannot run on WebAssembly. The official supported stable package set is
preferred over an untested bespoke wheel build. Runtime version differences are
explicit, and model equality across native/browser versions is not asserted.
Source/feature/objective reuse and browser-specific tests establish the bounded
claim instead. LightGBM comparison remains a native developer operation.

wasm-bindgen **0.2.128** is pinned for both Rust dependency and generator; the
project's Rust **1.98.1** target is `wasm32-unknown-unknown`.

## Asset build and serving

1. Install the Rust WASM target for the pinned toolchain.
2. Use the development harness to build `cdna-browser` for the release WASM target.
3. Run `python3 crates/cdna-browser/scripts/install-bindgen.py` to obtain official
   wasm-bindgen CLI 0.2.128 in the crate-local `.tools` directory; `scripts/build-domain.sh` generates JS/WASM.
4. Run `python3 crates/cdna-browser/scripts/prepare.py`. It retrieves fixed-version
   official Pyodide assets with at most four concurrent requests and validates
   package hashes from the committed lock plus committed core-file hashes.
5. Copy `crates/cdna-browser/assets/` into the static site's `/runtime/` directory.
   These downloaded/generated artifacts stay ignored; reproducible manifests and
   scripts are committed. Never rewrite or hand-maintain the learner source copy.
6. Build the static frontend. Runtime modules refer only to same-origin assets.

The first download contains 15 runtime/package/source files totaling about 36.9 MB;
its largest asset is 14,029,750 bytes. Individual files are checked against the
25 MiB Cloudflare static-asset limit. The small domain WASM is additional. Runtime
initialization is lazy on training/preview, not first page paint. Progress events
are emitted as `cdna-runtime-progress`. One operation is allowed at a time; a
180-second deadline or `reset()` terminates the worker and rejects the request.
Cancellation uses termination rather than requiring shared-memory browser headers.

## JS contract

`crates/cdna-browser/runtime.ts` exports:

- `call('validate_proposal', proposal)` → normalized Rust-validated proposal.
- `call('validate_rank', request)` → normalized Rust-validated rank request.
- `call('train', nativeLearnerRequest)` → native learner response including model,
  splits and evaluation. Confirmed-record construction belongs to the frontend
  repository adapter; the learner also excludes unconfirmed/exposed records.
- `call('preview', {model, request})` → native learner ranking response, preserving
  raw-score/probability/abstention semantics.
- `reset()` → terminate worker, release runtime memory, reject pending request.

Inputs are JSON only, bounded at 8 MiB. No arbitrary Python execution API is
exported. Outputs are capped at 1 MiB. The runtime does not persist user data or
send it to a remote training service. Static asset downloads are network traffic;
this is not an offline-first claim before those files are cached.

## Verified and remaining

A real Node-hosted Pyodide WASM smoke trained the shared learner twice with opposite
confirmed choices; model weights and the winning candidate changed accordingly.
The same run loaded compiled Rust domain WASM and rejected wrong-unit and
duplicate-key requests. This verifies actual WASM computation, not UI interaction.
Browser rendering, worker lifecycle/cancellation, published asset loading and
network-request observation require the frontend/deployment acceptance checks.

Official references:

- [Pyodide stable workers](https://pyodide.org/en/stable/usage/webworker.html)
- [Pyodide supported packages](https://pyodide.org/en/stable/usage/packages-in-pyodide.html)
- [Pyodide static deployment](https://pyodide.org/en/stable/usage/downloading-and-deploying.html)
- [wasm-bindgen releases](https://github.com/wasm-bindgen/wasm-bindgen/releases)
