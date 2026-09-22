"""Read the staged macOS bundle, run its demo backend, verify UI and session API."""
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import shutil
import tempfile
import uuid
import sys
import urllib.error
import urllib.parse
import urllib.request

root = Path(__file__).resolve().parents[4]
source = Path(sys.argv[1]) if len(sys.argv) > 1 else root/'target/release/bundle/macos/C-DNA.app'
scratch = tempfile.TemporaryDirectory(prefix='cdna-moved-bundle-')
moved = Path(scratch.name)/'Moved C-DNA.app'
shutil.copytree(source, moved, symlinks=True)
bundle = moved/'Contents'
for path in (bundle/'Resources/learner').rglob('*'):
    assert path.name not in {'direct_url.json', '.env', 'vault.db', 'manifest.json'}, path.name
    assert path.suffix not in {'.pth', '.key', '.pem', '.db', '.sqlite', '.sqlite3'}, path.name
    if path.is_symlink():
        assert path.resolve().is_relative_to(bundle.resolve()), path.name

# Deny reads from the repository and installed development runtimes, not just cwd.
profile = '(version 1)(allow default)' + ''.join(
    '(deny file-read* (subpath '+json.dumps(str(path))+'))'
    for path in [root, Path('/opt/homebrew'), Path.home()/'.local/share/uv', Path.home()/'.local/share/mise'])
environment = {'PATH':'/usr/bin:/bin', 'HOME':scratch.name, 'TMPDIR':scratch.name,
               'OMP_NUM_THREADS':'1', 'OPENBLAS_NUM_THREADS':'1', 'MKL_NUM_THREADS':'1'}
proc = subprocess.Popen(['/usr/bin/sandbox-exec', '-p', profile, str(bundle/'MacOS/cdna'), '--learner-dir', str(bundle/'Resources/learner'), 'playground', '--demo', '--port', '0', '--assets-dir', str(bundle/'Resources/ui')], cwd=scratch.name, env=environment, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, start_new_session=True)
try:
    if not select.select([proc.stderr], [], [], 10)[0]:
        raise TimeoutError('backend startup')
    line = proc.stderr.readline(4096).decode()
    if not line.startswith('C-DNA: '):
        raise RuntimeError('backend failed to announce readiness')
    parsed = urllib.parse.urlsplit(line.removeprefix('C-DNA: ').strip())
    assert parsed.hostname == '127.0.0.1' and parsed.port
    origin = f'http://{parsed.netloc}'
    token = parsed.fragment.removeprefix('session=')
    with urllib.request.urlopen(origin+'/', timeout=3) as response:
        assert response.status == 200 and b'<html' in response.read(65536).lower()
    request = urllib.request.Request(origin+'/api/command', data=b'{"operation":"status"}', headers={'Content-Type':'application/json','Origin':origin,'Authorization':f'Bearer {token}'})
    with urllib.request.urlopen(request, timeout=3) as response:
        assert json.load(response)['ok'] is True
    request = urllib.request.Request(origin+'/api/command', data=b'{"operation":"status"}', headers={'Content-Type':'application/json','Origin':origin})
    try:
        urllib.request.urlopen(request, timeout=3)
        raise AssertionError('missing token accepted')
    except urllib.error.HTTPError as error:
        assert error.code == 403
    def command(value):
        request = urllib.request.Request(origin+'/api/command', data=json.dumps(value).encode(), headers={'Content-Type':'application/json','Origin':origin,'Authorization':f'Bearer {token}'})
        with urllib.request.urlopen(request, timeout=40) as response:
            result = json.load(response)
        assert result['ok'], result
        return result['result']

    workspace = command({'operation':'workspace_create','name':'Isolated synthetic learning smoke'})['id']
    proposal = {'schema_version':'1.0','request_id':str(uuid.uuid4()),'workspace_id':workspace,
        'family_id':str(uuid.uuid4()),'case_kind':'hypothetical','domain':'product_delivery',
        'context':{'as_of':'2026-09-01T00:00:00Z','summary':'Synthetic deadline',
            'facts':[{'key':'deadline_pressure','value':1.0,'unit':'ratio','evidence_status':'explicit'}],'unknown_fields':[]},
        'candidates':[{'id':'fast','text':'Fast','attributes':{'speed':1.0,'cost':0.5}},
                      {'id':'slow','text':'Slow','attributes':{'speed':-1.0,'cost':-0.5}}],
        'response':{'response_type':'choose_one','candidate_id':'fast'},
        'source':{'artifact_id':str(uuid.uuid4()),'source_kind':'human_app','occurred_at':None,'observed_at':'2026-09-01T01:00:00Z'},
        'model_exposure':False,'rationale_explicit':'Synthetic test only','reversal_conditions':[]}
    row = command({'operation':'propose','proposal':proposal})
    command({'operation':'confirm','workspace_id':workspace,'id':row['id'],'expected_revision':row['revision'],'request_id':str(uuid.uuid4())})
    model = command({'operation':'train','workspace_id':workspace})
    query = {'schema_version':'1.0','request_id':str(uuid.uuid4()),'workspace_id':workspace,'mode':'imitate',
             'domain':proposal['domain'],'context':proposal['context'],'candidates':proposal['candidates']}
    preview = command({'operation':'preview_model','workspace_id':workspace,'id':model['id'],'request':query})
    assert preview['ranking'][0]['candidate_id'] == 'fast'
    assert preview['provisional'] is True and preview['probability'] is None
    subprocess.run(['/usr/bin/sandbox-exec','-p',profile,str(bundle/'Resources/learner/python/bin/python3'),'-I','-B','-c','import sklearn, scipy, pydantic, lightgbm, cdna_learner; print("PASS: isolated imports")'],cwd=scratch.name,env=environment,check=True,timeout=30)
    print('PASS: moved bundle UI/auth, propose/confirm/train/preview; repository and host runtimes denied')
finally:
    os.killpg(proc.pid, signal.SIGINT)
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        os.killpg(proc.pid, signal.SIGKILL)
        proc.wait()
    proc.stderr.close()

    scratch.cleanup()
