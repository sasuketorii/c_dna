# Public synthetic CEO demo

This is an invented persona, not a replica of a real CEO. No user data, human-confirmed observations, or human accuracy claims are included. Selecting a scenario immediately scores its explicit alternatives with the shipped 17 learned weights. It does not train in the browser or load Pyodide.

- `scenarios.json`: twelve authored business situations, explicit context facts and six attributes per alternative. No selected-answer field. Candidate order varies. `uncertain` deliberately abstains because material facts and an attribute are unknown.
- `model.json`: deterministic synthetic-only training output and provenance. The existing benchmark generator supplies 2,000 hypothetical, **unconfirmed** records. The existing internal `_fit_snapshot` numerical routine learns weights; production `train` eligibility is neither bypassed nor changed for user observations. Synthetic calibration is stripped and the model remains provisional.
- `parity.json`: Python learner reference scores and features, used only by tests; excluded from the application import graph.
- `engine.test.mjs`: actual compiled Rust WASM and TypeScript adapter checks, not mocked scoring. It verifies all scenario scores/features against Python, contributions summing to scores, abstention, invalid overrides, known-keyword matching and a deadline change reversing the delivery decision.

Attributes and context use the shared feature contract's normalized range -1 to 1. Greater cost, effort and irreversibility mean more burden; greater speed, reuse and customer impact mean more benefit. A null value is unknown, not zero. Facts are explicit within the **fictional fixture**, not verified real-world facts. UI sliders change context while holding the candidate estimates fixed; explanations describe only these model comparisons, not causal business forecasts.

Reasons are generated from actual winner-versus-runner feature contribution differences. Counterfactuals perform at most ten extra local WASM scores (each of five context facts at both endpoints), reporting only observed reversals. Absence of an endpoint reversal is not proof of global robustness. `elapsedMs` includes model loading when needed and the complete decision/reason/counterfactual calculation; `inferenceMs` measures the initial Rust scoring call plus JSON IO only. Neither measures UI painting.

Rebuild from the repository root:

```sh
cd learner
uv run python ../scripts/build-demo-model.py
uv run python ../scripts/build-demo-model.py --check
```

After the shared browser WASM has been built, verify from the repository root:

```sh
node --experimental-vm-modules apps/desktop/src/demo-data/engine.test.mjs
cd apps/desktop
npm run typecheck
```

Free text only locates known scenario keywords. It never silently invents candidate attributes or promises inference over arbitrary text. The demo engine persists nothing and sends no business inputs to a server.
