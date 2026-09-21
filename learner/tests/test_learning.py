import copy,json,struct,subprocess,sys,uuid
import numpy as np
import pytest
from cdna_learner.contracts import *
from cdna_learner.features import *
from cdna_learner.pairwise import *
from cdna_learner.train import train,incremental
from cdna_learner.evaluation import *
from conftest import make_snapshot


def test_pair_weights_symmetry_and_unselected_order(snapshot):
    record=verify_snapshot(snapshot).records[0]
    batch=make_pairs([record])
    assert len(batch.y)==6
    assert batch.weights.sum()==pytest.approx(1.)
    for i in range(0,len(batch.y),2):
        np.testing.assert_array_equal(batch.x[i],-batch.x[i+1])
        assert tuple(batch.y[i:i+2])==(1,0)
    assert len(set(batch.case_ids))==1

@pytest.mark.parametrize('kind,value',[('skip',None),('none_fit',None),('need_information',None),('defer',None),('pairwise','tie'),('pairwise','insufficient')])
def test_non_labels_never_create_negatives(snapshot,kind,value):
    r=copy.deepcopy(snapshot['records'][0]);r['response']={'kind':kind,'selected_ids':[],'value':value}
    if kind=='pairwise':r['candidates']=r['candidates'][:2]
    assert len(make_pairs([SnapshotRecord.model_validate(r)]).y)==0


def test_context_interaction_changes_pair_differences(snapshot):
    r=snapshot['records'][0]
    a=transform(r['domain'],r['context'],r['candidates'])
    other=copy.deepcopy(r['context']);other['facts'][0]['value']=0
    b=transform(r['domain'],other,r['candidates'])
    assert not np.allclose(a[0]-a[1],b[0]-b[1])
    assert not any('note' in n or 'case_id' in n or 'selected' in n for n in feature_names())


def test_option_order_does_not_change_scores(snapshot):
    r=snapshot['records'][0];w=np.arange(len(feature_names()))/len(feature_names())
    a=score(w,r['domain'],r['context'],r['candidates'])
    b=score(w,r['domain'],r['context'],r['candidates'][::-1])
    np.testing.assert_allclose(a,b[::-1],rtol=1e-12,atol=1e-12)


def test_unknown_is_not_zero_and_inferred_is_not_confirmed(snapshot):
    r=snapshot['records'][0];cx=copy.deepcopy(r['context']);cx['facts'][0]['value']=0
    zero=transform(r['domain'],cx,r['candidates'])
    cx['facts'][0].update(value=None,evidence_status='unknown')
    missing=transform(r['domain'],cx,r['candidates'])
    assert not np.array_equal(zero,missing)
    cx['facts'][0].update(value=100,evidence_status='inferred')
    np.testing.assert_array_equal(missing,transform(r['domain'],cx,r['candidates']))


def test_temporal_family_partition_and_exposure(snapshot):
    records=verify_snapshot(snapshot).records
    records[-1].family_id=records[0].family_id
    records[-2].model_exposure=True
    split=split_records(records)
    assert records[0] in split.quarantined and records[-1] in split.quarantined
    assert records[-2] in split.quarantined
    parts=[split.train,split.validation,split.calibration,split.test]
    for a,b in zip(parts,parts[1:]):
        if a and b: assert max(x.as_of for x in a)<=min(x.as_of for x in b)
        assert not {x.family_id for x in a}&{x.family_id for x in b}


def test_real_training_is_numeric_unverified_and_calibration_is_separate(snapshot):
    model=train(snapshot)
    assert len(model['weights'])==len(feature_names())
    assert np.linalg.norm(model['weights'])>0
    assert model['calibration']['status']=='insufficient_data'
    assert not model['evaluation']['person_agreement_verified']
    assert model['evaluation']['dataset_kind']=='synthetic_regression'
    metrics=model['evaluation']['test']['overall']
    assert metrics['total']>0
    assert metrics['coverage'] is not None
    assert model['training']['record_count']<len(snapshot['records'])
    again=train(snapshot)
    np.testing.assert_array_equal(model['weights'],again['weights'])


def test_synthetic_never_becomes_human_accuracy(snapshot):
    m=train(snapshot)
    assert not verified_claim_allowed(m['evaluation']['test'],True)
    assert wilson_lower(90,100)<.85
    assert wilson_lower(0,0) is None


def test_privileged_or_ineligible_data_fails(snapshot):
    for field in ('human_confirmed','owner_id','training_eligible'):
        s=copy.deepcopy(snapshot);s['records'][0][field]=True
        with pytest.raises(ContractError):verify_snapshot(s)
    s=copy.deepcopy(snapshot);s['is_demo']=False
    with pytest.raises(ContractError,match='入力'):verify_snapshot(s)
    s=copy.deepcopy(snapshot);s['records'][0]['verified']=False
    with pytest.raises(ContractError):verify_snapshot(s)


def test_duplicate_cases_and_candidates_fail(snapshot):
    s=copy.deepcopy(snapshot);s['records'].append(s['records'][0])
    with pytest.raises(ContractError):verify_snapshot(s)
    s=copy.deepcopy(snapshot);s['records'][0]['candidates'][1]['id']=s['records'][0]['candidates'][0]['id']
    with pytest.raises(ContractError):verify_snapshot(s)

@pytest.mark.parametrize('raw',['{"x":NaN}','{"x":Infinity}','{"x":1,"x":2}','{"x":1e999}','[1,'])
def test_strict_json(raw):
    with pytest.raises(ContractError):loads_strict(raw)


def test_lightgbm_preserves_groups(snapshot):
    snapshot['compare_lightgbm']=True
    model=train(snapshot)
    assert model['evaluation']['lightgbm']['state']=='evaluated'
    assert model['lightgbm_text'].startswith('tree')
    assert model['kind']=='linear_pairwise'


def test_cancelled_learning(snapshot):
    with pytest.raises(ContractError) as error:train(snapshot,cancelled=lambda:True)
    assert error.value.code=='CANCELLED'


def test_incremental_drops_previous_calibration_and_rejects_epoch(snapshot):
    model=train(snapshot);model['calibration']={'status':'fitted','models':[1]}
    update=copy.deepcopy(snapshot);update['records']=update['records'][:2];update['snapshot_id']=str(uuid.uuid4())
    out=incremental(update,model)
    assert out['id']!=model['id'] and out['calibration']['models']==[]
    update['deletion_epoch']+=1
    with pytest.raises(ContractError):incremental(update,model)


def test_wire_worker_has_one_bounded_frame_no_logs(snapshot):
    snapshot['records']=snapshot['records'][:12]
    raw=json.dumps({'operation':'train','snapshot':snapshot}).encode()
    process=subprocess.run([sys.executable,'-m','cdna_learner'],input=struct.pack('>I',len(raw))+raw,capture_output=True,timeout=30)
    assert process.returncode==0
    n=struct.unpack('>I',process.stdout[:4])[0]
    assert len(process.stdout)==n+4
    body=json.loads(process.stdout[4:]);assert body['ok']
    assert b'\xe3\x81\x93\xe3\x82\x8c\xe3\x81\xaf' not in process.stderr
