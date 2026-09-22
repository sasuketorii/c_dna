"""Synthetic domain isolation tests; no human accuracy or promotion evidence."""
import itertools

import pytest
from test_worker import record

from cdna_learner.worker import Request, run

DOMAINS = ("product_delivery", "risk_reputation")


def mixed(count=240):
    return [record(i, chosen="fast" if domain == DOMAINS[0] else "slow", domain=domain,
                   family_id=f"{domain}-{i}")
            for i in range(count) for domain in DOMAINS]


def fit(rows, domain=None):
    return run(Request(operation="train", feature_version="1.0", domain=domain, records=rows))


def rank(model, domain):
    row = record()
    return run(Request(operation="rank", feature_version="1.0", model=model, domain=domain,
                       context=row["context"], candidates=row["candidates"]))


@pytest.mark.parametrize("domain,winner", list(zip(DOMAINS, ("fast", "slow"), strict=True)))
def test_scoped_training_matches_filtered_snapshot_and_retrains(domain, winner):
    rows = mixed()
    result = fit(rows, domain)
    assert result["model"]["training_domain"] == domain
    assert result["model"]["feature_version"] == "1.0"
    assert len(result["model"]["weights"]) == 17
    direct = fit([r for r in rows if r["domain"] == domain], domain)
    assert result["model"] == direct["model"]
    assert result["splits"] == direct["splits"]
    assert result["split_family_ids"] == direct["split_family_ids"]
    assert result["counts"] == {"received": 480, "eligible": 240}
    groups = result["split_family_ids"]
    for a, b in itertools.combinations(groups, 2):
        assert not set(groups[a]) & set(groups[b])
    assert all(f.startswith(domain) for group in groups.values() for f in group)
    answer = rank(result["model"], domain)
    assert not answer["abstained"]
    assert answer["ranking"][0]["candidate_id"] == winner
    corrected = [{**r, "chosen_ids": ["slow" if winner == "fast" else "fast"]}
                 if r["domain"] == domain else r for r in rows]
    assert rank(fit(corrected, domain)["model"], domain)["ranking"][0]["candidate_id"] != winner
    foreign = next(d for d in DOMAINS if d != domain)
    assert fit(corrected, foreign)["model"] == fit(rows, foreign)["model"]


@pytest.mark.parametrize("count", [0, 1, 240])
def test_scope_survives_without_calibration_and_blocks_other_or_missing_domain(count):
    model = fit(mixed(count), DOMAINS[0])["model"]
    assert model["training_domain"] == DOMAINS[0]
    for domain in (DOMAINS[1], None):
        answer = rank(model, domain)
        assert answer["abstained"]
        assert answer["ranking"] == []
        assert answer["pairwise_probability"] is None
    if count < 20:
        assert model["calibration"] is None
        assert model["calibration_scope"] == []


def test_pooled_default_retains_opposite_preference_limitation():
    result = fit(mixed())
    assert result["model"]["training_domain"] is None
    assert result["counts"]["eligible"] == 480
    answers = [rank(result["model"], domain) for domain in DOMAINS]
    assert answers[0]["ranking"] == answers[1]["ranking"]
    assert all(answer["abstained"] for answer in answers)


@pytest.mark.parametrize("domain", ["", "unknown", 123])
def test_invalid_domain_is_rejected(domain):
    with pytest.raises(ValueError):
        fit([], domain)


def test_scoped_training_still_excludes_untrusted_records_and_acceptability():
    rows = mixed(1)
    rows[0]["verification_state"] = "unconfirmed"
    assert fit(rows, DOMAINS[0])["counts"]["eligible"] == 0
    rows[0]["verification_state"] = "confirmed"
    rows[0]["model_exposure"] = True
    assert fit(rows, DOMAINS[0])["counts"]["eligible"] == 0
    for row in rows:
        row.update(answer_kind="acceptability", acceptability_outcome="accept", model_exposure=False)
    result = fit(rows, DOMAINS[0])
    assert len(result["acceptability_memory"]) == 1
    assert result["acceptability_memory"][0]["family_id"].startswith(DOMAINS[0])


def test_artifact_cannot_expand_calibration_outside_training_domain():
    model = fit(mixed(), DOMAINS[0])["model"]
    model["calibration_scope"] = [DOMAINS[1]]
    with pytest.raises(ValueError):
        rank(model, DOMAINS[0])
