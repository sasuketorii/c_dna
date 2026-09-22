import itertools
import json
import subprocess
import sys
from datetime import UTC, datetime, timedelta

import numpy as np
import pytest

from cdna_learner.worker import Record, Request, features, pairs, run, split


def record(i=0, chosen="fast", **overrides):
    stamp = (datetime(2025, 1, 1, tzinfo=UTC) + timedelta(days=i)).isoformat()
    value = {
        "family_id": f"family-{i}",
        "timestamp": stamp,
        "domain": "product_delivery",
        "context": {
            "as_of": stamp,
            "summary": "deadline",
            "unknown_fields": [],
            "facts": [
                {
                    "key": "deadline_pressure",
                    "value": 1,
                    "evidence_status": "explicit",
                    "unit": "ratio",
                }
            ],
        },
        "candidates": [
            {"id": "fast", "text": "Fast", "attributes": {"speed": 1}},
            {"id": "slow", "text": "Slow", "attributes": {"speed": -1}},
        ],
        "chosen_ids": [chosen],
        "verification_state": "confirmed",
        "model_exposure": False,
        "answer_kind": "choose_one",
    }
    for candidate in value["candidates"]:
        for key in ("cost", "effort", "reuse", "customer_impact", "irreversibility"):
            candidate["attributes"].setdefault(key, 0)
    value.update(overrides)
    return value


def train(records):
    return run(Request(operation="train", feature_version="1.0", records=records))


def test_teaching_changes_ranking():
    positive = train([record()])["model"]["weights"]
    negative = train([record(chosen="slow")])["model"]["weights"]
    r = Record.model_validate(record())
    delta = features(r.context, r.candidates[0]) - features(r.context, r.candidates[1])
    assert delta @ positive > 0 > delta @ negative


def test_noneligible_and_ties_are_not_negatives():
    excluded = [
        record(verification_state="unconfirmed"),
        record(model_exposure=True),
        record(answer_kind="tie", chosen_ids=["fast", "slow"]),
        record(answer_kind="skip", chosen_ids=[]),
        record(verification_state="deleted"),
    ]
    assert train(excluded)["counts"]["eligible"] == 0
    assert train([record(), *excluded])["model"] == train([record()])["model"]


def test_pair_weights_and_symmetry():
    x, y, weights = pairs([Record.model_validate(record())])
    np.testing.assert_array_equal(x[0], -x[1])
    assert list(y) == [1, 0]
    assert sum(weights) == 1


def test_context_changes_pair_difference_and_text_ids_do_not():
    r = Record.model_validate(record())
    before = features(r.context, r.candidates[0])
    r.candidates[0].id = "different"
    r.candidates[0].text = "outcome winner company identity"
    r.candidates[0].attributes.update(outcome=1, reason="winner", candidate_id="leak")
    np.testing.assert_array_equal(before, features(r.context, r.candidates[0]))
    r.context.facts[0].value = -1
    assert not np.array_equal(before, features(r.context, r.candidates[0]))


def test_family_temporal_separation():
    rows = [Record.model_validate(record(i)) for i in range(100)]
    rows[0].family_id = rows[-1].family_id
    result = split(rows)
    assert len(result["purged"]) == 2
    names = ["train", "validation", "calibration", "test"]
    for left, right in itertools.pairwise(names):
        assert max(r.timestamp for r in result[left]) < min(r.timestamp for r in result[right])
        assert not {r.family_id for r in result[left]} & {r.family_id for r in result[right]}


def test_validation_and_provisional():
    with pytest.raises(ValueError):
        Request(operation="train", feature_version="2.0")
    with pytest.raises(ValueError):
        Record.model_validate(record(chosen_ids=["missing"]))
    model = train([record()])
    assert model["model"]["calibration"] is None
    assert not model["evaluation"]["promotion_allowed"]


def test_lightgbm_real_comparison():
    result = run(
        Request(
            operation="train",
            feature_version="1.0",
            records=[record(i) for i in range(40)],
            compare_lightgbm=True,
        )
    )
    assert result["lightgbm"]["cases"] > 0
    assert result["lightgbm"]["promotion_allowed"] is False


@pytest.mark.parametrize("payload", ['{"operation":"train","operation":"rank"}', '{"x":NaN}'])
def test_cli_invalid_json(payload):
    result = subprocess.run(
        [sys.executable, "-m", "cdna_learner.worker"],
        input=payload,
        check=False,
        capture_output=True,
        text=True,
        timeout=20,
    )
    assert result.returncode == 2
    assert json.loads(result.stdout) == {"ok": False, "error": "invalid_request"}


def test_fixed_module_entrypoint():
    result = subprocess.run(
        [sys.executable, "-m", "cdna_learner"],
        input=json.dumps({"operation": "train", "feature_version": "1.0", "records": [record()]}),
        capture_output=True,
        text=True,
        timeout=20,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout)["counts"]["eligible"] == 1


def test_pairwise_only_compared_pair_and_right_choice():
    value = record(
        answer_kind="pairwise",
        left_id="fast",
        right_id="slow",
        preference="right",
        chosen_ids=["slow"],
    )
    value["candidates"].append(
        {"id": "uncompared", "text": "Not compared", "attributes": {"speed": 0}}
    )
    x, _y, weights = pairs([Record.model_validate(value)])
    assert len(x) == 2
    assert sum(weights) == 1
    result = train([value])
    r = Record.model_validate(record())
    assert (features(r.context, r.candidates[0]) - features(r.context, r.candidates[1])) @ result[
        "model"
    ]["weights"] < 0
    for preference in ("tie", "insufficient"):
        value.update(preference=preference, chosen_ids=[])
        assert train([value])["counts"]["eligible"] == 0


def test_choose_set_cross_boundary_constraints_only():
    value = record(answer_kind="choose_set", chosen_ids=["fast", "also_fast"])
    value["candidates"].extend(
        [
            {"id": "also_fast", "text": "Also fast", "attributes": {"speed": 0.5}},
            {"id": "also_slow", "text": "Also slow", "attributes": {"speed": -0.5}},
        ]
    )
    x, _y, weights = pairs([Record.model_validate(value)])
    assert len(x) == 8  # 2 allowed x 2 unselected x 2 symmetric directions
    assert sum(weights) == 1
    assert np.all(x[::2, 2] > 0)
    value["chosen_ids"] = [c["id"] for c in value["candidates"]]
    assert train([value])["counts"]["eligible"] == 0


def test_conditional_acceptability_preserves_conditions_without_rank_labels():
    value = record(
        answer_kind="acceptability",
        acceptability_outcome="conditional",
        conditions=["Only after written approval"],
    )
    result = train([value])
    assert result["counts"]["eligible"] == 0
    assert result["model"]["weights"] == [0.0] * 17
    assert result["acceptability_memory"][0]["conditions"] == value["conditions"]
    assert result["acceptability_memory"][0]["outcome"] == "conditional"
    assert result["validation_status"] == "insufficient_data"
    assert result["calibration_status"] == "insufficient_data"
    value["conditions"] = []
    with pytest.raises(ValueError):
        Record.model_validate(value)


def test_context_feature_units_are_not_interchangeable():
    r = Record.model_validate(record())
    assert features(r.context, r.candidates[0])[12] == 1
    for unit, status, value in [
        ("day", "inferred", 1),
        (None, "unknown", 1),
        ("ratio", "inferred", 1),
        ("ratio", "unknown", None),
        ("ratio", "explicit", None),
    ]:
        r.context.facts[0].unit = unit
        r.context.facts[0].evidence_status = status
        r.context.facts[0].value = value
        assert features(r.context, r.candidates[0])[12] == 0


@pytest.mark.parametrize("unit,value", [("day", 1), (None, 1), ("ratio", 2), ("ratio", True)])
def test_invalid_explicit_numeric_interactions_are_rejected(unit, value):
    payload = record()
    payload["context"]["facts"][0].update(unit=unit, value=value)
    with pytest.raises(ValueError):
        Record.model_validate(payload)
    row = Record.model_validate(record())
    row.context.facts[0].unit = unit
    row.context.facts[0].value = value
    with pytest.raises(ValueError):
        features(row.context, row.candidates[0])


def test_calibration_uses_only_calibration_and_margin_only_validation():
    rows = [record(i, chosen="slow" if i % 10 == 0 else "fast") for i in range(240)]
    original = train(rows)
    assert original["model"]["calibration"]["source"] == "calibration"
    assert original["calibration_status"] == "fitted_calibration"
    partitions = original["split_family_ids"]
    for left, right in itertools.combinations(partitions, 2):
        assert not set(partitions[left]) & set(partitions[right])
    assert original["model"]["calibration"]["family_ids"] == partitions["calibration"]
    changed = [
        dict(r, chosen_ids=["slow"]) if r["family_id"] in partitions["test"] else r for r in rows
    ]
    after = train(changed)
    assert after["model"] == original["model"]
    assert after["evaluation"]["correct"] != original["evaluation"]["correct"]
    assert original["evaluation"]["calibration"]["status"] == "heldout_test"
    assert not original["evaluation"]["promotion_allowed"]


def test_artifact_validation_and_pairwise_probability_scope():
    from cdna_learner.worker import Model

    for value in (float("nan"), float("inf"), 1e100):
        with pytest.raises(ValueError):
            Model(feature_version="1.0", weights=[value] * 17)
    artifact = train([record(i) for i in range(240)])["model"]
    r = record()
    payload = {
        "operation": "rank",
        "feature_version": "1.0",
        "model": artifact,
        "context": r["context"],
        "candidates": r["candidates"],
        "domain": r["domain"],
    }
    answer = run(Request(**payload))
    assert 0 <= answer["pairwise_probability"]["value"] <= 1
    assert answer["probability"] is None
    payload["domain"] = "risk_reputation"
    assert run(Request(**payload))["pairwise_probability"] is None


def test_sparse_calibration_domain_is_not_authorized():
    rows = [record(i) for i in range(240)]
    rows[185]["domain"] = "risk_reputation"
    result = train(rows)
    assert result["model"]["calibration_scope"] == ["product_delivery"]
    assert "family-185" not in result["model"]["calibration"]["family_ids"]


def test_calibration_and_margin_missing_families_fail_closed():
    from cdna_learner.worker import fit_calibration, select_margin

    rows = [Record.model_validate(record(i)) for i in range(25)]
    for row in rows:
        row.family_id = "one-family"
    assert fit_calibration(rows, np.ones(17)) is None
    assert select_margin(rows, np.ones(17)) == (0.0, "insufficient_data")


def test_synthetic_benchmark_data_never_trains_through_production_api():
    result = train([record(i, verification_state="unconfirmed") for i in range(30)])
    assert result["counts"]["eligible"] == 0
    assert result["model"]["weights"] == [0.0] * 17


def test_dense_pairs_keep_reference_semantics():
    from cdna_learner.worker import comparisons

    payload = record(answer_kind="choose_set", chosen_ids=["fast", "third"])
    payload["candidates"].extend(
        [
            {"id": "third", "text": "Third", "attributes": {"speed": 0.3}},
            {"id": "fourth", "text": "Fourth", "attributes": {"speed": -0.3}},
        ]
    )
    row = Record.model_validate(payload)
    x, y, weights = pairs([row])
    expected = []
    for left, right in comparisons(row):
        delta = features(row.context, left) - features(row.context, right)
        expected.extend((delta, -delta))
    np.testing.assert_array_equal(x, np.array(expected))
    np.testing.assert_array_equal(y, [1, 0] * 4)
    assert sum(weights) == 1


def test_supported_missing_patterns_and_optional_context():
    rows = [record(i) for i in range(8)]
    for row in rows:
        row["context"]["facts"] = []  # Unspecified optional context is not unknown.
        for candidate in row["candidates"]:
            candidate["attributes"] = {"speed": candidate["attributes"]["speed"]}
    fitted = train(rows)["model"]
    assert fitted["supported_candidate_missing_masks"] == [59]
    payload = {
        "operation": "rank",
        "feature_version": "1.0",
        "model": fitted,
        "domain": rows[0]["domain"],
        "context": rows[0]["context"],
        "candidates": rows[0]["candidates"],
    }
    assert run(Request(**payload))["abstained"] is False
    payload["context"]["unknown_fields"] = ["deadline_pressure"]
    answer = run(Request(**payload))
    assert answer["abstained"] is True
    assert answer["missing_inputs"] == ["context:deadline_pressure"]


def test_unseen_missing_pattern_abstains_and_suppresses_probability():
    rows = [record(i) for i in range(240)]
    fitted = train(rows)["model"]
    assert fitted["supported_candidate_missing_masks"] == [0]
    rows[0]["candidates"][0]["attributes"].pop("cost")
    answer = run(
        Request(
            operation="rank",
            feature_version="1.0",
            model=fitted,
            context=rows[0]["context"],
            candidates=rows[0]["candidates"],
            domain="product_delivery",
        )
    )
    assert answer["abstained"] is True
    assert "candidate:fast:cost" in answer["missing_inputs"]
    assert answer["pairwise_probability"] is None
    assert answer["ranking"][0]["raw_score"] is not None


def test_training_is_invariant_to_record_and_candidate_order():
    rows = [record(i, chosen="slow" if i % 3 == 0 else "fast") for i in range(12)]
    # Same-time, contradictory observations previously made SGD depend on input order.
    for row in rows:
        row["timestamp"] = rows[0]["timestamp"]
        row["context"]["as_of"] = rows[0]["timestamp"]
    original = train(rows)
    permuted = json.loads(json.dumps(list(reversed(rows))))
    for row in permuted:
        row["candidates"].reverse()
    repeated = train(permuted)
    assert original["model"] == repeated["model"]
    assert original["split_family_ids"] == repeated["split_family_ids"]


def test_correction_replay_excludes_superseded_and_is_reversible():
    original = [record(i) for i in range(8)]
    corrected = [record(i, chosen="slow") for i in range(8)]
    old = [record(i, verification_state="superseded") for i in range(8)]
    before = train(original)["model"]
    after = train([*old, *corrected])
    assert after["model"] == train(corrected)["model"]
    assert after["counts"] == {"received": 16, "eligible": 8}
    r = Record.model_validate(record())
    direction = features(r.context, r.candidates[0]) - features(r.context, r.candidates[1])
    assert direction @ before["weights"] > 0 > direction @ after["model"]["weights"]
    assert train(original)["model"] == before
    assert after["model"]["calibration"] is None
    assert after["model"]["provisional"] is True


def test_scaling_is_train_only_and_test_labels_cannot_change_artifact():
    rows = [record(i) for i in range(100)]
    baseline = train(rows)["model"]
    altered = json.loads(json.dumps(rows))
    for row in altered[85:]:
        row["chosen_ids"] = ["slow"]
        row["candidates"][0]["attributes"]["speed"] = 0.000001
        row["candidates"][1]["attributes"]["speed"] = -0.000001
    assert train(altered)["model"] == baseline


def test_narrow_signal_and_folded_weight_export_are_usable_without_scaler():
    rows = [record(i) for i in range(12)]
    for i, row in enumerate(rows):
        row["context"]["facts"] = []
        for j, c in enumerate(row["candidates"]):
            c["attributes"]["speed"] *= 0.02
            # Distracting effort reverses across examples; the tiny speed is always preferred.
            c["attributes"]["effort"] = (-1 if i % 2 else 1) * (1 if j else -1)
    model = train(rows)["model"]
    for row in rows:
        ranked = run(
            Request(
                operation="rank",
                feature_version="1.0",
                model=model,
                domain=row["domain"],
                context=row["context"],
                candidates=row["candidates"],
            )
        )
        assert ranked["ranking"][0]["candidate_id"] == "fast"
        assert ranked["pairwise_probability"] is None
    assert len(model["weights"]) == 17
    assert all(np.isfinite(model["weights"]))


@pytest.mark.parametrize("count", [19, 20, 40, 200])
def test_equal_timestamps_remain_provisional_train_only(count):
    rows = [record(0, family_id=f"imported-{i}") for i in range(count)]
    result = train(rows)
    assert result["splits"] == {
        "train": count,
        "validation": 0,
        "calibration": 0,
        "test": 0,
        "purged": 0,
    }
    assert result["model"]["weights"][2] > 0
    assert result["model"]["calibration"] is None
    assert result["model"]["provisional"] is True
    assert result["evaluation"]["cases"] == 0
    assert result["evaluation"]["statistically_eligible"] is False
    assert result["evaluation"]["promotion_allowed"] is False
    assert result["validation_status"] == "insufficient_data"


def test_collapsed_cutoffs_keep_crossing_families_train_only_without_leakage():
    rows = [record(day, family_id=f"family-{i}") for day in (0, 100) for i in range(20)]
    partitions = split([Record.model_validate(r) for r in rows])
    assert len(partitions["train"]) == 40
    assert all(not partitions[name] for name in ("validation", "calibration", "test", "purged"))
