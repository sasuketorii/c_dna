# Bounded learner optimization evidence — 2026-09-22

The production learner now uses train-only weighted RMS scaling and a bounded
L-BFGS logistic solver instead of 30 fixed SGD passes. Scaling is folded into
17 exported coefficients; no feature, schema, runtime preprocessing, probability
claim, or acceptance threshold changed. Canonical numerical row ordering removes
SGD order sensitivity. Corrections rebuild the eligible snapshot from scratch.

## Selection and isolation

`optimizer_bench.py` independently defines four policy equations without calling
`worker.features()` to generate labels. These are simulated policies, **not human
accuracy evidence**. All records stay unconfirmed, and the public training API
rejects them. Nothing exports a model or promotes a candidate.

Four existing-library candidates were fixed before measurement: baseline SGD,
averaged SGD, unscaled LogisticRegression C=10, and RMS-scaled LogisticRegression
C=10. Equal mean validation agreement across four profiles and training sizes
8/32/128/512 selected scaled logistic: **65.06%**, versus **46.53%**, **46.34%**,
and **50.66%**, respectively. `optimizer-validation.json` was written before final
test labels were generated. Its SHA-256 is
`94c23992f022c3d63b2bd1aa1df7470b888ed334d8a4203a52be9d1d0457298f`.

Each policy has independent train (512), validation (200), calibration (160) and
test (240) families, separate random seeds and nonoverlapping temporal ranges.
Train size checkpoints use train only; margin selection uses validation only;
symmetric sigmoid fitting uses calibration only. Test never chooses the solver,
scale, regularization or threshold. Calibration/validation observations are extra
label costs, so the train size column is not the total sample acquisition cost.
The test snapshot is now inspected and is regression evidence on future runs.

The validation receipt binds the original comparison implementation; final
measurement calls the selected production `fit_linear` directly. The only
numerical consolidation is computing RMS after canonical row sorting, ensuring
bit-identical replay. The candidate configuration and feature equations were not
changed using test results.

## Held-out results

`optimizer-test.json` reports full per-domain agreement, selective coverage,
Wilson intervals and held-out calibration loss. At 512 training families:

| Independent policy | SGD agreement | New agreement | New selective coverage |
| --- | ---: | ---: | ---: |
| Context-dependent tradeoff | 90.83% | 99.17% | 100% |
| Important speed signal in range ±.02 | 36.25% | 99.17% | 100% |
| Nonlinear interior optimum | 71.25% | 71.25% | 0% |
| Opposite domain preferences | 25.83% | 24.58% | 0% |

Across all four sizes and policies, test mean rises from **49.09% to 65.18%**.
At only 32 training families, context agreement rises 84.58% → 92.50%, and the
narrow-signal profile rises 34.58% → 89.58%. This identifies a concrete numerical
failure: fixed-rate SGD underlearned important low-amplitude inputs despite their
being valid normalized features. It does not establish natural-language extraction
accuracy, individual-person accuracy, or an ability to learn domain-dependent
preferences when domain itself is excluded from the features.

## Repeat learning, cost and remaining failures

The bounded repeat test uses 128 observations, replaces 16 labels and performs
five fresh fits. Correction median drops **22.35 ms → 6.59 ms** (about 3.39×).
Reversing record order changes baseline coefficients by up to **0.01035**;
new coefficient delta is **0**. Repeated fits and restoring the original snapshot
both have coefficient delta **0**. Public-API tests separately verify that
superseded records do not contribute and a coherent preference reversal changes
the exported ranking in the correct direction.

The 16 deliberately arbitrary contradictory corrections are still learned poorly:
both solvers match only **1/16** after fitting (0/16 before correction). Retained
112-case agreement improves 84.82% → 87.50%. These are training replay measures,
not held-out generalization. Exact correction replay is deterministic, but this
linear model does not memorize arbitrary exceptions. Nonlinear and domain-conflict
profiles also remain failures; validation forces abstention for both. Domain
conflict gets slightly worse in raw agreement, and no test-informed repair was made.

The full independent benchmark consumed 5.19 user CPU seconds, 0.35 system CPU
seconds, 6.18 seconds wall time and 152.64 MiB peak RSS on this macOS arm64 run.
Library thread limits remain one; no new dependency, service, parallel trainer or
giant model was added. Max 200 iterations, 2000 input records and 16 candidates
bound fitting. These local measurements are not worst-case memory/latency proofs.

## Existing regression suite

`optimizer-regression.json` re-runs the previously inspected known-utility and
independently authored scenario fixtures. The old `results.json` remains the
historical SGD baseline rather than being overwritten.

- Independent 20-case scenario agreement: **70% → 85%**; still 3 wrong cases.
- Seven missing/unknown cases and one tie still abstain; zero schema/oracle mismatches.
- Known linear-utility curve at 8/32/128/512/1700 supplied observations:
  **67.33/81.67/91.00/98.33/99.00%**, versus **50.67/72.00/82.33/89.33/91.00%**.
- 20% contradictory-label raw agreement: **87% → 92.67%**, still **0 coverage**
  with the independently selected abstention margin.
- Full internal 254-case test: Brier .003615, log loss .011256, versus .0210/.0695.

These fixtures are regression checks, not an uninspected final test. Their results
were not used to tune the selected candidate.

## Verification and production boundary

Initial guarded `uv run pytest tests -q`: **30 passed**. Guarded
`uv run ruff check src tests benchmarks`: **passed**. Both initial verification receipts
reported no source change. Guarded benchmark generation exited 0 with no resource
findings; generation receipts correctly report output mutation and do not substitute
for unchanged-source verification. Required tree-sitter graph exploration identified
`train` and benchmark `benchmark_fit` callers of `_fit_snapshot`; graph scope remains
candidate-only and direct source references were also inspected.

Optimizer measurement worker SHA-256:
`19663bbaa17979c72ee8cd7372364d0fc5cd6da11f2933a1634d1a36278f9e8b`.
Source-bound benchmark artifacts record this hash and their generator hash.

A subsequent independent verifier found the existing temporal cliff: 20 or more
same-timestamp families all entered test, erasing trained weights. Final source
now uses train-only provisional fallback when temporal cutoffs collapse, begin
at the earliest timestamp, or family purging empties training. It creates zero
held-out cases and no calibration/promotion evidence. Tests cover 19/20/40/200
same-time families plus degenerate crossing families. This post-benchmark split
repair does not alter the selected numerical fitter; benchmark hashes above
identify the measured revision rather than being relabeled as the final revision.
The final expanded suite reports **35 passed**, including all temporal-cliff cases.
The shared-tree guard detected concurrent source changes, so that receipt is not
an unchanged-source acceptance receipt. An exact-source isolated verification
snapshot completed **35 passed in 6.42 seconds**, guard exit 0, sourceChanged=false,
no resource findings. The isolated worker SHA equals final source below. Final
ruff also passed under its guard. Benchmark generation remains the measured
pre-temporal-fix revision identified above; no benchmark receipt is relabeled.
Final worker SHA-256:
`ac81bf32feb6908ad0bdc430066fb8b828172e4c3b60c5456e1082ec97737a26`.

Native Rust/Mojo execution parity and signed-runtime activation were not run by
this learner worker; raw coefficient export and feature contract remain unchanged.
The production inference worker was informed before contract edits and the runtime
packager was notified of source freeze. Signed conversion still requires evaluator
validity windows, feature envelopes, combined missing-pattern support and independent
approval; learner candidate masks and synthetic results cannot manufacture them.

Existing dependency used directly: scikit-learn 1.9.1
[LogisticRegression documentation](https://scikit-learn.org/stable/modules/generated/sklearn.linear_model.LogisticRegression.html).
No stack or dependency files changed.

## Next bounded slice: domain-scoped fitting (proposal only)

Domain conflict is not solved by this optimizer. The trusted broker should filter
active training observations by domain before snapshot construction, fit one
existing weights17 model per domain, and bind each signed artifact to exactly that
domain. Runtime routing must reject absent/incorrect scope, never silently fall back
to another head. Each domain needs its own family-isolated temporal validation,
calibration and test evidence; sparse domains remain provisional/abstaining.

Simply honoring the currently unused train `Request.domain` field would be unsafe
without scope enforcement: low-data artifacts have no calibration scope and could
be misapplied as global weights. No such API change was made in this slice. The next
evaluation should include feature-identical pairs with opposite choices across two
domains, with fixed routing and no best-head selection using test labels. The root
coordinator received this proposal and the required signed-runtime boundary.
