# Mojo comparison runtime

Rust CPU scoring remains the default. Mojo is an optional experimental numerical
worker, not required to launch the app. No IDs, text, context facts or explanations
are sent to it. Python trains; Rust extracts the versioned 17 numeric features;
Mojo computes a float64 linear dot product. JSON transport uses CPython stdlib,
so IPC results deliberately include that overhead.

## Reproduce

Run heavy compilation/tests/benchmark through the development harness guard.
The following are the commands to supply after the guard's `--` separator:

```sh
env MODULAR_TELEMETRY_ENABLED=false uv sync --project mojo --locked
env MODULAR_TELEMETRY_ENABLED=false mojo/.venv/bin/mojo build mojo/scorer.mojo -o mojo/scorer
cargo build -p cdna-inference --release --examples --locked
cargo build -p cdna-inference --release --locked
env MODULAR_TELEMETRY_ENABLED=false OPENBLAS_NUM_THREADS=1 mojo/.venv/bin/python mojo/benchmark.py
cargo run -p cdna-inference --release --example verify_mojo -- mojo/scorer
cargo test -p cdna-inference --locked
```

`uv.lock` fixes compiler/runtime packages at 1.1.0. Verified compiler identifies as
`Mojo 1.1.0 (8189361e)`. SDK telemetry is disabled in each invocation and in the
Rust adapter; this does not prove that all dependencies make zero network calls.
Network observation and packaged distribution acceptance remain outstanding.

`benchmark-results.json` records actual measurements, three runs, 20 warmups,
100 samples, 1/4/16/1024 candidates, cold start plus request, warm IPC p50/p95/p99,
NumPy native baseline, Rust native baseline and process high-water RSS.
Numerical tolerances are fixed before measurement at abs 1e-9 and rel 1e-8.
The fixture covers zero, positive/negative extremes, tiny finite inputs,
and deterministic random rows. Rust rejects nonfinite features and bounded
model validation prevents score overflow. Equal scores preserve caller order.
The standard 10000-case/2048-feature performance fixture and full product memory,
thermal and power observations are not claimed by this 17-feature experiment.

## API stability and official sources

Mojo APIs used: `std.python.Python.import_module`, PythonObject indexing/calls,
`Int(py=...)`, `Float64(py=...)`, `List[Float64]`, `range`, and `Error`.
Compiler upgrades require recompilation, numerical equivalence and IPC failure
checks. Worker failure is permanent for that adapter instance; Rust fallback
reports a reason and does not repeatedly respawn a failing process.

- [Official stable installation](https://mojolang.org/install/)
- [Python interoperability](https://mojolang.org/docs/manual/python/python-from-mojo/)
- [Python type conversions](https://mojolang.org/docs/manual/python/types/)
- [Mojo releases](https://mojolang.org/releases/)

Observed first-run warm p95 at 16 candidates: Rust IPC 0.276 ms, Mojo IPC
0.327 ms; Rust native 0.000333 ms. Mojo is not faster on this measured path.
These are synthetic microbenchmarks, not complete inference latency.

The checked-in benchmark is bound to its recorded binary/source hashes. A later
feature-boundary fix requires explicit context numerical facts to carry
`unit: "ratio"`; it changes feature validation, not the measured numerical kernel
or numerical-only IPC. Its unit tests are separate from this benchmark snapshot.
The measured host has 96 GiB RAM, so this is not 16 GiB reference-host acceptance.
