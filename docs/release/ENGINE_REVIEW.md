# Engine independent review — 2026-09-22

Scope: `benchmarks/engine/**` and this report only. Production changes belong to the coordinator and other workers. This report distinguishes historical failures from current independently rerun fixes; synthetic examples never establish a person's accuracy.

## Concrete findings, ranked by user impact

1. **P1, repaired and independently reproduced — same-time twentieth family erased learned preference.** Production learner `split` sent all 20/40/200 same-time families into test, leaving zero training rows and zero weights while returning `ok:true`. Nineteen families gave a positive margin 4.438; twenty gave 0. Equal timestamps are possible in imported/batch-confirmed history. `benchmarks/engine/learner_probes.py` and `results/learner-before.json` reproduce the cliff. A provisional fallback must preserve learning without inventing a temporal holdout.
2. **P1, repaired and independently reproduced — raw corpus size blocked usable training data.** `Engine::prepare_train` originally rejected any first page of 1,000 records before filtering eligibility. The 10,000 confirmed-row probe has only 20 unexposed eligible records and still rejected with `training snapshot limit reached`. Root implemented bounded current-confirmed paging; final independent rerun is recorded below. The resource cap on genuinely eligible records remains explicit.
3. **P1, repaired gates independently exercised — signed numerical claims needed independent-family and per-domain gates.** Source review found row-count Wilson/paired calculations accepted repeated rows from one family, while per-domain comparison omitted absolute quality and sample minima. A signature authenticates an evaluator's payload but does not make correlated rows independent. `src/bin/signed.rs` supplies the single-family and one-case-domain fixtures through the public signed pipeline. Final acceptance requires both rejected and the independent control accepted.
4. **P2, repaired and independently reproduced — normal-sized export could not be previewed.** One valid Claude conversation with 1,001 small human messages is rejected by public `import_sources` even with `preview:true`, with `source batch too large`. The underlying importer supports more messages, but the Engine envelope has no selection/paging input and imposes a 1,000-source ceiling. No partial import is silently claimed. Reproduction is embedded in the memory harness.
5. **P2, open — dense exact memory becomes expensive then abstains.** At 9,980 identical confirmed matches the bounded search reads 1,000 full payloads, and the Engine processes them before reporting `memory_search_limit_reached`. More repeated memories therefore raise latency without producing an answer. Returning a truncation result before evidence reconstruction, or a bounded conflict-aware aggregate, is a focused optimization; correctness must preserve hidden conflicting winners.

## Measurement scope and evidence

The initial memory run used Apple M2 Max, 12 logical CPUs, 96 GiB RAM, macOS 26.6.2 arm64, Rust 1.98.1 release. One child process created the real encrypted corpus using public propose/confirm. Two-reader scenarios use two OS threads and independent connections to the same database; no concurrent writer or transport server was tested. Process RSS/CPU include setup and cleanup. No OS page-cache eviction, per-stage allocations, or per-stage CPU measurement occurred.

Accepted isolated-snapshot measurements (200 warm samples, dense 100; microseconds):

| Path | p50 | p95 |
|---|---:|---:|
| Full Engine, 10k corpus, 20 matches, one approved policy, response JSON | 383.62 | 491.67 |
| Encrypted matching retrieval | 140.00 | 185.62 |
| Preloaded policy evaluation | 10.25 | 11.00 |
| Encrypted policy retrieval + evaluation | 37.08 | 38.67 |
| Structured feature extraction + linear scoring | 1.67 | 1.96 |
| Existing explanation evidence serialization only | 7.17 | 8.75 |
| Dense 9,980 matches, truncated abstention | 36,314.54 | 42,406.96 |
| Concurrent reader 0 | 536.17 | 723.54 |
| Concurrent reader 1 | 546.38 | 705.25 |
| Approved signed learned Engine rank, independent 200-family fixture | 3,685.25 | 3,920.04 |

`results/memory-path.json` is **historical/non-current**: production and benchmark files changed during the run, and the recorder correctly returned `source_or_binary_changed`. Its binary was unchanged, but source-to-binary provenance is not established. The table above instead uses the final immutable-copy run and its exact source/binary maps; it does not claim later shared-root changes were tested.

Stage probes are independent overlapping API calls, not additive instrumentation. The existing public API does not expose explanation construction as a separately timed stage: the explanation microbenchmark measures only evidence serialization; full rank contains construction. No latency for arbitrary natural language or model-only speed was inferred from structured scoring.

## Adversarial coverage and limitations

The memory harness checks confirmed conflicts cause abstention, deleting the conflicting observation restores a valid match, and a trailing-space change in an exact-memory query is not silently generalized. Source-deletion cascade was already fixed and was not reimplemented. Signed probes verify rotated evaluator keys and a changed input epoch cause abstention after a successful synthetic activation. They do not constitute an audit of real signer custody or a real human's independent evaluation data.

Tree-sitter codebase-memory search was used for `rank`, `prepare_train`, `check`, and `split`. The inbound `split` trace found `_fit_snapshot` directly and `train` transitively, but its wrapper returned `unverified` due to a Rust/Python short-name collision; source call sites were read directly. The existing graph was a candidate index, not freshness or complete call-graph proof.

Reproduce with `benchmarks/engine/README.md`. `verify.py` freezes file hashes around a locked build and both native runs; it must be run after writers finish. An unchanged worktree SHA alone cannot identify these concurrently uncommitted production files.

## Final verification

**Bounded GO for the captured engine artifact and these probes, not the evolving whole repository.** The coordinator subsequently assigned a withAI answer/question/hypothesis/retrain loop; that work is outside this snapshot and needs separate integrated verification.

- `benchmarks/engine/results/capture-binding.json`: original-to-isolated-copy hashes equal for the exact engine dependency sources and harness.
- `benchmarks/engine/results/build-binding.json`: locked offline release build exited 0, pre/post source hashes equal, both binary SHA-256 values recorded.
- `benchmarks/engine/results/cdna-engine-isolated-receipt.json`: required heavy guard acquired, command exit 0, `sourceChanged:false`.
- `benchmarks/engine/results/memory-final.json` and `signed-final.json`: `stable_artifact_snapshot`, unchanged binaries and source maps throughout each run.

The memory binary is `da0d06a501297c7a12e59a78d5d92ea528f306978a16d7426a78c5083f836af7`; the signed binary is `07f01dbfaf709ea03b73852b16c4e78fb754c12b5819be3871454ff8ec06aea7`. The source maps, rather than the temporary Git commit, identify the captured uncommitted production state. Current shared files may have changed after capture.

The 10,000-row corpus now yields exactly 20 eligible training rows. Import preview has no writes; two commit pages store exactly 1,001 sources; replay of page one inserts zero and skips 1,000. Conflicting answers abstain and deleting the conflicting answer restores the expected choice. Signed evaluation rejects `duplicate independent test family` and `insufficient independent quality`; the 200-family positive control activates and ranks. Rotated evaluator keys and a changed input epoch both abstain.

Whole memory-process measurement: 42.624 s wall, 26.710 s user+system CPU, 34,521,088-byte peak RSS, including durable corpus setup and ingestion probes. Reopening SQLCipher plus first rank took 80.383 ms with no cache eviction. Signed-process wall was 1.570 s and peak RSS 16,744,448 bytes. Signed learned timing includes encrypted signed-state retrieval, signature/evidence revalidation, policy checking, scoring and output serialization; it is not the 1.67-microsecond numeric kernel.

Learner post-fix evidence is recorded separately in `learner-final.json` with worker hash `bdb22770986d72476e5d951465ea13d64ed379f5c216149b63681126a342e780`. Its isolated required-guard receipt `cdna-engine-learner-isolated-final.json` has exit 0 and `sourceChanged:false`. All ten sample-size/timestamp probes preserve positive learned preference; equal-time fallback claims zero evaluation cases and no calibration. This is a synthetic regression, not personal accuracy or an independently calibrated production model.

Remaining scope limits: dense-memory latency above; no writer-contention test; explanation construction not independently instrumented; no real signer custody or personal holdout. The learner's 2,000-eligible-record bound and ordinary 15% test split yield about 300 total held-out rows, so a balanced six-domain snapshot cannot supply 200 held-out cases per domain. Use explicitly evaluated domain scope; numerical speed or synthetic agreement does not establish a six-domain personal clone.
