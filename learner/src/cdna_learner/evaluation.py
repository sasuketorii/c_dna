"""Time-ordered family splits and honest selective-risk reporting."""
from __future__ import annotations
from collections import defaultdict
from dataclasses import dataclass
from datetime import datetime
import math
from typing import Callable, Any
import numpy as np
from .contracts import SnapshotRecord
from .pairwise import comparisons

@dataclass
class Split:
    train: list[SnapshotRecord]
    validation: list[SnapshotRecord]
    calibration: list[SnapshotRecord]
    test: list[SnapshotRecord]
    quarantined: list[SnapshotRecord]
    policy: str = 'family_interval_temporal_v1'


def split_records(records: list[SnapshotRecord]) -> Split:
    # Cases shown model output are eligible for learning, not independent evaluation.
    # The family interval must fit wholly inside one time window; boundary-spanning
    # families are quarantined rather than moving later facts into an early split.
    families: dict[str,list[SnapshotRecord]] = defaultdict(list)
    for r in records:
        if comparisons(r):
            families[r.family_id].append(r)
    ordered = sorted((r for rs in families.values() for r in rs),
                     key=lambda r:(datetime.fromisoformat(r.as_of.replace('Z','+00:00')), r.case_id))
    if len(families) < 8 or len(ordered) < 24:
        return Split(ordered,[],[],[],[])
    moments = [datetime.fromisoformat(r.as_of.replace('Z','+00:00')) for r in ordered]
    boundaries = [moments[min(len(moments)-1,int(len(moments)*q))] for q in (.60,.75,.85)]
    partitions: list[list[SnapshotRecord]] = [[],[],[],[]]
    quarantine = []
    for rs in families.values():
        buckets = {sum(datetime.fromisoformat(r.as_of.replace('Z','+00:00')) >= b for b in boundaries) for r in rs}
        if len(buckets) != 1:
            quarantine.extend(rs); continue
        bucket = buckets.pop()
        if bucket and any(r.model_exposure for r in rs):
            quarantine.extend(rs); continue
        partitions[bucket].extend(rs)
    for p in partitions:
        p.sort(key=lambda r:(r.as_of,r.case_id))
    return Split(*partitions,quarantine)


def wilson_lower(successes: int, total: int, z: float=1.959963984540054) -> float | None:
    if not total:
        return None
    p = successes/total
    return (p + z*z/(2*total)-z*math.sqrt(p*(1-p)/total+z*z/(4*total*total)))/(1+z*z/total)


def evaluate(records: list[SnapshotRecord], scorer: Callable[[SnapshotRecord],np.ndarray],
             margin_threshold: float=1e-9) -> dict[str,Any]:
    counts: dict[str,dict[str,int]] = {}
    details = []
    for record in records:
        if record.model_exposure or not comparisons(record):
            continue
        values = scorer(record)
        order = np.argsort(-values,kind='stable')
        margin = float(values[order[0]]-values[order[1]])
        selected = record.candidates[int(order[0])]['id']
        correct = selected in record.response['selected_ids']
        answered = margin > margin_threshold
        d = counts.setdefault(record.domain,{'total':0,'answered':0,'correct':0,'raw_correct':0})
        d['total'] += 1; d['answered'] += int(answered); d['correct'] += int(correct and answered); d['raw_correct'] += int(correct)
        details.append({'case_id':record.case_id,'family_id':record.family_id,'correct':bool(correct),'margin':margin,'candidate_count':len(values),'answered':bool(answered)})
    def aggregate(c: dict[str,int]) -> dict[str,Any]:
        n,a,k = c['total'],c['answered'],c['correct']
        return {**c,'coverage':a/n if n else None,'selective_agreement':k/a if a else None,
                'top1_agreement':c['raw_correct']/n if n else None,'wilson_lower_95':wilson_lower(k,a),
                'selective_error':1-k/a if a else None}
    overall = {k:sum(v[k] for v in counts.values()) for k in ('total','answered','correct','raw_correct')}
    return {'overall':aggregate(overall),'domains':{k:aggregate(v) for k,v in counts.items()},
            'details':details,'scope':'unexposed_confirmed_cases','margin_threshold':margin_threshold}


def verified_claim_allowed(report: dict[str,Any], is_demo: bool, policy_violations: int=0) -> bool:
    c = report['overall']
    return (not is_demo and c['total']>=200 and c['answered']>=100 and c['coverage']>=.6
            and c['selective_agreement']>=.9 and c['wilson_lower_95']>=.85 and policy_violations==0)
