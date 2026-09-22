"""Stage a relocatable macOS learner; never copy a development venv or user vault."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[4]
DEST = Path(__file__).resolve().parents[1] / 'resources/learner'
PYTHON = '3.14.7'
OPENMP = '23.1.1'


def run(*args, **kwargs):
    return subprocess.check_output([str(a) for a in args], text=True, **kwargs).strip()


def stage():
    if os.uname().sysname != 'Darwin' or os.uname().machine != 'arm64':
        raise RuntimeError('This distribution has only been validated for macOS arm64')
    with tempfile.TemporaryDirectory(prefix='cdna-learner-build-') as scratch:
        scratch = Path(scratch).resolve()
        # Reuse an exact managed version if available; otherwise install only in scratch.
        found = subprocess.run(['uv', 'python', 'find', '--managed-python',
                                '--no-python-downloads', PYTHON], capture_output=True, text=True)
        if found.returncode == 0:
            source = Path(found.stdout.strip()).resolve().parent.parent
        else:
            run('uv', 'python', 'install', '--no-bin', '--no-registry',
                '--install-dir', scratch/'managed', PYTHON)
            source = next((scratch/'managed').glob('cpython-*'))
        output = scratch/'learner'
        python = output/'python'
        shutil.copytree(source, python, symlinks=True,
                        ignore=shutil.ignore_patterns('__pycache__', '*.pyc', '*.a'))
        executable = python/'bin/python3'
        assert run(executable, '-I', '-B', '-c', 'import platform; print(platform.python_version())') == PYTHON
        site = python/'lib/python3.14/site-packages'
        shutil.rmtree(site)
        site.mkdir()
        requirements = scratch/'requirements.txt'
        run('uv', 'export', '--project', ROOT/'learner', '--frozen', '--no-dev',
            '--no-emit-project', '--output-file', requirements)
        run('uv', 'pip', 'install', '--python', executable, '--target', site,
            '--link-mode', 'copy', '--only-binary', ':all:', '--require-hashes', '-r', requirements)
        # Copy only first-party source, never editable metadata, tests, fixtures or local data.
        shutil.copytree(ROOT/'learner/src/cdna_learner', site/'cdna_learner',
                        ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
        shutil.copy2(ROOT/'LICENSE', output/'LICENSE-CDNA')
        shutil.copy2(ROOT/'learner/uv.lock', output/'uv.lock')
        # LightGBM's upstream macOS wheel expects host OpenMP. Vendor the pinned,
        # licensed runtime and replace both host search paths with loader-relative lookup.
        omp = Path(run('brew', '--prefix', 'libomp', env={**os.environ, 'HOMEBREW_NO_AUTO_UPDATE': '1'}))
        receipt = json.loads((omp/'INSTALL_RECEIPT.json').read_text())
        assert receipt['source']['versions']['stable'] == OPENMP, 'libomp version changed'
        lib = site/'lightgbm/lib'
        shutil.copy2(omp/'lib/libomp.dylib', lib/'libomp.dylib')
        shutil.copy2(omp/'LICENSE.TXT', lib/'LICENSE-OPENMP.txt')
        run('install_name_tool', '-change', '@rpath/libomp.dylib', '@loader_path/libomp.dylib',
            '-delete_rpath', '/opt/homebrew/opt/libomp/lib',
            '-delete_rpath', '/opt/local/lib/libomp', lib/'lib_lightgbm.dylib')
        run('install_name_tool', '-id', '@loader_path/libomp.dylib', lib/'libomp.dylib')
        for name in ('lib_lightgbm.dylib', 'libomp.dylib'):
            run('codesign', '--force', '--sign', '-', lib/name)
        # Console scripts have build-prefix shebangs; the fixed -m entry needs none.
        for path in list((python/'bin').iterdir()):
            if not path.name.startswith('python'):
                path.unlink()
        shutil.rmtree(site/'bin', ignore_errors=True)
        for path in python.rglob('*'):
            if path.is_symlink():
                target = path.resolve()
                if not target.is_relative_to(python):
                    raise RuntimeError(f'escaping symlink: {path.relative_to(python)}')
                path.unlink()
                path.symlink_to(os.path.relpath(target, path.parent))
        run(executable, '-I', '-B', '-c', 'import sklearn, scipy, pydantic, lightgbm, cdna_learner')
        inventory = run(executable, '-I', '-B', '-c',
                        'import importlib.metadata as m,json; print(json.dumps({d.metadata["Name"]:d.version for d in m.distributions()},sort_keys=True))')
        (output/'distribution.json').write_text(json.dumps({
            'python': PYTHON, 'provider': 'uv/python-build-standalone',
            'python_build': (source/'BUILD').read_text().strip(),
            'openmp': OPENMP, 'packages': json.loads(inventory),
            'lock_sha256': hashlib.sha256((ROOT/'learner/uv.lock').read_bytes()).hexdigest(),
            'source_sha256': {str(p.relative_to(site)): hashlib.sha256(p.read_bytes()).hexdigest()
                              for p in sorted((site/'cdna_learner').rglob('*.py'))},
        }, indent=2)+'\n')
        # Generated destination only. Never operate on a vault or user-controlled path.
        if DEST.exists():
            shutil.rmtree(DEST)
        shutil.copytree(output, DEST, symlinks=True)
    print('Staged pinned Python, locked wheels, source package and licenses')


if __name__ == '__main__':
    stage()
