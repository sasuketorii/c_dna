# Learner worker v1

Invoke `uv run --project learner cdna-learner`. One bounded JSON object on stdin,
one JSON object on stdout; no files, pickle, or executable model input accepted.
Maximum input is 8 MiB and 2,000 records. Errors are `{"ok":false,"error":"invalid_request"}`.

Train request: `{"operation":"train","feature_version":"1.0","records":[...]}`.
Each record requires `family_id`, timezone-aware `timestamp`, `domain`, `context`,
`candidates`, `chosen_ids`, `verification_state`, `model_exposure`, and `answer_kind`
(`choose_one`, `choose_set`, `pairwise`, `acceptability`, `tie`, `skip`). Optional `case_kind` is actual or hypothetical.
Context and candidates follow appendix 29: context has as_of, summary, facts,
unknown_fields; candidates have id, text, attributes. Only confirmed, unexposed
records with an explicit preference comparison train; see extended semantics below.
All supplied candidates must have been presented and compared, enforced by broker.

Response has `ok`, `model` (feature_version, weights, provisional, calibration, abstention_margin, calibration_scope),
`evaluation`, `counts`, and `splits`. Rank request is
`{"operation":"rank","feature_version":"1.0","model":MODEL,"domain":"product_delivery","context":CONTEXT,"candidates":[...]}`.
Response `ranking` entries have candidate_id, raw_score; probability is never implied.

Feature order v1: six candidate numeric attributes `cost`, `effort`, `speed`,
`reuse`, `customer_impact`, `irreversibility`; six missing indicators; five
interactions (`deadline_pressure*speed`, `asset_importance*reuse`,
`loss_tolerance*irreversibility`, `customer_impact*speed`, `budget_pressure*cost`).
In every interaction the left operand is an explicit context fact, and the right
operand is a candidate attribute. In particular, context customer_impact × candidate
speed does not use the candidate customer_impact attribute.
Numbers must be normalized to [-1,1] by the trusted broker. Interaction context
facts with evidence_status=explicit and non-null values must have unit=ratio and
finite numeric values in [-1,1]; wrong/missing units and invalid ranges are rejected.
Null, unknown and inferred facts remain missing (zero interaction), regardless of unit. Text, IDs, domain strings, outcomes and reasons are never features.
No fitted preprocessing is needed. Versions must match exactly.

Families crossing temporal cutoffs are purged. Train/validation/calibration/test
are separate; small datasets produce train-only provisional models. Collapsed temporal
cutoffs (including equal timestamps), an earliest-time first cutoff, or family purging
that empties training also fall back to train-only. This fallback provides no held-out
evaluation or calibration evidence and cannot qualify for promotion. Evaluations
remain unverified and cannot auto-promote. Optional `compare_lightgbm:true` trains
one bounded ranker on the same train split and reports held-out raw-score agreement;
it never replaces the linear model automatically.

## Extended response semantics

The broker can invoke the fixed `python -m cdna_learner` entrypoint.

- `answer_kind: choose_set`: chosen_ids is nonempty. Only selected-vs-unselected
  constraints are generated, normalized across every cross-boundary comparison.
  Selecting all candidates creates no ranking constraints.
- `answer_kind: pairwise`: include left_id, right_id and preference (left/right/tie/insufficient).
  chosen_ids must be the one winner ID, or empty for tie/insufficient. Only these two candidates
  are compared, even when the source case includes others. Tie generates no labels.
- `answer_kind: acceptability`: chosen_ids identifies exactly one candidate;
  acceptability_outcome is accept/reject/conditional/insufficient. conditions is an array of
  strings; conditional requires at least one explicit condition. This never trains
  ranking. Confirmed/unexposed cases return acceptability_memory entries retaining
  family_id, candidate_id, outcome, conditions and generalized=false. Original case
  context remains in the broker's observation store; this is not a generalized model.

Other answer kinds must not carry pairwise/acceptability metadata. The broker maps
source.observed_at to timestamp, keeping context.as_of unchanged. All timestamps
must be timezone aware; future context relative to observed timestamp is rejected.
## Held-out calibration and selective evaluation

Validation requires 20 independent families. A fixed raw-score margin grid
(0, .05, .1, .25, .5, 1, 2, 5) selects the smallest margin with validation coverage
at least .6 and agreement at least .9. If none qualifies, margin=1000000 forces
abstention. If validation is insufficient, margin=0 and status=insufficient_data;
this fallback is not validated. Test data never selects the margin.

Calibration requires at least 20 families per included domain. A symmetric sigmoid
slope is fitted by bounded weighted log-loss minimization on calibration only.
No intercept is fitted, preserving reversal symmetry. Artifact calibration is null
when insufficient; otherwise:

```json
{"method":"sigmoid_pairwise","slope":1.0,"cases":20,
 "family_ids":["..."],"sample_hashes":["sha256..."],"source":"calibration"}
```

Every calibration case has a SHA-256 of the validated record serialization; these
are provenance identifiers, not features. `split_family_ids` records disjoint
partition ownership. Source observed timestamps and family isolation remain binding.

Rust pairwise inference: `sigmoid(slope*(score_left-score_right))`, only for exactly
two candidates and a domain listed in model.calibration_scope. Use a numerically
stable sigmoid. JSON rank responses identify left/right IDs explicitly; `probability`
remains null because no multi-choice or correctness probability is supplied.
Abstain when best minus second raw score is <= abstention_margin, including ties.

Test reports selective coverage, agreement and 95% Wilson intervals per domain,
plus held-out weighted pairwise Brier/log loss. `statistically_eligible` requires
calibration, validation threshold selection, and numeric targets both overall and
for every evaluated domain (200 test cases, 100 non-abstained, coverage .6,
agreement .9, Wilson lower .85). It does not approve promotion. Current artifacts
remain provisional until separate policy, paired evidence and manual approval.
Feature-exact replay is measured using train data only; no-rule-snapshot is reported
as unavailable, not a fabricated rule baseline. Repeatedly inspecting a test snapshot
still requires the broker to demote it to regression data before future final tests.

## Missing-input support and abstention

`model.supported_candidate_missing_masks` contains unique integer masks observed
in **training only**, never validation/calibration/test. Bit positions 0..5 follow
candidate keys cost, effort, speed, reuse, customer_impact, irreversibility. A bit
is 1 when its attribute is absent or null; complete input is mask 0; speed-only
input is mask 59. Absence of this metadata defaults to an empty support set.

A candidate with a previously unseen nonzero missing mask triggers abstention.
Missing patterns observed in training remain representable by the existing missing
indicator features; seeing a pattern is not a claim that its accuracy is validated.
Recognized context facts explicitly marked unknown/inferred/null, or listed in
unknown_fields, also trigger abstention. **Unspecified context facts are optional**
and do not trigger this gate. Wrong explicit units are still rejected.

Rank responses preserve raw ranking, set abstained=true, list missing_inputs and
abstention_reasons, and suppress pairwise_probability when this gate or the margin
gate applies. Validation selective metrics and held-out test coverage use the same
missing-mask gate. Calibration fits/evaluates only usable in-scope cases; excluded
held-out cases are counted separately. This conservative support gate prevents an
unseen missing input pattern from being silently treated as a validated prediction.

## Deterministic bounded snapshot fitting

The linear utility uses existing scikit-learn `LogisticRegression` (L-BFGS,
C=10, no intercept, tolerance 1e-8, at most 200 iterations). Symmetric pair
weights retain the actual/hypothetical and selected-set normalization above.
Training pair columns are divided by their weighted RMS, floored at .01;
only training rows fit this scale. The coefficients are divided by that same
scale before export. **The artifact still contains 17 raw-feature weights**:
Rust/Mojo need no fitted preprocessing, extra feature, or changed dot product.
This scaling changes optimization/regularization, not the feature contract.
No classifier `predict_proba` output is exported; the independent calibration
partition remains the sole source of pairwise probability metadata.

Pairs are sorted by numerical features, label and sample weight before fitting;
temporal split ordering also has deterministic family/record tie-breakers.
Each request refits from its currently eligible snapshot. A corrected observation
must arrive with the old version superseded/deleted and the replacement confirmed
and unexposed; excluded versions cannot keep affecting optimizer state. There is
no incremental cache or automatic last-write-wins deduplication. Contradictory
active observations remain contradictory evidence, not overwritten preferences.

## Signed production conversion boundary

The reference learner artifact is provisional, not a signed production model.
The signed runtime converter must preserve raw weights, calibrated slope/domain
scope and selected margin, then require independently evaluated inference metadata
and approval. This worker does **not** establish calibration validity dates, a
maximum supported margin, feature minimum/maximum envelopes, or context missing
pattern support. Its six-bit candidate masks are not proof of support for the
runtime's combined candidate/context masks. Do not manufacture these missing
fields from synthetic benchmarks or infer that provisional=false is approval.
Any missing required production metadata must retain abstention/reject activation.

## Optional domain-scoped training

Train requests accept optional `domain` (one of the six canonical domain values).
Absent/null retains pooled training. The trusted broker selects this scope explicitly;
it is never inferred from text or used as a numeric/string feature. Records are
filtered by domain **before** temporal/family splitting, optimization, calibration,
validation and missing-mask fitting. Acceptability memory is also scoped. The input
request limit remains 2,000 records; the Rust snapshot builder filters before its
eligible-record/byte cap while retaining the bounded confirmed-record scan.

Every returned model includes `training_domain`, including empty or small training
snapshots without calibration. Null means pooled and absence on legacy models also
means pooled. This scope is independent of `calibration_scope`, which may be empty;
calibration scope must never extend outside a non-null training domain.
Rank requires a canonical domain: missing domain abstains with `domain_missing`,
and a different domain on a scoped artifact abstains with `outside_training_domain`.
These gates return an empty ranking and no probabilities, before applying weights.
An invalid domain value (including empty string) is rejected.

Rust preview and signed conversion must preserve/enforce the same scope: preview
must reject/abstain before scoring a foreign domain, and conversion must restrict
inference domains to the artifact's selected domain. A calibration scope alone
is insufficient to establish training scope for uncalibrated artifacts. Feature
version remains 1.0 and raw weights remain length 17. Default pooled fitting cannot
represent opposite preferences for identical feature vectors across domains;
explicit scoped training creates separate artifacts, with no automatic routing,
activation, human evidence or promotion.
