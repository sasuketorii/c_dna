"""Strict wire contracts shared with Rust and TypeScript via contracts/schemas.

Authority flags are accepted ONLY by the internal training snapshot contract;
they are never accepted by the external observation proposal schema.
"""
from __future__ import annotations
import json
import math
import sys
from functools import lru_cache
from pathlib import Path
from typing import Any
from datetime import datetime
from jsonschema import Draft202012Validator, FormatChecker
from pydantic import BaseModel, ConfigDict, Field, field_validator

MAX_FRAME = 64 * 1024 * 1024

class ContractError(ValueError):
    def __init__(self, code: str, detail: str = "入力の型・範囲を確認してください"):
        super().__init__(detail)
        self.code = code


def asset_path(name: str) -> Path:
    """Bundled resources take priority; development uses the repository contract."""
    bundled = Path(__file__).parent / "assets" / name
    if bundled.is_file():
        return bundled
    root = Path(__file__).resolve().parents[3] / "contracts"
    p = root / name
    if not p.is_file():
        raise ContractError("CONTRACT_MISSING")
    return p


def reject_nonfinite(value: Any, depth: int = 0) -> None:
    if depth > 24:
        raise ContractError("INPUT_INVALID")
    if isinstance(value, float) and not math.isfinite(value):
        raise ContractError("INPUT_INVALID")
    if isinstance(value, dict):
        for v in value.values():
            reject_nonfinite(v, depth + 1)
    elif isinstance(value, list):
        for v in value:
            reject_nonfinite(v, depth + 1)


def _pairs(items: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for k, v in items:
        if k in result:
            raise ContractError("INPUT_INVALID", "重複するJSONキーです")
        result[k] = v
    return result


def loads_strict(raw: str | bytes, limit: int = MAX_FRAME) -> Any:
    if len(raw.encode() if isinstance(raw, str) else raw) > limit:
        raise ContractError("INPUT_TOO_LARGE")
    def invalid(_: str) -> None:
        raise ContractError("INPUT_INVALID")
    try:
        obj = json.loads(raw, parse_constant=invalid, object_pairs_hook=_pairs)
    except (ValueError, UnicodeError, RecursionError) as e:
        if isinstance(e, ContractError):
            raise
        raise ContractError("INPUT_INVALID") from None
    reject_nonfinite(obj)
    return obj


@lru_cache(maxsize=8)
def validator(name: str) -> Draft202012Validator:
    schema = json.loads(asset_path(f"schemas/{name}.schema.json").read_text())
    Draft202012Validator.check_schema(schema)
    return Draft202012Validator(schema, format_checker=FormatChecker())


def validate_rank(data: dict[str, Any]) -> dict[str, Any]:
    reject_nonfinite(data)
    if next(validator("rank-options").iter_errors(data), None):
        raise ContractError("INPUT_INVALID")
    if len(data["context"]["summary"].encode()) > 32768:
        raise ContractError("INPUT_TOO_LARGE")
    ids = [c["id"] for c in data["candidates"]]
    keys = [f["key"] for f in data["context"]["facts"]]
    if len(ids) != len(set(ids)) or len(keys) != len(set(keys)):
        raise ContractError("INPUT_INVALID")
    unknown = set(data["context"]["unknown_fields"])
    for fact in data["context"]["facts"]:
        v = fact["value"]
        if fact["evidence_status"] == "unknown" and v is not None:
            raise ContractError("INPUT_INVALID")
        if fact["key"] in unknown and v is not None:
            raise ContractError("INPUT_INVALID")
        if isinstance(v, dict) and "amount_minor" in v:
            # These supported currency precisions are an explicit v1 contract.
            if v.get("currency") not in {"JPY", "USD", "EUR", "GBP", "KRW", "CNY"}:
                raise ContractError("UNSUPPORTED_CURRENCY")
    return data


class SnapshotRecord(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)
    case_id: str
    family_id: str
    revision: int = Field(ge=1)
    as_of: str
    domain: str
    context: dict[str, Any]
    candidates: list[dict[str, Any]]
    response: dict[str, Any]
    verified: bool
    origin: str
    model_exposure: bool
    case_kind: str
    weight: float = Field(gt=0, le=10)

    @field_validator("as_of")
    @classmethod
    def timezone_required(cls, value: str) -> str:
        dt = datetime.fromisoformat(value.replace("Z", "+00:00"))
        if dt.tzinfo is None:
            raise ValueError("timezone required")
        return value


class Snapshot(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)
    schema_version: str
    snapshot_id: str
    workspace_id: str
    deletion_epoch: int = Field(ge=0)
    feature_version: str
    is_demo: bool
    records: list[SnapshotRecord] = Field(max_length=100000)
    seed: int = Field(default=17, ge=0, le=2**31-1)
    compare_lightgbm: bool = True

    @field_validator("schema_version", "feature_version")
    @classmethod
    def supported_version(cls, value: str) -> str:
        if value != "1.0":
            raise ValueError("unsupported version")
        return value


def verify_snapshot(data: Any) -> Snapshot:
    reject_nonfinite(data)
    try:
        snapshot = Snapshot.model_validate(data)
    except Exception:
        raise ContractError("INPUT_INVALID") from None
    seen: set[str] = set()
    allowed_origins = {"human_ui", "human_confirmed_import"}
    if snapshot.is_demo:
        allowed_origins.add("synthetic_seed")
    for record in snapshot.records:
        if record.case_id in seen:
            raise ContractError("DUPLICATE_CASE")
        seen.add(record.case_id)
        if not record.verified or record.origin not in allowed_origins:
            raise ContractError("INELIGIBLE_TRAINING_DATA")
        if record.case_kind not in {"actual", "hypothetical"}:
            raise ContractError("INPUT_INVALID")
        if len(record.candidates) >= 2:
            validate_rank({"schema_version":"1.0", "request_id":record.case_id,
                "workspace_id":snapshot.workspace_id, "mode":"imitate", "domain":record.domain,
                "context":record.context,"candidates":record.candidates})
        if next(validator("response").iter_errors(record.response), None):
            raise ContractError("INPUT_INVALID")
        ids = [c["id"] for c in record.candidates]
        if len(ids) != len(set(ids)) or not set(record.response["selected_ids"]).issubset(ids):
            raise ContractError("INPUT_INVALID")
    return snapshot
