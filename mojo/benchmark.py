"""Reproducible numerical equivalence and CPU IPC comparison; no adoption claim."""
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import resource
import select
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
os.environ['MODULAR_TELEMETRY_ENABLED'] = 'false'
os.environ['OPENBLAS_NUM_THREADS'] = '1'
import numpy as np


def exchange(proc, weights, rows):
    payload = json.dumps({'weights': weights.tolist(), 'features': rows.tolist()}, allow_nan=False).encode() + b'\n'
    proc.stdin.write(payload)
    proc.stdin.flush()
    if not select.select([proc.stdout], [], [], 10)[0]:
        raise TimeoutError('worker timed out')
    line = proc.stdout.readline(262144)
    if not line.endswith(b'\n'):
        raise ValueError('bounded response required')
    return np.asarray(json.loads(line), dtype=np.float64)


def start(path):
    return subprocess.Popen([str(path)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=os.environ)


def stop(p):
    p.stdin.close()
    try:
        p.wait(timeout=2)
    except subprocess.TimeoutExpired:
        p.kill()
        p.wait()
    p.stdout.close()
    p.stderr.close()


def stats(values):
    return dict(zip(('p50_ms', 'p95_ms', 'p99_ms'), np.percentile(values, [50, 95, 99]).tolist()))


def main():
    rng = np.random.default_rng(20260922)
    weights = rng.uniform(-1, 1, 17)
    edge = np.vstack([np.zeros(17), np.ones(17), -np.ones(17), np.full(17, 1e-300), rng.uniform(-1, 1, (256, 17))])
    engines = {'rust_ipc': ROOT/'target/release/cdna-inference', 'mojo_ipc': ROOT/'mojo/scorer'}
    result = {'dtype': 'float64', 'abs_tolerance': 1e-9, 'rel_tolerance': 1e-8, 'dimensions': 17, 'cpu_only': True, 'runs': 3, 'warmup': 20, 'samples_per_run': 100, 'platform': platform.platform(), 'machine': platform.machine(), 'cpu_count': os.cpu_count(), 'thermal_state': 'unmeasured', 'power_mode': 'unmeasured', 'thread_limit': 1, 'measurements': {}, 'equivalence': {}}
    result['measured_at'] = datetime.now(timezone.utc).isoformat()
    result['compiler'] = subprocess.check_output([str(ROOT/'mojo/.venv/bin/mojo'), '--version'], text=True, timeout=10).strip()
    result['source_sha256'] = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in (ROOT/'mojo/scorer.mojo', ROOT/'crates/cdna-inference/src/lib.rs')}
    if platform.system() == 'Darwin':
        result['hardware'] = {k: subprocess.check_output(['sysctl', '-n', k], text=True, timeout=5).strip() for k in ('hw.model', 'hw.memsize', 'machdep.cpu.brand_string')}
    for name, path in engines.items():
        result.setdefault('binary_sha256', {})[name] = hashlib.sha256(path.read_bytes()).hexdigest()
        p = start(path)
        try:
            actual = exchange(p, weights, edge)
            expected = edge @ weights
            np.testing.assert_allclose(actual, expected, atol=1e-9, rtol=1e-8)
            assert np.array_equal(np.argsort(actual, kind='stable'), np.argsort(expected, kind='stable'))
            result['equivalence'][name] = {'rows': len(edge), 'max_abs_error': float(np.max(np.abs(actual-expected))), 'rank_matches': True}
        finally:
            stop(p)
        measurements = []
        for n in (1, 4, 16, 1024):
            rows = rng.uniform(-1, 1, (n, 17))
            for run in range(3):
                before = time.perf_counter_ns()
                p = start(path)
                try:
                    exchange(p, weights, rows)
                    cold = (time.perf_counter_ns()-before)/1e6
                    for _ in range(20):
                        exchange(p, weights, rows)
                    times = []
                    for _ in range(100):
                        before = time.perf_counter_ns()
                        exchange(p, weights, rows)
                        times.append((time.perf_counter_ns()-before)/1e6)
                    measurements.append({'candidates': n, 'run': run, 'cold_ms': cold, **stats(times)})
                finally:
                    stop(p)
        result['measurements'][name] = measurements
    measurements = []
    for n in (1, 4, 16, 1024):
        rows = rng.uniform(-1, 1, (n, 17))
        for run in range(3):
            for _ in range(20):
                _ = rows @ weights
            times = []
            for _ in range(100):
                before = time.perf_counter_ns()
                _ = rows @ weights
                times.append((time.perf_counter_ns()-before)/1e6)
            measurements.append({'candidates': n, 'run': run, **stats(times)})
    result['measurements']['numpy_native'] = measurements
    result['measurements']['rust_native'] = json.loads(subprocess.check_output([str(ROOT/'target/release/examples/native_bench')], timeout=10))
    result['parent_peak_rss_platform_units'] = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    result['children_peak_rss_platform_units'] = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    result['limitations'] = ['17 dimensions, not 2048-dimensional or 10000-case acceptance fixture', 'RSS is aggregate process high water, not per-engine idle memory', 'Mojo JSON transport uses CPython interoperability', 'Thermal and power conditions not observed', 'No network observation performed', 'Complete product distribution gate required before adoption']
    result['decision'] = 'Rust remains default; Mojo not promoted'
    Path(os.environ.get('CDNA_BENCH_OUTPUT', str(ROOT/'mojo/benchmark-results.json'))).write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({'equivalence': result['equivalence'], 'decision': result['decision']}))


if __name__ == '__main__':
    main()
