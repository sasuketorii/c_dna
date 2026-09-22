"""JSON-only worker. No persistence, network, or unsafe model deserialization."""

import os

for _name in ("OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS"):
    os.environ[_name] = "1"
import hashlib
import json
import math
import sys
from typing import Annotated, Literal

import numpy as np
from pydantic import AwareDatetime, BaseModel, ConfigDict, Field, StrictBool, model_validator
from scipy.optimize import minimize_scalar
from scipy.special import expit
from sklearn.linear_model import LogisticRegression

VERSION = "1.0"
KEYS = ("cost", "effort", "speed", "reuse", "customer_impact", "irreversibility")
INTERACTIONS = (
    ("deadline_pressure", "speed"),
    ("asset_importance", "reuse"),
    ("loss_tolerance", "irreversibility"),
    ("customer_impact", "speed"),
    ("budget_pressure", "cost"),
)


class DTO(BaseModel):
    model_config = ConfigDict(extra="forbid", allow_inf_nan=False)


class Fact(DTO):
    key: str = Field(pattern=r"^[a-z][a-z0-9_]{0,63}$")
    value: object
    evidence_status: Literal["explicit", "inferred", "unknown"]
    unit: str | None = Field(default=None, max_length=32)


class Context(DTO):
    as_of: AwareDatetime
    summary: str = Field(min_length=1, max_length=32768)
    facts: list[Fact] = Field(max_length=128)
    unknown_fields: list[str] = Field(max_length=128)

    @model_validator(mode="after")
    def coherent(self):
        keys = [f.key for f in self.facts]
        if len(set(keys)) != len(keys) or len(set(self.unknown_fields)) != len(self.unknown_fields):
            raise ValueError("duplicate fact")
        if any(f.key in self.unknown_fields and f.evidence_status != "unknown" for f in self.facts):
            raise ValueError("unknown contradiction")
        interaction_facts(self)
        return self


class Candidate(DTO):
    id: str = Field(pattern=r"^[a-zA-Z0-9_-]{1,64}$")
    text: str = Field(min_length=1, max_length=4096)
    attributes: dict[str, object] = Field(max_length=64)


Domain = Literal[
        "resource_allocation",
        "product_delivery",
        "customer_commercial",
        "organization_delegation",
        "growth_strategy",
        "risk_reputation",
]


class Record(DTO):
    family_id: str = Field(min_length=1, max_length=128)
    timestamp: AwareDatetime
    domain: Domain
    context: Context
    candidates: list[Candidate] = Field(min_length=1, max_length=16)
    chosen_ids: list[str] = Field(max_length=16)
    verification_state: Literal["confirmed", "unconfirmed", "rejected", "deleted", "superseded"]
    model_exposure: StrictBool
    answer_kind: Literal["choose_one", "choose_set", "pairwise", "acceptability", "tie", "skip"]
    left_id: str | None = None
    right_id: str | None = None
    preference: Literal["left", "right", "tie", "insufficient"] | None = None
    acceptability_outcome: Literal["accept", "reject", "conditional", "insufficient"] | None = None
    conditions: list[str] = Field(default_factory=list, max_length=16)
    case_kind: Literal["actual", "hypothetical"] = "actual"

    @model_validator(mode="after")
    def coherent(self):
        ids = [c.id for c in self.candidates]
        if len(ids) != len(set(ids)) or len(self.chosen_ids) != len(set(self.chosen_ids)):
            raise ValueError("duplicate candidate")
        if not set(self.chosen_ids) <= set(ids):
            raise ValueError("unknown choice")
        if self.answer_kind == "choose_one" and len(self.chosen_ids) != 1:
            raise ValueError("one choice required")
        if self.answer_kind == "choose_set" and not self.chosen_ids:
            raise ValueError("selected set required")
        if self.answer_kind == "pairwise":
            if self.left_id == self.right_id or self.left_id not in ids or self.right_id not in ids:
                raise ValueError("invalid pair references")
            expected = {
                "left": [self.left_id],
                "right": [self.right_id],
                "tie": [],
                "insufficient": [],
            }
            if self.preference not in expected or self.chosen_ids != expected[self.preference]:
                raise ValueError("pairwise outcome mismatch")
        elif self.left_id is not None or self.right_id is not None or self.preference is not None:
            raise ValueError("pairwise metadata on other answer")
        if self.answer_kind == "acceptability":
            if len(self.chosen_ids) != 1 or self.acceptability_outcome is None:
                raise ValueError("acceptability target and outcome required")
            if self.acceptability_outcome == "conditional" and not self.conditions:
                raise ValueError("conditional acceptance requires conditions")
            if any(not text.strip() or len(text) > 2048 for text in self.conditions):
                raise ValueError("invalid condition")
        elif self.acceptability_outcome is not None or self.conditions:
            raise ValueError("acceptability metadata on ranking")
        if self.context.as_of > self.timestamp:
            raise ValueError("future context")
        return self


class Calibration(DTO):
    method: Literal["sigmoid_pairwise"] = "sigmoid_pairwise"
    slope: float = Field(gt=0, le=100)
    cases: int = Field(ge=20, le=2000)
    family_ids: list[str] = Field(min_length=20, max_length=2000)
    sample_hashes: list[str] = Field(min_length=20, max_length=2000)
    source: Literal["calibration"] = "calibration"

    @model_validator(mode="after")
    def coherent(self):
        if len(set(self.family_ids)) != len(self.family_ids) or len(self.family_ids) > self.cases:
            raise ValueError("invalid calibration families")
        if len(self.sample_hashes) != self.cases or any(
            len(h) != 64 or any(c not in "0123456789abcdef" for c in h) for h in self.sample_hashes
        ):
            raise ValueError("invalid calibration sample hashes")
        return self


class Model(DTO):
    feature_version: Literal["1.0"]
    weights: list[Annotated[float, Field(ge=-1e6, le=1e6)]] = Field(min_length=17, max_length=17)
    provisional: StrictBool = True
    calibration: Calibration | None = None
    abstention_margin: float = Field(default=0, ge=0, le=1e6)
    training_domain: Domain | None = None
    calibration_scope: list[Domain] = Field(default_factory=list, max_length=6)
    supported_candidate_missing_masks: list[Annotated[int, Field(ge=0, le=63)]] = Field(
        default_factory=list, max_length=64
    )

    @model_validator(mode="after")
    def coherent(self):
        if len(set(self.supported_candidate_missing_masks)) != len(
            self.supported_candidate_missing_masks
        ):
            raise ValueError("duplicate missing masks")
        if self.training_domain is not None and any(
            domain != self.training_domain for domain in self.calibration_scope
        ):
            raise ValueError("calibration outside training domain")
        if bool(self.calibration) != bool(self.calibration_scope):
            raise ValueError("calibration scope mismatch")
        if not self.provisional and self.calibration is None:
            raise ValueError("nonprovisional model requires calibration")
        return self


class Request(DTO):
    operation: Literal["train", "rank"]
    feature_version: Literal["1.0"]
    records: list[Record] = Field(default_factory=list, max_length=2000)
    compare_lightgbm: bool = False
    domain: Domain | None = None
    model: Model | None = None
    context: Context | None = None
    candidates: list[Candidate] = Field(default_factory=list, max_length=16)


def numeric(value):
    if value is None:
        return 0.0, 1.0
    if type(value) not in (int, float) or not math.isfinite(value) or not -1 <= value <= 1:
        raise ValueError("feature must be normalized")
    return float(value), 0.0


def interaction_facts(context):
    facts = {}
    allowed = {key for key, _ in INTERACTIONS}
    for fact in context.facts:
        if fact.key not in allowed or fact.evidence_status != "explicit" or fact.value is None:
            continue
        if fact.unit != "ratio":
            raise ValueError("interaction context feature requires ratio unit")
        facts[fact.key] = numeric(fact.value)[0]
    return facts


def features(context, candidate):
    values = [numeric(candidate.attributes.get(k)) for k in KEYS]
    attrs = dict(zip(KEYS, (v[0] for v in values), strict=True))
    facts = interaction_facts(context)
    return np.array(
        [v[0] for v in values]
        + [v[1] for v in values]
        + [numeric(facts.get(x))[0] * attrs[a] for x, a in INTERACTIONS]
    )


def candidate_missing_mask(candidate):
    return sum(1 << i for i, key in enumerate(KEYS) if candidate.attributes.get(key) is None)


def missing_inputs(context, candidates, supported_masks=()):
    """Explicit uncertainty prevents decisive output; unspecified context facts are optional."""
    recognized = {key for key, _ in INTERACTIONS}
    missing = {f"context:{key}" for key in context.unknown_fields if key in recognized}
    missing.update(
        f"context:{fact.key}"
        for fact in context.facts
        if fact.key in recognized and (fact.evidence_status != "explicit" or fact.value is None)
    )
    for candidate in candidates:
        mask = candidate_missing_mask(candidate)
        if mask not in supported_masks:
            missing.update(
                f"candidate:{candidate.id}:{key}" for i, key in enumerate(KEYS) if mask & (1 << i)
            )
    return sorted(missing)


def comparisons(r):
    """Only comparisons the answer actually establishes; never order unselected items."""
    if r.answer_kind == "pairwise":
        if r.preference in ("tie", "insufficient"):
            return []
        ids = (r.left_id, r.right_id) if r.preference == "left" else (r.right_id, r.left_id)
        by_id = {c.id: c for c in r.candidates}
        return [(by_id[ids[0]], by_id[ids[1]])]
    if r.answer_kind in ("choose_one", "choose_set"):
        return [
            (chosen, other)
            for chosen in r.candidates
            if chosen.id in r.chosen_ids
            for other in r.candidates
            if other.id not in r.chosen_ids
        ]
    return []


def eligible(r):
    return r.verification_state == "confirmed" and not r.model_exposure and bool(comparisons(r))


def comparison_candidates(r):
    if r.answer_kind == "pairwise":
        return [c for c in r.candidates if c.id in (r.left_id, r.right_id)]
    return r.candidates


def split(records):
    # Temporal boundaries are chosen from cases, then entire crossing families purged.
    ordered = sorted(records, key=lambda r: (r.timestamp, r.family_id, r.model_dump_json()))
    result = {k: [] for k in ("train", "validation", "calibration", "test", "purged")}
    if len({r.family_id for r in records}) < 20:
        result["train"] = ordered
        return result
    cuts = [ordered[int(len(ordered) * f)].timestamp for f in (0.6, 0.75, 0.85)]
    # Coarse/imported timestamps may collapse quantiles. Do not erase learned
    # preferences by assigning every observation to a held-out bucket. Without
    # independent temporal windows, train provisionally and claim no evaluation.
    if len(set(cuts)) < 3 or cuts[0] <= ordered[0].timestamp:
        result["train"] = ordered
        return result
    families = {}
    for r in ordered:
        families.setdefault(r.family_id, []).append(r)
    names = list(result)
    for rows in families.values():
        buckets = {sum(r.timestamp >= cut for cut in cuts) for r in rows}
        result[names[next(iter(buckets))] if len(buckets) == 1 else "purged"].extend(rows)
    if not result["train"]:
        # Family purging can also eliminate the complete training partition.
        return {name: ordered if name == "train" else [] for name in result}
    return result


def pairs(records):
    x, y, weights = [], [], []
    for r in records:
        constraints = comparisons(r)
        if not constraints:
            continue
        weight = (1.0 if r.case_kind == "actual" else 0.5) / (2 * len(constraints))
        # A selected-set cross product repeats each candidate many times. Extract
        # each involved candidate once; never inspect unrelated pairwise options.
        involved = {candidate.id: candidate for pair in constraints for candidate in pair}
        vectors = {key: features(r.context, candidate) for key, candidate in involved.items()}
        for chosen, other in constraints:
            diff = vectors[chosen.id] - vectors[other.id]
            x.extend((diff, -diff))
            y.extend((1, 0))
            weights.extend((weight, weight))
    return np.asarray(x), np.asarray(y), np.asarray(weights)


def metrics(cases, answered, correct):
    p = correct / answered if answered else 0.0
    z = 1.96
    center = (p + z * z / (2 * answered)) / (1 + z * z / answered) if answered else 0.0
    radius = (
        z
        * math.sqrt(p * (1 - p) / answered + z * z / (4 * answered * answered))
        / (1 + z * z / answered)
        if answered
        else 0.0
    )
    coverage = answered / cases if cases else 0.0
    return {
        "cases": cases,
        "non_abstained": answered,
        "correct": correct,
        "coverage": coverage,
        "agreement": correct / cases if cases else None,
        "non_abstained_agreement": p if answered else None,
        "wilson_lower_95": max(0.0, center - radius),
        "wilson_upper_95": min(1.0, center + radius) if answered else 1.0,
        "numeric_targets_met": cases >= 200
        and answered >= 100
        and coverage >= 0.6
        and p >= 0.9
        and center - radius >= 0.85,
    }


def evaluate(records, score, margin=0, supported_masks=()):
    answered = correct = 0
    by_domain = {}
    for r in records:
        candidates = comparison_candidates(r)
        values = np.asarray(score(np.array([features(r.context, c) for c in candidates])))
        order = np.argsort(values)
        responded = not missing_inputs(r.context, candidates, supported_masks) and bool(
            values[order[-1]] - values[order[-2]] > margin
        )
        hit = responded and candidates[int(order[-1])].id in r.chosen_ids
        correct += int(hit)
        answered += int(responded)
        counts = by_domain.setdefault(r.domain, [0, 0, 0])
        counts[0] += 1
        counts[1] += int(responded)
        counts[2] += int(hit)
    result = metrics(len(records), answered, correct)
    result["by_domain"] = {d: metrics(*counts) for d, counts in by_domain.items()}
    result["metric_definition"] = "unique_top_in_chosen_set; pairwise restricted to compared pair"
    return result


def select_margin(records, weights, supported_masks=()):
    """Fixed search grid, validation only; no final-test tuning."""
    if len({r.family_id for r in records}) < 20:
        return 0.0, "insufficient_data"
    for margin in (0.0, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0):
        measured = evaluate(records, lambda x: x @ weights, margin, supported_masks)
        if measured["coverage"] >= 0.6 and (measured["non_abstained_agreement"] or 0) >= 0.9:
            return margin, "fitted_validation"
    return 1e6, "validation_target_not_met_abstain"


def fit_calibration(records, weights):
    domains = {r.domain for r in records}
    allowed = {d for d in domains if len({r.family_id for r in records if r.domain == d}) >= 20}
    records = [r for r in records if r.domain in allowed]
    if not records:
        return None
    x, y, sample_weight = pairs(records)
    scores = x @ weights
    if not np.any(np.abs(scores) > 1e-12):
        return None

    # Symmetric temperature scaling preserves P(a>b) = 1-P(b>a).
    def loss(slope):
        logits = slope * scores
        return float(np.average(np.logaddexp(0, logits) - y * logits, weights=sample_weight))

    fitted = minimize_scalar(loss, bounds=(0.001, 100), method="bounded", options={"maxiter": 100})
    if not fitted.success or not math.isfinite(fitted.fun):
        return None
    return Calibration(
        slope=float(fitted.x),
        cases=len(records),
        family_ids=sorted({r.family_id for r in records}),
        sample_hashes=[hashlib.sha256(r.model_dump_json().encode()).hexdigest() for r in records],
    )


def calibration_test(records, weights, calibration):
    if calibration is None or not records:
        return {"status": "unavailable", "brier": None, "log_loss": None}
    x, y, sample_weight = pairs(records)
    logits = calibration.slope * (x @ weights)
    probabilities = expit(logits)
    return {
        "status": "heldout_test",
        "cases": len(records),
        "brier": float(np.average((probabilities - y) ** 2, weights=sample_weight)),
        "log_loss": float(np.average(np.logaddexp(0, logits) - y * logits, weights=sample_weight)),
    }


def memory_baseline(train_records, test_records):
    # Feature-exact replay, no future answers and no invented rules.
    known = {}
    for r in train_records:
        key = (r.domain, tuple(tuple(features(r.context, c)) for c in comparison_candidates(r)))
        chosen = tuple(i for i, c in enumerate(comparison_candidates(r)) if c.id in r.chosen_ids)
        known.setdefault(key, set()).add(chosen)
    answered = correct = 0
    for r in test_records:
        key = (r.domain, tuple(tuple(features(r.context, c)) for c in comparison_candidates(r)))
        choices = known.get(key, set())
        if len(choices) == 1:
            selected = next(iter(choices))
            if len(selected) == 1:
                answered += 1
                correct += int(comparison_candidates(r)[selected[0]].id in r.chosen_ids)
    return {
        "kind": "feature_exact_replay_train_only",
        **metrics(len(test_records), answered, correct),
    }


def fit_linear(records):
    """Fit bounded pairwise logistic utility; fold train-only scaling into weights17.

    Refit active observations from scratch after correction/deletion. Canonical
    numerical rows make fitting independent of record and candidate ordering.
    Predictions remain raw utilities; only separate held-out calibration supplies
    pairwise probabilities. No transform or estimator object crosses the boundary.
    """
    if not records:
        return np.zeros(17)
    x, y, sample_weight = pairs(records)
    order = np.lexsort((sample_weight, y, *x.T))
    x, y, sample_weight = x[order], y[order], sample_weight[order]
    scale = np.maximum(np.sqrt(np.average(x * x, axis=0, weights=sample_weight)), 0.01)
    clf = LogisticRegression(C=10, fit_intercept=False, solver="lbfgs", tol=1e-8, max_iter=200)
    clf.fit(x / scale, y, sample_weight=sample_weight)
    return clf.coef_[0] / scale


def train(request):
    return _fit_snapshot(request, [r for r in request.records if eligible(r)])


def _fit_snapshot(request, good):
    """Internal fitting primitive; production caller must enforce observation eligibility."""
    good = [r for r in good if request.domain is None or r.domain == request.domain]
    partitions = split(good)
    weights = fit_linear(partitions["train"])
    supported_masks = sorted(
        {candidate_missing_mask(c) for r in partitions["train"] for c in comparison_candidates(r)}
    )
    margin, validation_status = select_margin(partitions["validation"], weights, supported_masks)
    calibration_records = [
        r
        for r in partitions["calibration"]
        if not missing_inputs(r.context, comparison_candidates(r), supported_masks)
    ]
    calibration = fit_calibration(calibration_records, weights)
    evaluation = evaluate(partitions["test"], lambda x: x @ weights, margin, supported_masks)
    scope = (
        sorted(
            {
                r.domain
                for r in calibration_records
                if len({c.family_id for c in calibration_records if c.domain == r.domain}) >= 20
            }
        )
        if calibration
        else []
    )
    calibration_cases = [
        r
        for r in partitions["test"]
        if r.domain in scope
        and not missing_inputs(r.context, comparison_candidates(r), supported_masks)
    ]
    evaluation["calibration"] = calibration_test(calibration_cases, weights, calibration)
    evaluation["calibration"]["out_of_scope_cases"] = sum(
        r.domain not in scope for r in partitions["test"]
    )
    evaluation["calibration"]["uncertain_input_cases"] = sum(
        r.domain in scope
        and bool(missing_inputs(r.context, comparison_candidates(r), supported_masks))
        for r in partitions["test"]
    )
    evaluation["memory_baseline"] = memory_baseline(partitions["train"], partitions["test"])
    evaluation["rule_baseline"] = {"status": "unavailable", "reason": "no_explicit_rule_snapshot"}
    evaluation["abstention_margin"] = margin
    evaluation["threshold_source"] = "validation"
    evaluation["statistically_eligible"] = bool(
        calibration
        and validation_status == "fitted_validation"
        and evaluation["numeric_targets_met"]
        and all(d["numeric_targets_met"] for d in evaluation["by_domain"].values())
    )
    evaluation.update(
        {
            "verified": False,
            "promotion_allowed": False,
            "reason": "independent_scope_policy_and_paired_evidence_required",
        }
    )
    comparison = None
    if request.compare_lightgbm and partitions["train"] and partitions["test"]:
        from lightgbm import LGBMRanker

        rows, labels, groups = [], [], []
        for r in partitions["train"]:
            candidates = comparison_candidates(r)
            rows.extend(features(r.context, c) for c in candidates)
            labels.extend(int(c.id in r.chosen_ids) for c in candidates)
            groups.append(len(candidates))
        ranker = LGBMRanker(
            n_estimators=30,
            num_leaves=7,
            max_depth=3,
            min_child_samples=5,
            n_jobs=1,
            verbosity=-1,
            random_state=17,
        )
        ranker.fit(np.asarray(rows), np.asarray(labels), group=groups)
        comparison = evaluate(partitions["test"], ranker.predict)
        comparison["threshold_source"] = "fixed_zero_not_tuned"
        comparison["promotion_allowed"] = False
    acceptance_memory = [
        {
            "family_id": r.family_id,
            "candidate_id": r.chosen_ids[0],
            "outcome": r.acceptability_outcome,
            "conditions": r.conditions,
            "generalized": False,
        }
        for r in request.records
        if (request.domain is None or r.domain == request.domain)
        and r.answer_kind == "acceptability"
        and r.verification_state == "confirmed"
        and not r.model_exposure
    ]
    return {
        "acceptability_memory": acceptance_memory,
        "calibration_status": "fitted_calibration" if calibration else "insufficient_data",
        "validation_status": validation_status,
        "ok": True,
        "model": Model(
            feature_version=VERSION,
            weights=weights.tolist(),
            calibration=calibration,
            abstention_margin=margin,
            calibration_scope=scope,
            training_domain=request.domain,
            supported_candidate_missing_masks=supported_masks,
        ).model_dump(),
        "evaluation": evaluation,
        "lightgbm": comparison,
        "counts": {"received": len(request.records), "eligible": len(good)},
        "splits": {k: len(v) for k, v in partitions.items()},
        "split_family_ids": {k: sorted({r.family_id for r in v}) for k, v in partitions.items()},
    }


def run(request):
    if request.operation == "train":
        return train(request)
    if request.model is None or request.context is None or not 2 <= len(request.candidates) <= 16:
        raise ValueError("rank input missing")
    if len({c.id for c in request.candidates}) != len(request.candidates):
        raise ValueError("duplicate candidates")
    if request.domain is None or (
        request.model.training_domain is not None
        and request.domain != request.model.training_domain
    ):
        return {
            "ok": True, "ranking": [], "pairwise_probability": None,
            "abstained": True, "missing_inputs": [],
            "abstention_reasons": ["domain_missing" if request.domain is None
                                   else "outside_training_domain"],
            "provisional": request.model.provisional, "probability": None,
        }
    ranking = [
        {
            "candidate_id": c.id,
            "raw_score": float(features(request.context, c) @ request.model.weights),
        }
        for c in request.candidates
    ]
    ordered = sorted(ranking, key=lambda x: -x["raw_score"])
    pair_probability = None
    missing = missing_inputs(
        request.context, request.candidates, request.model.supported_candidate_missing_masks
    )
    margin_abstain = (
        ordered[0]["raw_score"] - ordered[1]["raw_score"] <= request.model.abstention_margin
    )
    if (
        not missing
        and not margin_abstain
        and request.model.calibration is not None
        and len(ranking) == 2
        and request.domain in request.model.calibration_scope
    ):
        pair_probability = {
            "left_candidate_id": ranking[0]["candidate_id"],
            "right_candidate_id": ranking[1]["candidate_id"],
            "value": float(
                expit(
                    request.model.calibration.slope
                    * (ranking[0]["raw_score"] - ranking[1]["raw_score"])
                )
            ),
        }
    return {
        "ok": True,
        "ranking": ordered,
        "pairwise_probability": pair_probability,
        "abstained": bool(missing) or margin_abstain,
        "missing_inputs": missing,
        "abstention_reasons": (["missing_or_uncertain_features"] if missing else [])
        + (["margin_below_threshold"] if margin_abstain else []),
        "provisional": request.model.provisional,
        "probability": None,
    }


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate key")
        result[key] = value
    return result


def main():
    try:
        raw = sys.stdin.buffer.read(8 * 1024 * 1024 + 1)
        if len(raw) > 8 * 1024 * 1024:
            raise ValueError("input limit")
        obj = json.loads(
            raw,
            object_pairs_hook=unique_object,
            parse_constant=lambda _: (_ for _ in ()).throw(ValueError("nonfinite")),
        )
        response = run(Request.model_validate(obj))
        output = json.dumps(response, allow_nan=False, separators=(",", ":"))
        if len(output.encode()) > 1024 * 1024:
            raise ValueError("output limit")
        print(output)
    except ValueError, TypeError, KeyError, OverflowError, RecursionError:
        print('{"ok":false,"error":"invalid_request"}')
        raise SystemExit(2)


if __name__ == "__main__":
    main()
