# Learned inference implementation evidence

Date: 2026-09-22. Synthetic fixtures only; no real human accuracy or deployment acceptance.

## Public boundary

`Engine::new` defaults to no learned trust. Hosts configure distinct pinned evaluator and human public keys using `with_evolution_trust`. CLI `exec` accepts host-level `--evaluator-public-key` and `--human-public-key` paths. Request JSON cannot configure trust. Existing owner-only `activate_model` now activates only the verified signed evolution candidate. The separate evolution CLI registers, evaluates, approves, activates and rolls back using revision CAS.

`prepare_candidate` converts a current provisional learner artifact into unsigned canonical candidate bytes. Conversion preserves weights, calibration slope, domain scope and abstention margin and cannot widen candidate missing-pattern support. Explicit evaluator numerical support is required for context masks, OOD feature ranges, maximum margin and validity period. Conversion alone writes no state and never promotes.

Each learned request validates encrypted evolution state against current vault epoch and host keys. Executable weights and numerical support share the approved artifact hash. The runtime reloads revision and epoch before returning. Exact confirmed memory retains precedence; conflicts and missing/unknown evidence prevent model fallback. Policy rejection never substitutes the second model choice. Only supported two-choice scopes return `pairwise_probability` with identified left/right choices; generic `probability` and `top_choice_agreement_estimate` remain null. Raw scores are actual `LinearScorer` output.

Evaluation rejects duplicate held-out test families. Every evaluated domain must independently pass the absolute minimum sample/quality/Wilson and paired-baseline uncertainty gates; a sparse secondary domain does not inherit pooled acceptance.

## Verification

Guarded `cargo test -p cdna-app -p cdna-evolution -j 2`: all 99 tests passed, including eight learned-inference integration tests and seven evolution tests. The learned suite covers signed register/evaluate/approve/activate through unseen ranking, exact raw score and sigmoid, rollback, wrong key, permissions, epoch invalidation, stale replay, detached weights, memory precedence/conflict, policy rejection, unsupported domains/missing patterns, tie/margin/OOD/period restrictions, and unsigned conversion nonpromotion.

Two CLI vault-reservation tests cover concurrent creation and dangling leaf rejection. Root-requested module wiring for natural language, local provider, training pagination and import pagination was included in this compile/test run. No UI or deployment was changed.

Receipt basename: `cdna-inference-tests-3.json` in the external temporary receipt directory. Child cargo exit was zero; guard wrapper exit was 50 because concurrent work changed repository source during the run. This is successful test execution, **not** final frozen-tree guard acceptance. Coordinator must obtain that receipt after parallel writers settle.

Codebase graph search found `Engine::execute` as the direct caller of `Engine::rank`. Trace returned `unverified` because Rust language evidence is not evaluated and short-name collisions exist; actual callsites and constructor migrations were checked in source. No graph completeness or freshness claim is made.

## Remaining acceptance

Independent signed benchmark and final stable-source verification belong to coordinator/verifier. Production requires trusted verification material, calibration/OOD scope, evaluator receipts and explicit human approval. Synthetic test signatures are never production approval or evidence. No real model was promoted or deployed during this implementation.
