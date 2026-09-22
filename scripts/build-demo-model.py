#!/usr/bin/env python3
"""Build a synthetic-only public demo; never enters confirmed-observation training.

Run: cd learner && uv run python ../scripts/build-demo-model.py [--check]
The shipped model is an illustration of an invented CEO, not evidence about a person.
"""
import argparse
import hashlib
import importlib.metadata
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'learner/src'))
sys.path.insert(0, str(ROOT / 'learner/benchmarks'))
from cdna_learner.worker import Context, Candidate, Request, _fit_snapshot, eligible, features, run
from preference_bench import generate, ORACLE

OUTPUT = ROOT / 'apps/desktop/src/demo-data'


def build():
    # Existing benchmark generates deterministic, explicitly UNCONFIRMED hypothetical
    # observations from an invented utility. The production train() excludes all of them.
    records = generate(count=2000, seed=271828)
    assert all(r.verification_state == 'unconfirmed' and not eligible(r) for r in records)
    result = _fit_snapshot(Request(operation='train', feature_version='1.0', records=records), records)
    # Calibration on invented labels is not a probability about a human CEO.
    model = result['model']
    model.update(provisional=True, calibration=None, calibration_scope=[])
    provenance = {
        'kind': 'synthetic_demo_only', 'humanLabel': False, 'userData': False,
        'personalAccuracyEvidence': False, 'seed': 271828, 'trainingCases': len(records),
        'verificationState': 'unconfirmed', 'caseKind': 'hypothetical',
        'trainingPath': 'cdna_learner.worker._fit_snapshot (internal synthetic-only build)',
        'generator': 'learner/benchmarks/preference_bench.py:generate',
        'teacherWeights': ORACLE.tolist(),
        'learnerSha256': hashlib.sha256((ROOT / 'learner/src/cdna_learner/worker.py').read_bytes()).hexdigest(),
        'generatorSha256': hashlib.sha256((ROOT / 'learner/benchmarks/preference_bench.py').read_bytes()).hexdigest(),
        'dependencies': {name: importlib.metadata.version(name) for name in ('numpy', 'scipy', 'scikit-learn', 'pydantic')},
        'splits': result['splits'],
        'notice': '架空のCEO像を合成データで事前学習したデモです。実在の経営者の判断・正解率を示しません。',
    }
    scenarios = json.loads((OUTPUT / 'scenarios.json').read_text())
    parity = []
    for scenario in scenarios:
        context = Context.model_validate(scenario['context'])
        candidates = [Candidate.model_validate(c) for c in scenario['candidates']]
        ranked = run(Request(operation='rank', feature_version='1.0', model=model, context=context, candidates=candidates))
        parity.append({'scenarioId': scenario['id'], 'ranking': ranked['ranking'], 'abstained': ranked['abstained'],
                       'features': {c.id: features(context, c).tolist() for c in candidates}})
    return {'model.json': {'personaLabel': '架空のCEO・速度と再利用を重視', 'model': model, 'provenance': provenance},
            'parity.json': {'source': 'Python learner reference; synthetic fixtures only', 'cases': parity}}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    for name, value in build().items():
        content = json.dumps(value, ensure_ascii=False, indent=2, allow_nan=False) + '\n'
        path = OUTPUT / name
        if args.check:
            if path.read_text() != content:
                raise SystemExit(f'stale generated fixture: {path.relative_to(ROOT)}')
        else:
            path.write_text(content)
    print('synthetic demo: 17 learned weights and 12 scenario reference outputs verified' if args.check else 'synthetic demo artifacts built')


if __name__ == '__main__':
    main()
