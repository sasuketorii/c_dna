from __future__ import annotations
import json
import uuid
from pathlib import Path
from datetime import datetime,timedelta,timezone
import numpy as np
import pytest
from cdna_learner.features import transform,feature_names

ROOT=Path(__file__).resolve().parents[2]

def make_snapshot(count=120, is_demo=True):
    templates=json.loads((ROOT/'contracts/question-templates.json').read_text())
    rng=np.random.default_rng(81)
    ws=str(uuid.uuid4())
    weights=np.zeros(len(feature_names()))
    for k,v in {'candidate:quality':.5,'candidate:cost_efficiency':.4,'context:deadline_days*knowledge_gain':2.5,'context:customer_impact*customer_protection':2,'context:resource_pressure*cost_efficiency':1.6,'candidate:speed':1.5,'context:deadline_days*speed':-2}.items():
        weights[feature_names().index(k)]=v
    records=[]
    for n in range(count):
        t=templates[n%len(templates)]
        ts=(datetime(2024,1,1,tzinfo=timezone.utc)+timedelta(days=n)).isoformat()
        ctx={'as_of':ts,'summary':'架空の検証用シナリオ。本人の実際の判断ではありません。','facts':[
            {'key':'deadline_days','value':int(rng.integers(1,61)),'evidence_status':'explicit','unit':'day'},
            {'key':'customer_impact','value':['low','medium','high'][n%3],'evidence_status':'explicit'},
            {'key':'resource_pressure','value':float(rng.random()),'evidence_status':'explicit'},
            {'key':'reversibility','value':['low','medium','high'][(n+1)%3],'evidence_status':'explicit'},
            {'key':'horizon_months','value':int(rng.integers(1,37)),'evidence_status':'explicit','unit':'month'},
            {'key':'evidence_confidence','value':float(rng.random()),'evidence_status':'explicit'},
            {'key':'available_people','value':int(rng.integers(1,10)),'evidence_status':'explicit','unit':'person'}], 'unknown_fields':[]}
        candidates=t['candidates']
        scores=transform(t['domain'],ctx,candidates)@weights
        selected=candidates[int(np.argmax(scores))]['id']
        records.append({'case_id':str(uuid.uuid4()),'family_id':str(uuid.uuid4()),'revision':1,'as_of':ts,'domain':t['domain'],'context':ctx,'candidates':candidates,'response':{'kind':'choose_one','selected_ids':[selected],'note':'これは合成ラベルです。','reversal_condition':''},'verified':True,'origin':'synthetic_seed' if is_demo else 'human_ui','model_exposure':False,'case_kind':'hypothetical','weight':1.0})
    return {'schema_version':'1.0','snapshot_id':str(uuid.uuid4()),'workspace_id':ws,'deletion_epoch':0,'feature_version':'1.0','is_demo':is_demo,'records':records,'seed':17,'compare_lightgbm':False}

@pytest.fixture
def snapshot(): return make_snapshot()
