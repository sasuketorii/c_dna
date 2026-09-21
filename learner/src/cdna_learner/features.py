"""Fixed, explainable context×option features; no fitting on evaluation data.

IDs, display order, notes, outcomes and free text are not predictive features.
The exact traversal order is part of the Rust/Python/Mojo parity contract.
"""
from __future__ import annotations
from functools import lru_cache
import hashlib
import json
import math
from typing import Any
import numpy as np
from .contracts import asset_path, ContractError

@lru_cache(maxsize=1)
def manifest() -> dict[str, Any]:
    return json.loads(asset_path("features.json").read_text())


def manifest_hash() -> str:
    return hashlib.sha256(asset_path("features.json").read_bytes()).hexdigest()


def feature_names() -> list[str]:
    m = manifest()
    ca, cx, ds = m["candidate_features"], m["context_features"], m["domains"]
    return ([f"candidate:{a}" for a in ca] + [f"candidate_missing:{a}" for a in ca]
        + [f"context:{c['key']}*{a}" for c in cx for a in ca]
        + [f"context_missing:{c['key']}*{a}" for c in cx for a in ca]
        + [f"domain:{d}*{a}" for d in ds for a in ca])


def context_values(context: dict[str, Any]) -> tuple[list[float], list[float], list[str]]:
    facts = {f["key"]: f for f in context["facts"]}
    out: list[float] = []
    missing: list[float] = []
    out_of_range: list[str] = []
    for cfg in manifest()["context_features"]:
        fact = facts.get(cfg["key"])
        if (fact is None or fact.get("value") is None or fact.get("evidence_status") != "explicit"
                or cfg["key"] in context["unknown_fields"]):
            out.append(0.0); missing.append(1.0); continue
        value = fact["value"]
        if cfg["kind"] == "category":
            if not isinstance(value, str) or value not in cfg["values"]:
                out.append(0.0); missing.append(1.0); out_of_range.append(cfg["key"]); continue
            out.append(cfg["values"][value]); missing.append(0.0)
        else:
            if isinstance(value, bool) or not isinstance(value, (int,float)) or not math.isfinite(value):
                raise ContractError("INPUT_INVALID")
            if cfg.get("unit") and fact.get("unit") != cfg["unit"]:
                raise ContractError("UNIT_REQUIRED")
            if not cfg["minimum"] <= value <= cfg["maximum"]:
                out_of_range.append(cfg["key"])
            out.append(max(0.0, min(1.0, value / cfg["scale"]))); missing.append(0.0)
    return out, missing, out_of_range


def transform(domain: str, context: dict[str,Any], candidates: list[dict[str,Any]]) -> np.ndarray:
    m = manifest()
    if domain not in m["domains"]:
        raise ContractError("UNSUPPORTED_DOMAIN")
    cx, cm, _ = context_values(context)
    rows: list[list[float]] = []
    for candidate in candidates:
        attrs = candidate["attributes"]
        a: list[float] = []
        am: list[float] = []
        for key in m["candidate_features"]:
            v = attrs.get(key)
            if v is None:
                a.append(0.0); am.append(1.0)
            elif isinstance(v, bool) or not isinstance(v,(float,int)) or not math.isfinite(v) or not 0 <= v <= 1:
                raise ContractError("INVALID_CANDIDATE_FEATURE")
            else:
                a.append(float(v)); am.append(0.0)
        rows.append(a + am + [c*v for c in cx for v in a] + [c*v for c in cm for v in a]
                    + [float(d == domain)*v for d in m["domains"] for v in a])
    return np.asarray(rows, dtype=np.float64).reshape(len(rows), len(feature_names()))


def score(weights: list[float], domain: str, context: dict[str,Any], candidates: list[dict[str,Any]]) -> np.ndarray:
    x = transform(domain, context, candidates)
    w = np.asarray(weights,dtype=np.float64)
    if w.shape != (x.shape[1],) or not np.isfinite(w).all():
        raise ContractError("MODEL_INVALID")
    result = x @ w
    if not np.isfinite(result).all():
        raise ContractError("MODEL_INVALID")
    return result
