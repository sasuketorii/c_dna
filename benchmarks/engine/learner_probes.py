#!/usr/bin/env python3
"""Reproduce sample-efficiency boundaries through production learner Request/run."""
import hashlib
import json
from datetime import UTC, datetime, timedelta
from pathlib import Path
from cdna_learner.worker import Request, Record, features, run

ROOT = Path(__file__).resolve().parents[2]
source = ROOT / 'learner/src/cdna_learner/worker.py'
before = hashlib.sha256(source.read_bytes()).hexdigest()

def record(i, same_time=False):
    stamp = (datetime(2025, 1, 1, tzinfo=UTC) + timedelta(days=0 if same_time else i)).isoformat()
    attrs = {k: 0 for k in ('cost','effort','reuse','customer_impact','irreversibility')}
    return {'family_id':f'family-{i}', 'timestamp':stamp, 'domain':'product_delivery',
            'context':{'as_of':stamp,'summary':'Synthetic choose speed','facts':[{'key':'deadline_pressure','value':1,'unit':'ratio','evidence_status':'explicit'}],'unknown_fields':[]},
            'candidates':[{'id':'fast','text':'Fast','attributes':{**attrs,'speed':1}}, {'id':'slow','text':'Slow','attributes':{**attrs,'speed':-1}}],
            'chosen_ids':['fast'],'verification_state':'confirmed','model_exposure':False,'answer_kind':'choose_one'}

def probe(n, same_time):
    rows = [record(i, same_time) for i in range(n)]
    output = run(Request(operation='train',feature_version='1.0',records=rows))
    r = Record.model_validate(rows[0])
    delta = features(r.context,r.candidates[0])-features(r.context,r.candidates[1])
    margin = float(delta @ output['model']['weights'])
    return {'families':n,'same_timestamp':same_time,'splits':output['splits'],'fast_minus_slow_score':margin,
            'learned_preference':margin>0,'calibration_status':output['calibration_status'],
            'validation_status':output['validation_status'],'evaluation':output['evaluation']}

rows = [probe(n, same) for same in (True,False) for n in (1,19,20,40,200)]
after = hashlib.sha256(source.read_bytes()).hexdigest()
print(json.dumps({'synthetic':True,'personal_accuracy':'not measured','worker_sha256':before,
                  'sources_unchanged':before==after,'probes':rows},indent=2))
if before != after:
    raise SystemExit(2)

if any(not p["learned_preference"] or (p["same_timestamp"] and p["evaluation"]["cases"] != 0) for p in rows):
    raise SystemExit("sample efficiency regression or fabricated holdout")
