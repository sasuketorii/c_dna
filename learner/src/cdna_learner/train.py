"""Small explicit models with immutable snapshots; no pickle and no network."""
from __future__ import annotations
import hashlib
import importlib.metadata
import json
import time
import uuid
import warnings
from collections import Counter
from typing import Any
import numpy as np
from sklearn.linear_model import SGDClassifier, LogisticRegression
from .contracts import Snapshot, SnapshotRecord, ContractError, verify_snapshot
from .features import transform, score, feature_names, manifest_hash
from .pairwise import make_pairs, comparisons
from .evaluation import split_records, evaluate


def _ranker(records: list[SnapshotRecord]):
    from lightgbm import LGBMRanker
    rows, labels, groups, weights = [],[],[],[]
    for r in records:
        if r.response['kind'] not in {'choose_one','choose_set','pairwise'} or not comparisons(r):
            continue
        x = transform(r.domain,r.context,r.candidates)
        rows.extend(x)
        selected = set(r.response['selected_ids'])
        labels.extend(int(c['id'] in selected) for c in r.candidates)
        groups.append(len(r.candidates)); weights.extend([r.weight/len(r.candidates)]*len(r.candidates))
    if len(groups)<8:
        return None
    model=LGBMRanker(objective='lambdarank',n_estimators=64,num_leaves=7,max_depth=4,
        min_child_samples=5,learning_rate=.05,n_jobs=2,verbosity=-1,random_state=17,
        deterministic=True,force_col_wise=True)
    model.fit(np.asarray(rows),np.asarray(labels),group=groups,sample_weight=np.asarray(weights))
    return model


def _calibrate(records: list[SnapshotRecord], weights: list[float]) -> dict[str,Any]:
    report = evaluate(records,lambda r:score(weights,r.domain,r.context,r.candidates))
    groups: dict[int,list[dict[str,Any]]] = {}
    for item in report['details']:
        groups.setdefault(item['candidate_count'],[]).append(item)
    models = []
    for n, items in groups.items():
        y = [int(i['correct']) for i in items]
        if len(items)<60 or len(set(y))<2 or min(sum(y),len(y)-sum(y))<10:
            continue
        calibrator = LogisticRegression(C=.1,random_state=17,max_iter=500)
        calibrator.fit(np.asarray([[i['margin']] for i in items]),y)
        models.append({'candidate_count':n,'sample_count':len(items),
                       'coefficient':float(calibrator.coef_[0,0]),'intercept':float(calibrator.intercept_[0])})
    return {'status':'fitted_not_yet_validated' if models else 'insufficient_data','models':models}


def train(data: dict[str,Any], *, cancelled=lambda:False) -> dict[str,Any]:
    started=time.perf_counter()
    snapshot=verify_snapshot(data)
    split=split_records(snapshot.records)
    pairs=make_pairs(split.train)
    if len(pairs.y)<2:
        raise ContractError('INSUFFICIENT_TRAINING_DATA')
    model=SGDClassifier(loss='log_loss',penalty='l2',alpha=.005,fit_intercept=False,
                        learning_rate='constant',eta0=.05,shuffle=False,random_state=snapshot.seed)
    rng=np.random.default_rng(snapshot.seed)
    # Epoch boundaries provide cancellation points. The broker can kill the worker
    # as a final deadline measure without touching the active vault/model.
    for epoch in range(90):
        if cancelled():
            raise ContractError('CANCELLED')
        order=rng.permutation(len(pairs.y))
        model.partial_fit(pairs.x[order],pairs.y[order],classes=np.array([0,1]),sample_weight=pairs.weights[order])
    weights=model.coef_[0].tolist()
    linear=lambda r:score(weights,r.domain,r.context,r.candidates)
    validation=evaluate(split.validation,linear)
    calibration=_calibrate(split.calibration,weights)
    test=evaluate(split.test,linear)
    tree_report: dict[str,Any]={'state':'not_requested'}
    tree_artifact=None
    if snapshot.compare_lightgbm:
        if cancelled():
            raise ContractError('CANCELLED')
        try:
            with warnings.catch_warnings():
                warnings.simplefilter('ignore',UserWarning)
                tree=_ranker(split.train)
                if tree is None:
                    tree_report={'state':'insufficient_data'}
                else:
                    tree_validation=evaluate(split.validation,lambda r:tree.predict(transform(r.domain,r.context,r.candidates)))
                    tree_test=evaluate(split.test,lambda r:tree.predict(transform(r.domain,r.context,r.candidates)))
                    # Comparison never silently replaces the default linear engine.
                    tree_report={'state':'evaluated','validation':tree_validation,'test':tree_test,'automatic_activation':False}
                    tree_artifact=tree.booster_.model_to_string()
        except ImportError:
            tree_report={'state':'dependency_unavailable'}
    if cancelled():
        raise ContractError('CANCELLED')
    domains=dict(Counter(r.domain for r in split.train if comparisons(r)))
    versions={p:importlib.metadata.version(p) for p in ('numpy','scipy','scikit-learn','pydantic')}
    records_hash=hashlib.sha256(json.dumps(data['records'],sort_keys=True,ensure_ascii=False,separators=(',',':'),allow_nan=False).encode()).hexdigest()
    return {'schema_version':'1.0','id':str(uuid.uuid4()),'workspace_id':snapshot.workspace_id,
        'snapshot_id':snapshot.snapshot_id,'deletion_epoch':snapshot.deletion_epoch,
        'feature_version':'1.0','feature_manifest_sha256':manifest_hash(), 'kind':'linear_pairwise',
        'dtype':'float64','weights':weights,'intercept':0.0,'feature_names':feature_names(),
        'state':'evaluated','is_demo':snapshot.is_demo,'provisional':True,'calibration':calibration,
        'domain_support':domains,'evaluation':{'validation':validation,'test':test,'lightgbm':tree_report,
            'dataset_kind':'synthetic_regression' if snapshot.is_demo else 'personal_unexposed',
            'person_agreement_verified':False,
            'note':'順位の独立比較。校正・方針・本番の保留方針を含む本人再現性能は未検証です。'},
        'lightgbm_text':tree_artifact,'training':{'record_count':len(split.train),'pair_count':len(pairs.y),
            'pair_weight_sum':float(pairs.weights.sum()),'seed':snapshot.seed,'epochs':90,
            'record_revisions':[{'case_id':r.case_id,'revision':r.revision} for r in split.train],
            'records_sha256':records_hash,'dependency_versions':versions,'split_policy':split.policy,
            'split_counts':{k:len(getattr(split,k)) for k in ('train','validation','calibration','test','quarantined')},
            'elapsed_ms':(time.perf_counter()-started)*1000}}


def incremental(data: dict[str,Any], previous: dict[str,Any]) -> dict[str,Any]:
    """An internal candidate update only; deletion must always take the full path."""
    snapshot=verify_snapshot(data)
    if (previous.get('feature_manifest_sha256')!=manifest_hash() or
        previous.get('workspace_id')!=snapshot.workspace_id or
        previous.get('deletion_epoch')!=snapshot.deletion_epoch):
        raise ContractError('MODEL_INVALIDATED')
    pairs=make_pairs(snapshot.records)
    if not len(pairs.y):
        raise ContractError('INSUFFICIENT_TRAINING_DATA')
    w=np.asarray(previous['weights'],dtype=np.float64)
    if w.shape!=(len(feature_names()),) or not np.isfinite(w).all():
        raise ContractError('MODEL_INVALID')
    model=SGDClassifier(loss='log_loss',alpha=.005,fit_intercept=False,learning_rate='constant',eta0=.05,shuffle=False)
    # Initialise sklearn's internal state without contributing a real observation.
    model.partial_fit(np.zeros((2,len(w))),np.array([0,1]),classes=np.array([0,1]),sample_weight=np.zeros(2))
    model.coef_=w.reshape(1,-1).copy()
    for _ in range(5):
        model.partial_fit(pairs.x,pairs.y,sample_weight=pairs.weights)
    result=dict(previous)
    result.update(id=str(uuid.uuid4()),snapshot_id=snapshot.snapshot_id,weights=model.coef_[0].tolist(),
        provisional=True,state='candidate',calibration={'status':'insufficient_data','models':[]},
        evaluation={'dataset_kind':'synthetic_regression' if snapshot.is_demo else 'personal_unexposed','person_agreement_verified':False},
        training={'record_count':len(snapshot.records),'base_model_id':previous['id'],'update_kind':'incremental'})
    return result
