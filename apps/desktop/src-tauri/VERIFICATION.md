# Native runtime verification — 2026-09-22

Scope: native persistent startup and relocatable engine runtime only. No current
user vault or global Python installation was changed.

## Verified

- Native unit tests: 3 passed (readiness boundary, owned-process cleanup,
  init-once/existing/partial/dangling-symlink preservation and session lock).
- Release backend and `.app` builds completed successfully. The harness observed
  concurrent repository source changes and returned exit 50 after successful
  build commands; this is not a whole-worktree verification pass.
- Twice, `scripts/bundle-smoke.py` copied the app to an isolated temporary path,
  denied repository/Homebrew/uv/mise reads with macOS sandbox, and passed
  UI/auth/403 checks plus synthetic propose → confirm → train → preview. The
  learned candidate ranked first and remained provisional with null probability.
- Python isolated imports passed for sklearn, scipy, pydantic, LightGBM and the
  source package. No development venv or editable import path was used.
- Artifact scan found no `.pth`, `direct_url.json`, `.env`, vault database,
  key/PEM file or escaping symlink in the bundled learner.
- The moved `.app` launched via macOS `open --args --demo`; Orca computer-use
  read back the native demo title, rendered HTML and connected backend. No
  personal data was entered. API learning above is separate from UI interaction.
- Sending SIGINT to the exact owned backend after this check closed the shell;
  both owned PIDs disappeared. Keyboard quit automation could not assert focus,
  so no keyboard-quit success is claimed.

## Fixed artifact

- `cdna` SHA-256: `d750b1a65740f9406318d3c58f54cf07954b7832925c94fee5b9ed14f2b91e9a`
- `cdna-desktop-shell` SHA-256: `8eca20bbf8953ba028b16e3f53ea5972f4ca72b8b20c1430edc82cfb374ae151`

Copied learner distribution (snapshot, not latest concurrently edited source):

```json
{
  "python": "3.14.7",
  "provider": "uv/python-build-standalone",
  "python_build": "20260901",
  "openmp": "23.1.1",
  "packages": {
    "annotated-types": "0.8.0",
    "cloudpickle": "3.1.2",
    "joblib": "1.6.0",
    "lightgbm": "4.7.0",
    "narwhals": "2.26.0",
    "numpy": "2.5.3",
    "pydantic": "2.13.5",
    "pydantic_core": "2.46.5",
    "scikit-learn": "1.9.1",
    "scipy": "1.18.1",
    "threadpoolctl": "3.7.0",
    "typing-inspection": "0.4.4",
    "typing_extensions": "4.16.0"
  },
  "lock_sha256": "20d9d4f3c5a535e86f68ad1f94a3f25057fe32abb6c26a44bdf14468e84cedb6",
  "source_sha256": {
    "cdna_learner/__init__.py": "32aab1a925050497e3e1a97a209c14d4e3da874d3982b9bcc819d8b007b390dd",
    "cdna_learner/__main__.py": "5a513a4ff5b076fd41c1a39968b0b670c5d7ceaa0e0fc311c7b8dffb05e12f97",
    "cdna_learner/worker.py": "4db7d5852b18ee2ac8e9728b721594ff0613ab5ff4b0aaafa5e87f14981b39dd"
  }
}
```

The last smoke child exited 0 in 9.65 s; harness peak RSS was
157 MB and orphan count was 0. The guard itself returned 50 due
to concurrent source drift, including on a scoped-root retry. Evidence is bound
to the artifact hashes above; newer engine/learner edits require another prepare,
build and smoke. Runtime receipts were written as `cdna-native-tests.json`,
`cdna-native-backend.json`, `cdna-native-bundle.json` and
`cdna-moved-runtime-smoke-scoped.json` in the system temporary directory.

## Remaining acceptance and limits

- Real persistent first launch, Keychain permission denial/cancellation and
  reopening under a clean macOS user account are not exercised. The init flow
  reuses the CLI and was checked with isolated mock initialization; no existing
  user vault was touched.
- Native locking excludes other native instances, not independent CLI users.
  The CLI owner was informed that atomic destination creation is needed to close
  its existing check-then-create race.
- macOS arm64 only. Host libomp is version-checked at build time and vendored;
  the moved learning test proves it is not needed at runtime.
- `codesign -dv` shows linker ad-hoc signature, no TeamIdentifier and no sealed
  resources. Developer ID signing, notarization and clean-machine installation
  remain unverified.
- Codebase graph found `Engine::train` and HTTP `command` as run_training callers;
  its Rust scope verdict is unverified (language not evaluated), and actual call
  sites were checked directly. No graph completeness claim.

Reproduction commands and runtime provenance are in [README](README.md).
