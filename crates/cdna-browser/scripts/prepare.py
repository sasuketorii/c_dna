"""Download pinned official Pyodide assets and byte-copy shared learner source.

Defaults to this crate's ignored assets directory; pass deployment staging root
as the optional positional argument. Every published file must fit 25 MiB.
"""
import concurrent.futures
import hashlib
import json
from pathlib import Path
import shutil
import sys
import urllib.request

CRATE = Path(__file__).resolve().parents[1]
ROOT = CRATE.parents[1]
VERSION = '314.0.7'
BASE = f'https://cdn.jsdelivr.net/pyodide/v{VERSION}/full/'
LIMIT = 25 * 1024 * 1024
LOCK = json.loads((CRATE/'pyodide-lock.json').read_text())


def main():
    target = Path(sys.argv[1]) if len(sys.argv) > 1 else CRATE/'assets'
    runtime = target/'pyodide'
    runtime.mkdir(parents=True, exist_ok=True)
    packages = set()
    def include(name):
        name = name.replace("_", "-")
        if name in packages:
            return
        packages.add(name)
        for dep in LOCK['packages'][name]['depends']:
            include(dep)
    for name in ('scikit-learn', 'pydantic'):
        include(name)
    files = [(LOCK['packages'][name]['file_name'], LOCK['packages'][name]['sha256']) for name in sorted(packages)]
    core = ('pyodide.mjs', 'pyodide.asm.mjs', 'pyodide.asm.wasm', 'python_stdlib.zip')
    pins_path = CRATE/'runtime-integrity.json'
    pins = json.loads(pins_path.read_text())
    files.extend((name, pins[name]) for name in core)
    def fetch(item):
        name, expected = item
        path = runtime/name
        if path.exists() and expected and hashlib.sha256(path.read_bytes()).hexdigest() == expected:
            data = path.read_bytes()
        else:
            with urllib.request.urlopen(BASE+name, timeout=90) as response:
                data = response.read(LIMIT+1)
            if len(data) > LIMIT:
                raise ValueError(f'Cloudflare 25 MiB per-file limit exceeded: {name}')
            actual = hashlib.sha256(data).hexdigest()
            if expected and actual != expected:
                raise ValueError(f'integrity mismatch: {name}')
            path.write_bytes(data)
        return name, {'sha256': hashlib.sha256(data).hexdigest(), 'bytes': len(data)}
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        manifest = dict(pool.map(fetch, files))
    shutil.copyfile(CRATE/'pyodide-lock.json', runtime/'pyodide-lock.json')
    source = ROOT/'learner/src/cdna_learner/worker.py'
    shutil.copyfile(source, target/'learner.py')
    manifest['learner.py'] = {'sha256': hashlib.sha256(source.read_bytes()).hexdigest(), 'bytes': source.stat().st_size}
    (target/'manifest.json').write_text(json.dumps({'pyodide': VERSION, 'files': manifest, 'packages': {k: LOCK['packages'][k]['version'] for k in sorted(packages)}}, indent=2)+'\n')
    print(json.dumps({'files': len(manifest), 'bytes': sum(x['bytes'] for x in manifest.values()), 'max_file_bytes': max(x['bytes'] for x in manifest.values()), 'learner_sha256': manifest['learner.py']['sha256']}))


if __name__ == '__main__':
    main()
