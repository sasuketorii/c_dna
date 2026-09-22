#!/usr/bin/env python3
"""Reuse the core process/resource recorder; only change source coverage and limits."""
import sys
sys.dont_write_bytecode = True
import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('core_recorder', ROOT / 'benchmarks/core/run.py')
core = importlib.util.module_from_spec(spec)
spec.loader.exec_module(core)
original_hashes = core.source_hashes

def source_hashes():
    result = original_hashes()
    for folder in ('benchmarks/engine', 'crates/cdna-evolution', 'crates/cdna-mcp'):
        for path in (ROOT / folder).rglob('*'):
            if path.is_file() and path.suffix in {'.rs', '.toml', '.py', '.lock'}:
                result[str(path.relative_to(ROOT))] = core.digest(path)
    return dict(sorted(result.items()))

core.source_hashes = source_hashes
if __name__ == '__main__':
    core.main()
