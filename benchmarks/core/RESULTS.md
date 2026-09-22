# Indexed Engine measurement follow-up

Both runs use the real Engine with synthetic proposal/confirmation/policy fixtures, release binaries, 10 warmups and 200 samples. The 1/100/1000 all-matching fixtures preserve the previous workload shape (UUIDs are generated per run). The 1000-row output intentionally changed from blanket abstention to complete-match selection. The new 1001-match and 1000-total/1-match cases have no historical same-fixture baseline.

**Evidence limit:** the indexed binary remained byte-identical during its run, but source changed during both its build and measurement. The indexed runner therefore reports `source_or_binary_changed` and exits 2; the outer guard exits 50. These are observations of that fixed executable, not a verified mapping to the final source or a release performance acceptance. No repeat loop was used to chase a global source digest while other owners worked.

| Real Engine scenario | Total / matching | Previous p50 µs | Indexed p50 µs | Indexed p95 µs |
|---|---:|---:|---:|---:|
| engine_rank_confirmed_memory | 1 / 1 | 75.792 | 76.500 | 93.958 |
| engine_rank_memory_and_approved_policy | 1 / 1 | 85.5 | 86.625 | 96.792 |
| engine_rank_confirmed_memory | 100 / 100 | 2940.125 | 1287.084 | 1488.833 |
| engine_rank_memory_and_approved_policy | 100 / 100 | 2862.333 | 1342.791 | 2735.291 |
| engine_rank_confirmed_memory | 1000 / 1000 | 33610.125 | 19600.708 | 21084.000 |
| engine_rank_memory_and_approved_policy | 1000 / 1000 | 33802.542 | 20036.292 | 21518.750 |
| engine_rank_confirmed_memory | 1001 / 1001 | unmeasured | 20363.000 | 23575.541 |
| engine_rank_memory_and_approved_policy | 1001 / 1001 | unmeasured | 20168.791 | 24716.583 |
| engine_rank_confirmed_memory | 1000 / 1 | unmeasured | 80.750 | 308.458 |
| engine_rank_memory_and_approved_policy | 1000 / 1 | unmeasured | 92.583 | 380.708 |

All 14 scenario assertions completed: exactly 1000 matches select fast, 1001 matches abstain with memory_search_limit_reached, and one match among 1000 records selects fast. Policies still classify fast compliant and slow violated; execution authorization is always none. The evidence list remains capped at 20.

Whole-process peak RSS was 24,330,240 bytes before and 27,394,048 bytes after, but the new run has 14 rather than 10 scenarios and additional fixture construction, so these are not comparable per-scenario memory measurements. CPU/wall ratios are approximately 0.918 in both runs; they are not hardware-wide CPU percentages.

Artifacts: [baseline](results/baseline.json), [indexed](results/indexed.json). Guard receipt basenames: `cdna-core-bench-indexed-build.json` (cargo exit 0, sourceChanged=true), `cdna-core-bench-indexed-run.json` (runner exit 2, sourceChanged=true). The earlier baseline report itself says stable_artifact_snapshot. Rebuild and rerun after integration stabilizes before attributing these measurements to a final source revision.
