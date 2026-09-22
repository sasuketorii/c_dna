# Engine benchmark and adversarial review

Independent, synthetic probes of the real SQLCipher Store and public Engine path. This package is a standalone Cargo workspace so benchmark ownership does not require editing the root workspace. It shares production crates by path; it does not implement an alternate inference engine.

From the repository root, after production writers finish:

```sh
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:$PATH"
REV_SKILLS="${REV_SKILLS:-$HOME/dev/rev_skills}"
ENGINE_EVIDENCE="$(mktemp -d)"
node "$REV_SKILLS/skills/rev-development-harness/scripts/bin/revh.mjs" command run \
  --guard required --class heavy --root . --task engine-independent-review \
  --out "$ENGINE_EVIDENCE/engine-receipt.json" -- \
  python3 benchmarks/engine/verify.py --out-dir "$ENGINE_EVIDENCE/results" --iterations 200
```

The locked offline build requires the repository dependencies already cached. `verify.py` binds pre/post-build source hashes to both binaries, then reuses the existing `benchmarks/core/run.py` resource recorder for each child. It rejects source or binary changes during measurement, signed acceptance of the two adversarial datasets, and failure to prepare exactly 20 eligible records from the 10,000-record corpus. Guard receipts belong outside the worktree; copy accepted artifacts into `results/` after the command finishes.

The memory benchmark creates exactly 10,000 confirmed records via public propose/confirm. Twenty are human-unexposed sparse matches; 9,980 are model-exposed dense matches, excluded from learning. It measures one process with at most two concurrent reader threads, separate connections to one encrypted database, no concurrent writer. Corpus creation is included in process CPU/RSS but excluded from request latency. Setup is intentionally real durable ingestion, not a direct SQL bulk insert.

Nearest-rank p50/p95 use monotonic wall time, 10 warm-up calls and 200 measured requests (dense search capped at 100). First calls and database reopen are reported separately; neither evicts OS caches. Numeric scoring uses supplied structured attributes; it is not natural-language understanding. Explanation-only measurement is serialization of precomputed real evidence, not explanation generation; generation remains in the full Engine measurement. Component measurements overlap and cannot be summed or subtracted as profiler stages.

The signed fixture uses public register/evaluate/approve/activate/rank with test-only signing keys. Its positive control has 200 independent test families. Negative probes use 200 rows from one family, or a second domain with only one evaluated case. Fixture labels, timing claims inside the signed receipt and keys are deliberately synthetic; only externally measured Engine latency is benchmark evidence. No synthetic signature is real-person acceptance.

Learner sample-efficiency reproduction:

```sh
node "$REV_SKILLS/skills/rev-development-harness/scripts/bin/revh.mjs" command run \
  --guard required --class heavy --root . --task engine-learner-review \
  --out "$ENGINE_EVIDENCE/learner-receipt.json" -- \
  learner/.venv/bin/python benchmarks/engine/learner_probes.py
```

This tests 1/19/20/40/200 families with equal versus increasing timestamps through production Request/run. Historical `learner-before.json` captures the 20-family zero-weight cliff. See `docs/release/ENGINE_REVIEW.md` for current acceptance and unresolved limits. No UI, deployment, arbitrary-NL SLA, independent personal accuracy, or live evaluator evidence is claimed.

Final reported measurements use an equal-byte isolated source snapshot captured before the separately assigned withAI expansion. The capture map, stable build map and binary hashes in `results/` bind that scope; later production changes need their own integrated verification. When other writers remain active, copy the mapped source tree into a temporary Git repository, verify every copied file hash against the capture map, and run the same guarded command there. Do not relax drift detection or call stale results current.
