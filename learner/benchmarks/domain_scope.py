"""Independent synthetic holdout for explicit domain scope, never promotion evidence."""
import hashlib
import json
import time
from datetime import UTC, datetime, timedelta
from pathlib import Path

import numpy as np

from cdna_learner.worker import KEYS, Record, Request, _fit_snapshot, run

DOMAINS = ("product_delivery", "risk_reputation")


def generate(seed, count, offset):
    rng = np.random.default_rng(seed)
    rows = []
    for i in range(count):
        stamp = (datetime(2020, 1, 1, tzinfo=UTC) + timedelta(days=offset+i)).isoformat()
        attrs = rng.uniform(-1, 1, (4, 6))
        for domain in DOMAINS:
            # Both domains receive exactly the same feature vectors. The independent
            # policy selects maximum speed in delivery and minimum speed in risk.
            winner = int(np.argmax(attrs[:, 2]) if domain == DOMAINS[0]
                         else np.argmin(attrs[:, 2]))
            rows.append(Record.model_validate({
                "family_id": f"{seed}-{domain}-{i}", "timestamp": stamp, "domain": domain,
                "context": {"as_of": stamp, "summary": "Synthetic opposite domain policy",
                            "facts": [], "unknown_fields": []},
                "candidates": [{"id": f"c{j}", "text": "Simulated option",
                                "attributes": dict(zip(KEYS, map(float, a), strict=True))}
                               for j, a in enumerate(attrs)],
                "chosen_ids": [f"c{winner}"], "verification_state": "unconfirmed",
                "model_exposure": False, "answer_kind": "choose_one",
                "case_kind": "hypothetical",
            }))
    return rows


def main():
    start = time.perf_counter()
    rows = generate(95173, 600, 0)
    artifacts, timings = {}, {}
    for domain in (None, *DOMAINS):
        request = Request(operation="train", feature_version="1.0", domain=domain, records=rows)
        assert run(request)["counts"]["eligible"] == 0
        fit_start = time.perf_counter()
        artifacts[domain] = _fit_snapshot(request, rows)
        timings[str(domain)] = (time.perf_counter()-fit_start)*1000
    # Holdout materialized only after all artifacts freeze; no hyperparameter search.
    holdout = generate(86249, 400, 2000)
    results = {}
    for mode in ("pooled", "domain_scoped"):
        correct = answered = selective_correct = ood_abstained = 0
        latencies = []
        for row in holdout:
            model = artifacts[None if mode == "pooled" else row.domain]["model"]
            request = Request(operation="rank", feature_version="1.0", model=model,
                              domain=row.domain, context=row.context, candidates=row.candidates)
            tick = time.perf_counter()
            result = run(request)
            latencies.append((time.perf_counter()-tick)*1000)
            hit = result["ranking"][0]["candidate_id"] in row.chosen_ids
            correct += hit
            answered += not result["abstained"]
            selective_correct += hit and not result["abstained"]
            foreign = request.model_copy(update={"domain": "growth_strategy"})
            ood_abstained += run(foreign)["abstained"]
        results[mode] = {
            "cases": len(holdout), "raw_top1_accuracy": correct/len(holdout),
            "coverage": answered/len(holdout),
            "selective_accuracy": selective_correct/answered if answered else None,
            "ood_abstention_rate": ood_abstained/len(holdout),
            "rank_ms_median": float(np.median(latencies)),
            "rank_ms_p95": float(np.percentile(latencies, 95)),
        }
    output = {
        "evidence": "synthetic independent policy holdout; not human accuracy",
        "promotion_allowed": False, "feature_version": "1.0", "weights": 17,
        "fit_seed": 95173, "holdout_seed": 86249,
        "source_sha256": {p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
                          for p in ("learner/src/cdna_learner/worker.py",
                                    "learner/benchmarks/domain_scope.py")},
        "training_records_total": len(rows), "fit_ms": timings,
        "splits": {str(d): a["splits"] for d, a in artifacts.items()},
        "results": results, "wall_ms_in_process": (time.perf_counter()-start)*1000,
        "timing_scope": "Single process, imports excluded; rank excludes Request validation; no IPC or Rust latency claim",
    }
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
