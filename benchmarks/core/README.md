# Native application-path benchmark

This harness measures the existing Rust application's critical paths. It creates
isolated temporary SQLCipher stores and calls the real Engine proposal,
confirmation, policy approval and ranking entrypoints. All records are synthetic
hypothetical fixtures. No personal reproduction accuracy is measured.

The fourteen scenarios are:

- JSON → shared feature extraction → model validation → Rust scoring → output
  serialization at batch sizes 1, 4, 16 and 1024. These are internal numeric loads;
  the public domain ranking contract still permits only 2–16 candidates.
- Engine memory ranking over 1, 100, 1000 and 1001 matching confirmed records,
  plus 1000 total records with only one exact match.
- The same corpus sizes with an approved candidate policy evaluated during rank.

The existing Mojo benchmark owns Rust/Mojo kernel and IPC comparisons. This
harness does not duplicate that comparison or select a backend from its results.

Each scenario records its first call, ten warmups, then nearest-rank p50/p95/p99
and extrema. Timing includes correctness assertions, response handling and output
serialization, so it is an application-path measurement, not bare database time.
Synthetic setup is excluded from these latency samples and separately timed.
SQLCipher reopen plus first rank is reported separately. Neither first-call nor
reopen measurements evict the OS page cache or claim a cold machine.

The indexed Engine accepts exactly 1000 complete matches and abstains when
1001 matching records require truncation. A 1000-record corpus with one match
must select that match; unrelated records do not trigger abstention. Every
response must deny execution authority. Each result records matching_records
separately from total size. Historical reports with the same 1/100/1000 matching
fixtures remain timing comparisons, but their former 1000-row abstention output
is a different behavior and must be labeled when compared.

Build with the repository heavy-command guard, targeting only this binary:

```sh
cargo build --release -p cdna-core-bench
python3 benchmarks/core/run.py --iterations 200 --out benchmarks/core/results/native.json
```

Run both commands through the canonical `revh command run --guard required
--class heavy` wrapper. The runner intentionally does not build implicitly. A
quick smoke may use `--iterations 3`; its percentiles are not useful estimates of
a latency distribution. Use at least 100 samples to distinguish a nearest-rank
p99, and repeat a stable release build before making performance decisions.

The report records the executable and relevant source SHA-256 hashes, available
toolchain versions, OS, CPU and memory information. If source or executable bytes
change during measurement, the runner writes an invalidated report and exits 2.
Available rustc version is not proof of the supplied binary's build provenance;
retain the guarded build receipt alongside the benchmark report.

Process CPU and peak RSS are obtained with `resource.getrusage(RUSAGE_CHILDREN)`
before any metadata subprocesses run. These cover the whole benchmark, including
fixture construction and cleanup. They are not attributed to individual
scenarios. CPU seconds per wall second is reported rather than a hardware-wide
CPU percentage. macOS RSS is already bytes; Linux KiB is converted to bytes.
Unsupported RSS units remain unconverted. No allocation count, thermal state,
per-scenario CPU/RSS, Python training, browser latency or transport latency is
measured.
