"""Synthetic oracle benchmark only: never exports a model or asserts human accuracy.

Run from learner: uv run python benchmarks/preference_bench.py --output benchmarks/results.json
"""

import argparse
import hashlib
import json
import platform
import subprocess
import sys
import time
from datetime import UTC, datetime, timedelta
from pathlib import Path

import numpy as np

from cdna_learner.worker import (
    INTERACTIONS,
    KEYS,
    Record,
    Request,
    _fit_snapshot,
    eligible,
    evaluate,
    features,
    pairs,
    run,
)

DOMAINS = (
    "resource_allocation",
    "product_delivery",
    "customer_commercial",
    "organization_delegation",
    "growth_strategy",
    "risk_reputation",
)
ORACLE = np.array(
    [-0.4, -0.25, 0.65, 0.5, 0.3, -0.45, 0, 0, 0, 0, 0, 0, 0.9, 0.7, 0.6, -0.35, -0.8]
)


def generate(count=2000, seed=271828, candidates=4):
    rng = np.random.default_rng(seed)
    records = []
    for i in range(count):
        timestamp = (datetime(2020, 1, 1, tzinfo=UTC) + timedelta(hours=i)).isoformat()
        payload = {
            "family_id": f"synthetic-family-{i // 2}",
            "timestamp": timestamp,
            "domain": DOMAINS[i % len(DOMAINS)],
            "context": {
                "as_of": timestamp,
                "summary": "Synthetic oracle benchmark only",
                "unknown_fields": [],
                "facts": [
                    {
                        "key": key,
                        "value": float(rng.uniform(-1, 1)),
                        "evidence_status": "explicit",
                        "unit": "ratio",
                    }
                    for key, _ in INTERACTIONS
                ],
            },
            "candidates": [
                {
                    "id": f"candidate-{j}",
                    "text": "Synthetic option",
                    "attributes": {key: float(rng.uniform(-1, 1)) for key in KEYS},
                }
                for j in range(candidates)
            ],
            "chosen_ids": ["candidate-0"],
            "verification_state": "unconfirmed",
            "model_exposure": False,
            "answer_kind": "choose_one",
            "case_kind": "hypothetical",
        }
        record = Record.model_validate(payload)
        scores = [features(record.context, c) @ ORACLE for c in record.candidates]
        record.chosen_ids = [record.candidates[int(np.argmax(scores))].id]
        records.append(record)
    return records


def benchmark_fit(records):
    assert all(r.verification_state == "unconfirmed" and not eligible(r) for r in records)
    start = time.perf_counter()
    # Deliberately internal benchmark path. The public API excludes every record.
    result = _fit_snapshot(
        Request(operation="train", feature_version="1.0", records=records), records
    )
    return result, (time.perf_counter() - start) * 1000


def timed_score(records, weights, repeats=5):
    samples = []
    for _ in range(repeats):
        for record in records:
            start = time.perf_counter()
            for candidate in record.candidates:
                float(features(record.context, candidate) @ weights)
            samples.append((time.perf_counter() - start) * 1000)
    return {
        "p50_ms_per_case": float(np.median(samples)),
        "p95_ms_per_case": float(np.quantile(samples, 0.95)),
        "samples": len(samples),
        "includes": "feature extraction and four candidate dot products; excludes IPC",
    }


def cold_start():
    samples = []
    payload = json.dumps({"operation": "train", "feature_version": "1.0", "records": []})
    for _ in range(3):
        start = time.perf_counter()
        child = subprocess.run(
            [sys.executable, "-m", "cdna_learner"],
            input=payload,
            capture_output=True,
            text=True,
            timeout=30,
            check=True,
        )
        assert json.loads(child.stdout)["ok"]
        samples.append((time.perf_counter() - start) * 1000)
    return {
        "worker_empty_train_ms": samples,
        "median_ms": float(np.median(samples)),
        "includes": "fresh process, imports, JSON input/output and empty training",
    }


def scenario_check(path):
    if not path.exists():
        return {"status": "not_available"}
    items = [json.loads(line) for line in path.read_text().splitlines()]
    valid = invalid = mismatches = feature_rejections = 0
    accepted = []
    for item in items:
        assert item["source"]["human_label"] is False
        try:
            record = Record.model_validate(item["record"])
        except ValueError:
            invalid += 1
            mismatches += int(item["expectation"]["record_valid"])
            continue
        valid += 1
        mismatches += int(not item["expectation"]["record_valid"])
        mismatches += int(record.verification_state != "unconfirmed" or eligible(record))
        try:
            vectors = [features(record.context, c) for c in record.candidates]
        except ValueError:
            feature_rejections += 1
            mismatches += int(item["expectation"]["feature_valid"])
            continue
        mismatches += int(not item["expectation"]["feature_valid"])
        oracle = item["oracle"]
        if oracle["expected"] == "rank":
            scores = np.array(vectors) @ np.array(oracle["weights17"])
            best = record.candidates[int(np.argmax(scores))].id
            mismatches += int(best not in oracle["preferred_ids"])
        accepted.append((item, record))
    development = [
        r
        for item, r in accepted
        if item["expectation"]["benchmark_eligible"] and item["split"] != "holdout"
    ]
    holdout = [
        r
        for item, r in accepted
        if item["expectation"]["benchmark_eligible"] and item["split"] == "holdout"
    ]
    fitted, _ = benchmark_fit(development)
    weights = np.array(fitted["model"]["weights"])
    failed_diagnostics = []
    abstentions = ties = 0
    for item, r in accepted:
        if item["split"] != "challenge":
            continue
        expected = item["oracle"]["expected"]
        if expected not in ("abstain", "tie"):
            continue
        response = run(
            Request(
                operation="rank",
                feature_version="1.0",
                model=fitted["model"],
                context=r.context,
                candidates=r.candidates,
                domain=r.domain,
            )
        )
        abstentions += int(expected == "abstain")
        ties += int(expected == "tie")
        if not response["abstained"]:
            failed_diagnostics.append(
                {
                    "case_id": item["case_id"],
                    "expected": expected,
                    "actual": "rank_without_abstention",
                }
            )
    return {
        "status": "checked",
        "valid": valid,
        "invalid": invalid,
        "feature_rejections": feature_rejections,
        "mismatches": mismatches,
        "development_pipeline_splits": fitted["splits"],
        "holdout": evaluate(holdout, lambda x: x @ weights),
        "abstention_challenges": abstentions,
        "tie_challenges": ties,
        "unmet_behavior_diagnostics": failed_diagnostics,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    records = generate()
    # External final holdout is identical for every learning-curve checkpoint.
    holdout = records[1700:]
    learning = []
    full = None
    for size in (0, 8, 32, 128, 512, 1700):
        result, elapsed = benchmark_fit(records[:size])
        weights = np.array(result["model"]["weights"])
        score = lambda x, fitted_weights=weights: x @ fitted_weights
        learning.append(
            {
                "observations": size,
                "fit_ms": elapsed,
                "unconditional": evaluate(holdout, score),
                "selective": evaluate(holdout, score, result["model"]["abstention_margin"]),
                "calibration_status": result["calibration_status"],
                "validation_status": result["validation_status"],
            }
        )
        full = result
    weights = np.array(full["model"]["weights"])
    noisy = [r.model_copy(deep=True) for r in records[:1700]]
    for i, r in enumerate(noisy):
        if i % 5 == 0:
            r.chosen_ids = [next(c.id for c in r.candidates if c.id not in r.chosen_ids)]
    noise_result, noise_ms = benchmark_fit(noisy)
    noisy_weights = np.array(noise_result["model"]["weights"])
    # Dense choose-set cases expose repeated feature extraction costs.
    dense = generate(128, candidates=16)
    for r in dense:
        r.answer_kind = "choose_set"
        r.chosen_ids = [c.id for c in r.candidates[:8]]
    pair_times = []
    for _ in range(3):
        start = time.perf_counter()
        x, _, _ = pairs(dense)
        pair_times.append((time.perf_counter() - start) * 1000)
    output = {
        "schema_version": "1.0",
        "worker_sha256": hashlib.sha256(
            (Path(__file__).parents[1] / "src/cdna_learner/worker.py").read_bytes()
        ).hexdigest(),
        "runtime": {"python": platform.python_version(), "platform": platform.platform()},
        "benchmark_only": True,
        "source": {
            "kind": "synthetic_known_utility",
            "human_label": False,
            "personal_accuracy_evidence": False,
            "seed": 271828,
        },
        "design": {
            "cases": 2000,
            "external_holdout": 300,
            "feature_contract": "1.0",
            "domains": list(DOMAINS),
            "oracle_weights": ORACLE.tolist(),
            "holdout_tuning": False,
            "model_exported": False,
        },
        "learning_curve": learning,
        "internal_final_test": full["evaluation"],
        "latency": timed_score(holdout, weights),
        "cold_start": cold_start(),
        "contradiction_20_percent": {
            "fit_ms": noise_ms,
            "unconditional": evaluate(holdout, lambda x: x @ noisy_weights),
            "selective": evaluate(
                holdout, lambda x: x @ noisy_weights, noise_result["model"]["abstention_margin"]
            ),
        },
        "dense_choose_set": {
            "cases": 128,
            "candidates": 16,
            "pairs": len(x),
            "pair_generation_median_ms": float(np.median(pair_times)),
        },
        "external_scenarios": scenario_check(
            Path("../benchmarks/scenarios/preference_cases.jsonl")
        ),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, indent=2, allow_nan=False) + "\n")
    print(
        json.dumps(
            {
                "output": str(args.output),
                "learning_agreement": [
                    [p["observations"], p["unconditional"]["agreement"]] for p in learning
                ],
                "dense_pair_ms": output["dense_choose_set"]["pair_generation_median_ms"],
                "noise_agreement": output["contradiction_20_percent"]["unconditional"]["agreement"],
            }
        )
    )


if __name__ == "__main__":
    main()
