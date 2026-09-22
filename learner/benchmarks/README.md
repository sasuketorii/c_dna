# Synthetic preference learning benchmark

This is benchmark-only synthetic evidence, not human labels, personal prediction
accuracy, or production promotion evidence. Every record stays `unconfirmed` and
is rejected by the public training API. The benchmark calls the internal numerical
snapshot fitter explicitly and exports metrics, never trained model weights.

From `learner`, run through the required heavy-command harness:

```text
uv run python benchmarks/preference_bench.py --output benchmarks/results.json
```

Use the harness generation intent when writing the versioned results file; use an
external temporary output for unchanged-source verification. Tests remain
`uv run pytest tests -q`.

## Current optimizer evidence

See [optimizer-report.md](optimizer-report.md) for the bounded solver comparison,
independent four-way family isolation, held-out results, repeat/correction costs,
source hash and unchanged failures. `optimizer-validation.json` freezes candidate
selection; `optimizer-test.json` measures that selection; `optimizer-regression.json`
re-runs the previously inspected fixtures. The sections below preserve the measured
SGD baseline, not current optimizer scores. No synthetic observation is a human label.

Run validation first, freeze its result, then test through the heavy-command guard:

```text
uv run python benchmarks/optimizer_bench.py --stage validation --output benchmarks/optimizer-validation.json
uv run python benchmarks/optimizer_bench.py --stage test --selection benchmarks/optimizer-validation.json --output benchmarks/optimizer-test.json
```

Do not overwrite a frozen selection after inspecting its test. Verification reruns
should use temporary output files and count as regression, not new final evidence.

## Historical SGD design and measured results

`results.json` records the worker SHA-256, Python/platform, fixed seed, oracle
weights and all metrics. Two thousand cases use an expressible 17-feature utility,
six domains, four candidates, timezone-aware increasing timestamps and grouped
families. The last 300 cases form the identical external holdout at all learning
checkpoints. No oracle weights enter model training, and no threshold is selected
from either internal or external test labels. The historical SGD configuration was
not tuned using these benchmark scores. Already-inspected fixtures are regression
evidence for future algorithm changes, not a new final test.

| Available synthetic observations | External holdout agreement |
| ---: | ---: |
| 0 | 0% (all ties, no coverage) |
| 8 | 50.7% |
| 32 | 72.0% |
| 128 | 82.3% |
| 512 | 89.3% |
| 1700 | 91.0% |

The 1700-observation candidate has 254 usable internal held-out cases after family
purging: pairwise Brier 0.0210, log loss 0.0695. These are different case counts
from the external 300-case learning curve. No per-domain promotion claim follows.
With 20% deliberately contradictory synthetic labels, raw external agreement is
87%; the validation-selected margin abstains on every external case. This is
reported as coverage 0, not 100% accuracy.

Before/after snapshots are `before.json` and `results.json`. Repeated candidate
feature extraction in choose-set cross products was removed; the pair values,
weights, raw learning curve and noise result remain unchanged. On the recorded
macOS arm64/Python 3.14.7 run:

| Measurement | Before | After |
| --- | ---: | ---: |
| 128 cases × 16 candidates, symmetric pair generation | 150.8 ms | 33.5 ms |
| 1700-case snapshot fit | 223.6 ms | 199.9 ms |
| Four-candidate resident scoring p50 / p95 | — | 0.0355 / 0.0422 ms |
| Fresh worker process + imports + empty train, median | — | 1189.7 ms |

Pair generation improved about 4.5×. Timings are local observations under the
recorded load, not a cross-platform guarantee. Resident scoring excludes IPC;
fresh-worker timing includes it. Native inference should not inherit Python's
process/import cost.

## Historical scenarios and discovered safety defect

The benchmark also consumes `../benchmarks/scenarios/preference_cases.jsonl`:
142 source records, 139 structurally valid, 3 correctly rejected by schema/unit
validation, 2 additional correctly rejected by numeric feature extraction, and
zero contract/oracle mismatches. Normal data uses 60 effective training cases and
a separate 20-case external holdout: measured agreement is 70%. This smaller,
differently shaped synthetic profile is not pooled into the 2000-case score.

The first behavioral run found seven unknown/missing input cases producing a
ranking without abstention. The worker now retains raw reference scores but
abstains for explicitly uncertain context facts and candidate missing patterns
not observed in training. Unspecified context facts remain optional; historically
supported missing patterns remain usable. Validation, test coverage and calibration
eligibility use the same support gate. The seven missing/unknown cases and one tie
case now satisfy their abstention expectations (`unmet_behavior_diagnostics=[]`).

Remaining limits: modest scenario accuracy, no natural-language feature learning,
no real-person acceptance evidence, no model-vs-current paired promotion test, and
no generalization guarantee outside the declared synthetic distributions.
