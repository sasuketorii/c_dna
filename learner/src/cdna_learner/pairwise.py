from __future__ import annotations
from dataclasses import dataclass
from typing import Iterable
import numpy as np
from .contracts import SnapshotRecord, ContractError
from .features import transform, feature_names

@dataclass(frozen=True)
class PairBatch:
    x: np.ndarray
    y: np.ndarray
    weights: np.ndarray
    case_ids: tuple[str,...]


def comparisons(record: SnapshotRecord) -> list[tuple[int,int]]:
    r = record.response
    ids = [c['id'] for c in record.candidates]
    chosen = r['selected_ids']
    if r['kind'] == 'choose_one':
        if len(chosen) != 1:
            raise ContractError('INVALID_RESPONSE')
    elif r['kind'] == 'choose_set':
        if not chosen:
            raise ContractError('INVALID_RESPONSE')
    elif r['kind'] == 'pairwise':
        if len(ids) != 2:
            raise ContractError('INVALID_RESPONSE')
        if r.get('value') in {'tie','insufficient'}:
            return []
        if r.get('value') not in {'left','right'}:
            raise ContractError('INVALID_RESPONSE')
        # "left"/"right" is canonical presentation captured by the core. chosen IDs
        # are mandatory, so display reordering cannot silently relabel this response.
        if len(chosen) != 1:
            raise ContractError('INVALID_RESPONSE')
    else:
        return []
    selected = set(chosen)
    if not selected.issubset(ids):
        raise ContractError('INVALID_RESPONSE')
    return [(i,j) for i,a in enumerate(ids) for j,b in enumerate(ids)
            if a in selected and b not in selected]


def make_pairs(records: Iterable[SnapshotRecord]) -> PairBatch:
    rows, labels, weights, case_ids = [], [], [], []
    for record in records:
        edges = comparisons(record)
        if not edges:
            continue
        x = transform(record.domain,record.context,record.candidates)
        # A single human decision carries total weight record.weight, not N*(N-1).
        weight = record.weight/(2*len(edges))
        for i,j in edges:
            delta = x[i]-x[j]
            rows.extend([delta,-delta]); labels.extend([1,0])
            weights.extend([weight,weight]); case_ids.extend([record.case_id,record.case_id])
    d = len(feature_names())
    return PairBatch(np.asarray(rows,dtype=np.float64).reshape(-1,d),
                     np.asarray(labels,dtype=np.int64),np.asarray(weights,dtype=np.float64),tuple(case_ids))
