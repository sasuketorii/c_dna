# C-DNA learner

Python 3.14.7 worker with pinned scikit-learn 1.9.1, Pydantic 2.13.5 and
LightGBM 4.7.0. `uv.lock` fixes transitive dependencies. Public registry versions
were checked during implementation; installation, imports and tests were exercised
on macOS arm64.

From this directory: `uv sync --locked`, then `uv run cdna-learner` with one JSON
request on stdin. See [worker contract](CONTRACT.md). Development verification:
`uv run pytest tests -q` and `uv run ruff check .` (run heavy commands through the
repository's required harness).

Training rebuilds a candidate from an eligible immutable snapshot. Confirmed,
unexposed choose-one, choose-set and non-tied pairwise observations generate symmetric comparisons with each
case's total weight preserved (actual=1, hypothetical=0.5). A deterministic,
regularized logistic comparison model uses train-only RMS scaling and one native
thread; scaling is folded into the exported weights. IDs and free text are never encoded. The broker
must supply the actual presented comparison set and authoritative verification
states; worker JSON is an internal trusted-broker interface, not public ingestion.

Temporal split cutoffs are 60/75/85 percent; any family spanning cutoffs is purged.
With fewer than 20 families, or when equal timestamps leave no training partition,
all data stays in provisional training and held-out performance is explicitly
absent. Optional domain-scoped training filters before splitting and exports its
scope even when calibration is unavailable; foreign-domain predictions abstain.
Validation selects an abstention margin from a fixed grid; calibration fits a
symmetric sigmoid on independent families. Neither partition updates ranking weights. Test metrics include domain counts, top-1 agreement, coverage,
non-abstained agreement and Wilson lower bound. Numeric product targets cannot
approve promotion without independent policy, scope and paired model evidence.

Conditional acceptance is retained separately as explicit memory without ranking labels.

Pairwise calibration needs 20 independent calibration families per included domain;
small datasets retain null calibration. Scope-bound probabilities are pairwise only.

The current 17-feature representation is a measured baseline, not the intended
ceiling of CEO/LLM teaching. Learning new concepts and exception representations
requires a versioned extractor/model bundle and independent evaluation; that
semantic distillation loop is not implemented by this numeric fitter alone.

Limitations: generalized acceptability models are not implemented. All exported
models remain provisional pending independent policy and manual approval. LightGBM is a
single experimental configuration, evaluated on the same untouched split; its
scores are not probabilities and it cannot auto-promote. Repeated inspection of a
test snapshot invalidates its final-test role; snapshot lifecycle, cancellation,
process timeout and memory limits and final-test reuse control belong to the broker. Input/output byte caps,
record/candidate limits and bounded thread/iteration counts are enforced here.

Calibration uses SciPy's bounded scalar optimizer and stable sigmoid, with bounded
iterations and no hand-written optimizer: [minimize_scalar](https://docs.scipy.org/doc/scipy/reference/generated/scipy.optimize.minimize_scalar.html),
[expit](https://docs.scipy.org/doc/scipy/reference/generated/scipy.special.expit.html).
