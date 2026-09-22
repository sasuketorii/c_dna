#!/usr/bin/env python3
"""Build and measure a stable source snapshot. Run this through revh heavy guard."""
import argparse
import sys
sys.dont_write_bytecode = True
import importlib.util
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('engine_recorder', ROOT / 'benchmarks/engine/run.py')
recorder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(recorder)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out-dir',type=Path,required=True)
    parser.add_argument('--iterations',type=int,default=200)
    args=parser.parse_args()
    if not 1<=args.iterations<=1000: parser.error('iterations must be 1..1000')
    args.out_dir.mkdir(parents=True,exist_ok=True)
    before=recorder.source_hashes()
    command=['cargo','build','--release','--locked','--offline','--manifest-path','benchmarks/engine/Cargo.toml','--target-dir','target','-j','2']
    completed=subprocess.run(command,cwd=ROOT,timeout=600,check=False)
    after=recorder.source_hashes()
    binaries={name:recorder.core.digest(ROOT/'target/release'/name) for name in ('cdna-engine-review','signed') if (ROOT/'target/release'/name).is_file()}
    binding={'build_command':command,'build_exit':completed.returncode,'sources_before':before,'sources_after':after,'sources_unchanged':before==after,'binaries_sha256':binaries}
    (args.out_dir/'build-binding.json').write_text(json.dumps(binding,indent=2)+'\n')
    if completed.returncode or before!=after:
        raise SystemExit('build failed or sources changed during build; rerun after writers freeze')
    for binary,name in [('cdna-engine-review','memory-final.json'),('signed','signed-final.json')]:
        subprocess.run([sys.executable,'benchmarks/engine/run.py','--binary',f'target/release/{binary}','--iterations',str(args.iterations),'--out',str(args.out_dir/name)],cwd=ROOT,timeout=300,check=True)
    if recorder.source_hashes()!=before:
        raise SystemExit('sources changed between build and final measurement')
    signed=json.loads((args.out_dir/'signed-final.json').read_text())
    outcomes={row['probe']:row['accepted'] for row in signed['probes']}
    if outcomes!={'independent_control':True,'one_test_family':False,'one_case_domain':False}:
        raise SystemExit(f'signed trust regression: {outcomes}')
    memory=json.loads((args.out_dir/'memory-final.json').read_text())
    train=next(row['result'] for row in memory['probes'] if row['probe']=='training_at_10000_records')
    if train!={'ok':True,'records':20}:
        raise SystemExit(f'large-corpus training regression: {train}')
    print(json.dumps({'accepted':True,'scope':'stable local synthetic memory/signed paths; no personal accuracy or arbitrary NL claim','out_dir':str(args.out_dir)}))

if __name__=='__main__':main()
