#!/usr/bin/env python3
"""Run an already-built native benchmark and bind measurements to actual artifacts."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import resource
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def capture(*args: str) -> str | None:
    try:
        result = subprocess.run(args, cwd=ROOT, capture_output=True, text=True, timeout=10, check=False)
        return result.stdout.strip() if result.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired):
        return None


def source_hashes() -> dict[str, str]:
    paths = {ROOT / 'Cargo.toml', ROOT / 'Cargo.lock'}
    for relative in ('benchmarks/core', 'crates/cdna-app', 'crates/cdna-domain', 'crates/cdna-store', 'crates/cdna-inference'):
        folder = ROOT / relative
        paths.update(p for p in folder.rglob('*') if p.is_file() and p.suffix in {'.rs', '.toml', '.py'})
    return {str(path.relative_to(ROOT)): digest(path) for path in sorted(paths) if path.is_file()}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/cdna-core-bench')
    parser.add_argument('--iterations', type=int, default=200)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    if not 1 <= args.iterations <= 10000:
        parser.error('--iterations must be 1..10000')
    binary = args.binary.resolve()
    before_hashes = source_hashes()
    binary_hash = digest(binary)
    # No subprocess has run in this Python process before this measurement.
    # Thus RUSAGE_CHILDREN peak RSS here belongs to this benchmark tree only.
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    started = time.monotonic()
    completed = subprocess.run([str(binary), '--iterations', str(args.iterations)], cwd=ROOT,
                               capture_output=True, text=True, timeout=300, check=False)
    wall = time.monotonic() - started
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    if completed.returncode:
        raise SystemExit(f'benchmark failed with exit {completed.returncode}: {completed.stderr[:2000]}')
    report = json.loads(completed.stdout)
    after_hashes = source_hashes()
    binary_after = digest(binary)
    report['artifact_binding'] = {
        'binary_sha256': binary_hash,
        'binary_unchanged': binary_hash == binary_after,
        'sources_sha256': before_hashes,
        'sources_unchanged': before_hashes == after_hashes,
        'git_commit_after_run': capture('git', 'rev-parse', 'HEAD'),
        'git_dirty_after_run': capture('git', 'status', '--porcelain'),
        'note': 'Hashes bind existing files; build provenance is not inferred from matching timestamps. Git metadata is read after the run.',
    }
    cpu = (after.ru_utime - before.ru_utime) + (after.ru_stime - before.ru_stime)
    rss_scale = 1 if sys.platform == 'darwin' else 1024 if sys.platform.startswith('linux') else None
    report['process_resources'] = {
        'scope': 'entire benchmark process tree, including SQLCipher setup, database writes and cleanup',
        'wall_seconds': wall,
        'user_cpu_seconds': after.ru_utime - before.ru_utime,
        'system_cpu_seconds': after.ru_stime - before.ru_stime,
        'cpu_seconds_per_wall_second': cpu / wall,
        'peak_rss_bytes': after.ru_maxrss * rss_scale if rss_scale else None,
        'peak_rss_native_value': after.ru_maxrss,
        'measurement': 'Python resource.getrusage(RUSAGE_CHILDREN); not instantaneous RSS, allocation count, or per-scenario memory',
    }
    report['environment'] = {
        'platform': platform.platform(), 'architecture': platform.machine(),
        'logical_cpu_count': os.cpu_count(),
        'cpu_model': capture('/usr/sbin/sysctl', '-n', 'machdep.cpu.brand_string') if sys.platform == 'darwin' else platform.processor(),
        'physical_memory_bytes': capture('/usr/sbin/sysctl', '-n', 'hw.memsize') if sys.platform == 'darwin' else None,
        'rustc_available': capture('rustc', '--version'),
        'cargo_available': capture('cargo', '--version'),
        'python': platform.python_version(),
        'compiler_note': 'Available toolchain version is recorded; this is not proof it built the supplied binary.',
    }
    report['measurement_validity'] = 'source_or_binary_changed' if before_hashes != after_hashes or binary_hash != binary_after else 'stable_artifact_snapshot'
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')
    print(json.dumps({'output': str(args.out), 'scenarios': len(report['results']),
                      'validity': report['measurement_validity'], 'wall_seconds': wall,
                      'peak_rss_bytes': report['process_resources']['peak_rss_bytes']}))
    if report['measurement_validity'] != 'stable_artifact_snapshot':
        raise SystemExit(2)


if __name__ == '__main__':
    main()
