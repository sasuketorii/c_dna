# Static playground deployment

## Current publication and stopped acceptance (2026-09-22)

The coordinator relayed the user's cancellation of playground work while the last
deployment was already in progress. It completed successfully at the next safe
process boundary. No further builds, deployment, browser checks or rollback were
performed after receiving the stop instruction.

- URL: <https://cdna-playground.sasuketorii-business.workers.dev>.
- Latest deployed version: `473f9ef1-9b79-4709-9d59-0de50257bb54`.
- Latest artifact SHA-256: `2cbc74831ea3629c1b174bca09f6da3b3aa9d963f2536612d8c843f1af0df0a2`.
- Configuration SHA-256: `2f045fc5eab2667701db991dd09af1de369d08a8ce02bdf82d89c611f8464342`.
- 32 files, 38,050,492 bytes; 30 public assets and two metadata files.
- Known accepted rollback version: `a71ffff4-1a10-426f-93a1-45bb65918a55`.
- Intermediate version: `67d80291-97ae-4248-a5fb-a2a414758773`.
- **Latest hosted acceptance: NOT COMPLETED, stopped by user instruction.**
  Do not transfer intermediate browser/hash evidence to the latest version.

### Pre-deployment gate and local evidence

`DEPLOY: GO`, `MODE: DELTA` was recorded before publication for this frozen
artifact, authorized static-only Worker and unchanged configuration. This was a
pre-deployment decision, not final hosted acceptance. Owner: release coordinator;
evidence validity ends on changed target/configuration/source/artifact or release
completion. Cost, security scope, monitoring and kill switch below remain the same.
Official static pricing and headers documentation was rechecked.

Fresh live readback before the follow-up publications showed zero bindings,
no observability, workers.dev enabled and preview URLs disabled. Both frozen
Wrangler 4.136.1 dry-runs reported no bindings. No application Functions, route,
secret, database or other binding was added. Latest post-deployment provider
readback was not performed after cancellation.

Executed the existing `prepare.py`, `build-domain.sh`, and `build:static` using
the development harness. Learner SHA-256 remained
`4db7d5852b18ee2ac8e9728b721594ff0613ab5ff4b0aaafa5e87f14981b39dd`.
Actual Pyodide training reversed winners with opposite confirmed choices; Rust
WASM rejected wrong units and duplicate keys. The first smoke's computation passed
but its receipt detected concurrent repository changes; the repeated verification
passed with an unchanged source snapshot. Shared WASM scoring smoke passed.

The final UI-only rebuild followed the coordinator's result-first/collapsed-controls
change. The 59-input source inventory was checked unchanged before this final
upload, and all copied bytes matched preflight. Runtime/learner source was not
edited by this release worker. The upload completed with the exact version above.
Local command receipts are named `cdna-final-prepare.json`, `cdna-final-domain.json`,
`cdna-final-wasm-confirm.json`, `cdna-final-scoring.json`, and
`cdna-final2-static.json`; frozen preflight and deploy receipts are
`cdna-final2-preflight.json` and `cdna-final2-deploy.log` in the operator's temporary
evidence directory. These temporary receipts are not durable CI evidence.

### Intermediate hosted observations only

Version `67d80291-97ae-4248-a5fb-a2a414758773` passed all 30 public asset SHA-256
comparisons (following Cloudflare's canonical redirect for `index.html`). The
native Orca browser selected the delivery example and displayed outsourcing as
the judgment, three reasons and actual measured 10.4 ms. Changing deadline pressure
from 0.95 to -1 changed the result to a two-week reduced-scope trial, measured 2.6 ms.
Unknown input requested clarification. Public resource entries contained only
same-origin JS/CSS and Rust domain WASM, with no Pyodide or worker request; the
console was empty. These times are single-session observations, not performance
SLOs. Studio opened successfully, but the follow-up save-confirm-train-preview/reload
sequence was not completed before cancellation. Initial-release Studio acceptance
below remains historical only.

### Latest frozen asset hashes (local preflight, not hosted readback)

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `_headers` | 419 | `036ed857fa16df57d4246b7226e1abd7842ca62608806d4433dd8530abb1c2e1` |
| `_redirects` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `assets/App-CFBYZ_ow.js` | 185256 | `6c97aeb4b4e85492d8a7c55d4bd3f1c4e934457e3d668a2e558f2fddc84441e7` |
| `assets/PublicPlayground-C-mw7BGB.css` | 7220 | `3780219b688dafb8ece37ad93583b0459a36324f025453d271a7969635e675c8` |
| `assets/PublicPlayground-DSAfuMrP.js` | 32957 | `6aeb9804e6873ec21438096496f773faf763b70dd5d1bfa8fcded17028ea7a71` |
| `assets/adapter-Cp9pIPdA.js` | 9198 | `3161dbde9bc5e6344500f3f1cac245fa9092a7f531cf58b88007ff5ff7ce1fa4` |
| `assets/createLucideIcon-B2Wh66-4.js` | 42135 | `3f086d9a887bd845093492293f9cf6ff8c6358f596f00823aa019ef888e35791` |
| `assets/index-CmBu7kGJ.js` | 247341 | `24831b4acd88ac16d378e97d00344d46e242400e2d717dfa6f9523eeb78c66fa` |
| `assets/index-iEVQCYC-.css` | 53791 | `60eac32ca2db6df062c771d7d9cc18cf577342e9bb5eafaaf0484ba178572a04` |
| `assets/worker-B-ENxdC_.js` | 2699 | `6cfc64ad081379398a8dc0fc20dda8bdb39db0066267ff29ab7b5457a2302da9` |
| `index.html` | 420 | `cc41c6c94731b116dd600567b65e2903c3ebfc7ad632350dc1fb599c60916fe9` |
| `runtime/domain/cdna_browser.d.ts` | 1849 | `52dbfafd1e8a876435ddba2604c4168c014d6c0a398ccf6accb71477d8512dfa` |
| `runtime/domain/cdna_browser.js` | 10389 | `a103b190f9e78414a830e1c12a5da6de9c8c606e3644a7885f5be769d09a3326` |
| `runtime/domain/cdna_browser_bg.wasm` | 413895 | `730df199b8a1113c627797d1f4bac2d3c37cc4262f15f99c05803b2f90c81f75` |
| `runtime/domain/cdna_browser_bg.wasm.d.ts` | 603 | `65e792f694e1690fa5d4e3d53984667119e05a8bd1c29b21104b0123ca639eac` |
| `runtime/learner.py` | 24994 | `4db7d5852b18ee2ac8e9728b721594ff0613ab5ff4b0aaafa5e87f14981b39dd` |
| `runtime/manifest.json` | 2714 | `c3acabec557dddd050d21dc57a93914f5e6d12a52c00069eadbe2f918b6d4fcb` |
| `runtime/pyodide/annotated_types-0.7.0-py3-none-any.whl` | 11963 | `6541523b4b16c1953d2fc9b82fb86c540eea9f78fca329ab371ed13a8b0faa51` |
| `runtime/pyodide/joblib-1.5.3-py3-none-any.whl` | 180317 | `fb2860ff70d0b6b43a3df5e3ce52d3a938ad2fcacd80a3f2f3f37e930843e607` |
| `runtime/pyodide/numpy-2.4.6-cp314-cp314-pyemscripten_2026_0_wasm32.whl` | 2960568 | `a292c1f5d7d8a2208cd5e94fc467604c131cabcd2fc14fed6eefde121e7fabdf` |
| `runtime/pyodide/pydantic-2.12.5-py3-none-any.whl` | 463578 | `04854875001e07a8a74d87b7c80296cba168773d8c9804e232d695f82484d7d7` |
| `runtime/pyodide/pydantic_core-2.41.5-cp314-cp314-pyemscripten_2026_0_wasm32.whl` | 1310164 | `54703fa42a0d4d574283cfb8f08050ba7820c9a42b0a21bc1286407d9a9e54da` |
| `runtime/pyodide/pyodide-lock.json` | 119077 | `5dc2fc119108bc148c7457dc86e7675b5c87e1cafd420b9c34c1eaef7b36c010` |
| `runtime/pyodide/pyodide.asm.mjs` | 1250344 | `f7cdc8ece80678ceb712f8e65ebe6d3a83203a180c399865f49612a051693635` |
| `runtime/pyodide/pyodide.asm.wasm` | 9598218 | `cc36e3cab04fdfc9a63ff13eb52eae2b911bf46c025cc7b281f394bd3de1d5e6` |
| `runtime/pyodide/pyodide.mjs` | 17931 | `6f1d60f7bf529beb300f0f47983c921d3982363640ba20af0e38efdddbc66109` |
| `runtime/pyodide/python_stdlib.zip` | 2545637 | `fa1957e5777068fc4f7437f96d860ae2fbe9c19732ba06c84e004ec16dd7dd7a` |
| `runtime/pyodide/scikit_learn-1.8.0-cp314-cp314-pyemscripten_2026_0_wasm32.whl` | 4449204 | `432f560f54c5283d13e9d7c11dddc8bdf2f15aec326cc1d7314f600786154181` |
| `runtime/pyodide/scipy-1.18.0-cp314-cp314-pyemscripten_2026_0_wasm32.whl` | 14029750 | `17ee329a957863516d1bb6a6aaa0c60576fac027e9cd3d43de27f58b5b599b50` |
| `runtime/pyodide/threadpoolctl-3.6.0-py3-none-any.whl` | 18638 | `3521783ffb6bf212e03fb67ec1955af206d6ba7ea09ac21f200875077bbbe7c2` |
| `runtime/pyodide/typing_extensions-4.15.0-py3-none-any.whl` | 44613 | `4fec68770a05408e668a4fa53d6bf4f0f684b9dacf820c7c5d6e97d3ad225e29` |
| `runtime/pyodide/typing_inspection-0.4.2-py3-none-any.whl` | 14610 | `deb989630985f7a7296b90357a947a804a7612cecbb6439d6e71128e01f61051` |

### Latest frozen input hashes

| Source | SHA-256 |
| --- | --- |
| `Cargo.lock` | `c7b94c0e6fdf4e7c6e7535741914d6b2d7bff637b555cadce0815dbf4e7153a7` |
| `Cargo.toml` | `106f49bfa891fd7f7d45e99b3f574e41f044ebac88d22c6977fc78fb43599d14` |
| `apps/desktop/package.json` | `6285f1d4906eff58f848466d6e2dad751f21f60d37a0b86b61bcff0914227bd9` |
| `apps/desktop/pnpm-lock.yaml` | `e71e6e6205619628fa2d54e2e720cc1d99f72590cef58731e17255774ee7d002` |
| `apps/desktop/src/App.tsx` | `c4fa85a3e731070093cc6fbd8ccfab1b225a2d73ee4adced3eaba33a80c7eb09` |
| `apps/desktop/src/PolicyPanel.tsx` | `ef30f63c91019620084340284adf717ae639aeb2a3efbe2f3e77a917088cfec0` |
| `apps/desktop/src/api.ts` | `96af737d69f900c900cb05607307e85732e540f7d1176012a36e614f39fb9239` |
| `apps/desktop/src/assessment/AssessmentPanel.test.mjs` | `86f4e8d17d26016049739c18f221e139d9d4fc66bf48ef7c28cfb1720ae4c65e` |
| `apps/desktop/src/assessment/AssessmentPanel.tsx` | `b7d50a40142089bee6aa202458a68a4e3f606bf9f1f40c1ada98fa42e447c515` |
| `apps/desktop/src/components/ui/badge.tsx` | `d9fb280e266119c4b62a4c518e63152de27b094c68d3e5f8929d05bfb0c1bc6c` |
| `apps/desktop/src/components/ui/button.tsx` | `fe8220afc0ec9b62c7a520f4e6eb0b56b3b9e951eb963183c29d274395ee476c` |
| `apps/desktop/src/components/ui/card.tsx` | `816b335ae2c9b17a7daf5887095df94d2aebc36ce7ad491424ec3b36511bd8e0` |
| `apps/desktop/src/components/ui/empty.tsx` | `e65139139c60b2cab7adc71776565b55d867fa79b7ddf6b428b01af0fe0be7d1` |
| `apps/desktop/src/components/ui/input.tsx` | `b8d0c2a2ab66080b5fe9a3467884f896eb6f9e6b45ac0185925e831f551362e6` |
| `apps/desktop/src/components/ui/label.tsx` | `86232709f634da7f357a8aaec0e815f3f562604bb67202c88997fae97659b450` |
| `apps/desktop/src/components/ui/separator.tsx` | `8a1d9397433ccf70631104f377b0e778754582a74c93a57cf13a2f2dcba778bf` |
| `apps/desktop/src/components/ui/sheet.tsx` | `67a782455ac3890c74a4bc5ca9bd558a73047b4444b55a96d61419d606183543` |
| `apps/desktop/src/components/ui/sidebar.tsx` | `99a33ca61431687ae02cc3c4daeaa0eae33d5d45dd842a08c4962e9ac9081196` |
| `apps/desktop/src/components/ui/skeleton.tsx` | `4c2af7fa9c645358a0fb188779deab6f0b981733b62befc25e18aaebea1350dc` |
| `apps/desktop/src/components/ui/textarea.tsx` | `6d096ef81ee782d94696d0ddad1c926e530d952b54b9ff9e8b837d8bbeea8d00` |
| `apps/desktop/src/components/ui/tooltip.tsx` | `f83d5f511812573f4ac98b866d397a46fda0f27eeb0af0ea89af01969eefbc5a` |
| `apps/desktop/src/demo-data/README.md` | `4c6b3c7ce5ad752ae80270aca117b0dfd285fba0d86be42e381e79413a9f50d1` |
| `apps/desktop/src/demo-data/engine.test.mjs` | `700d3671f62ef306714b5716fd4ba774a1eacf1ea5415ebbbe957357d7e12378` |
| `apps/desktop/src/demo-data/model.json` | `3aaf463c7e8b29584b39f6e939d6046e94eb50cf2134ec079fb6f9c341b9b27b` |
| `apps/desktop/src/demo-data/parity.json` | `39e285aa78584e428b65acf8d5f04f2099a4122663d87594d2aeeddc7c64df92` |
| `apps/desktop/src/demo-data/scenarios.json` | `d84afb005f652f979ee96cc3650adb0b55a2402acefdfdafbd9ef7707dd20e71` |
| `apps/desktop/src/demo-engine.ts` | `a12e197ef77e3f2d71fea8a427a1f9f7ebf6bb00d6ec19689adbe5a144ba503c` |
| `apps/desktop/src/exposure.ts` | `4607143d5967d24b79da105b02829dd200482c9e0130af62d047b9270639827f` |
| `apps/desktop/src/hooks/use-mobile.ts` | `ad0936f84f1df79d3697bfbff9c18f8ad58431c1cbaf2359c6a853b0fcc9f28b` |
| `apps/desktop/src/ja.ts` | `8dcfeed6eb3271706ccce88de813d7f32569e78f6410fdb5523691411b78641f` |
| `apps/desktop/src/main.tsx` | `7de92309b76061a3aa636f6245955c2a2a50c388ca2274c7967c923c1aa0f156` |
| `apps/desktop/src/onboarding/CloneSetup.tsx` | `23a0791239ac5bb4da8892523245edb1a5ef8c18b6614378b94dabd9cc5d4000` |
| `apps/desktop/src/onboarding/client.test.mjs` | `3290239cf2f804ebfb5e5d9bda1c36916466f99a7b05aff0b711d536389804c6` |
| `apps/desktop/src/onboarding/client.ts` | `18c6e683d6c8892fb3a15e3134838062c2a4c254aebef3ce2f532f378c456003` |
| `apps/desktop/src/playground/PublicPlayground.tsx` | `1828ae19df8120e00f49507985dcf249d71ea429e9a62ac41f5cc7583311ddd5` |
| `apps/desktop/src/playground/playground.css` | `e6511f0a8a4b5dbe69317bef0f923fce1addda05fc0bf4a34943999389319ca9` |
| `apps/desktop/src/policy-form.ts` | `e7a5497e1dfa17192238033ddf6c08c98057f9cbf30eef738b0a3a6b819645b9` |
| `apps/desktop/src/static/adapter.test.mjs` | `d2bbccf15a09149f89c6a828b7237be85a2ed69d787f0162c2f9b1d909bf6008` |
| `apps/desktop/src/static/adapter.ts` | `5aade8df16c7e6bd1fc2532af664f66f323936ac22e2fbefd9e5baa4fd081bda` |
| `apps/desktop/src/static/validation.ts` | `19915d0be31b3500976cc3dd825ef50ffdb67efb82eac7e3e6e2eece2f5b824c` |
| `apps/desktop/src/styles.css` | `50e1f856a8913687fb41d20467ede744e77ebf38a81eb6f003efb9200854d779` |
| `apps/desktop/src/vite-env.d.ts` | `65996936fbb042915f7b74a200fcdde7e410f32a669b1ab9597cfaa4b0faddb5` |
| `apps/desktop/vite.config.ts` | `5c2be5ce150061a07f855da5f184770e571893b0bae3aacce13baea2fcc67dc2` |
| `apps/desktop/wrangler.jsonc` | `2f045fc5eab2667701db991dd09af1de369d08a8ce02bdf82d89c611f8464342` |
| `crates/cdna-browser/.gitignore` | `712b0c9059df6fd7b9c66149cc9528b99db82f63fd7699539109b5175b07e28c` |
| `crates/cdna-browser/Cargo.toml` | `467bc15f02159815a3cf4de39ef6b2db24c3c7166f81d8a702c4b4cc91123a23` |
| `crates/cdna-browser/pyodide-lock.json` | `5dc2fc119108bc148c7457dc86e7675b5c87e1cafd420b9c34c1eaef7b36c010` |
| `crates/cdna-browser/runtime-integrity.json` | `d571e086610c9c39a0d4c587b0063b7cd40cf8a5dd9cd387dae9ca306bfb467a` |
| `crates/cdna-browser/runtime.ts` | `8a510bd5f18520e079211284982c23c1746885d2f512976b1e4c7464aa6ffe60` |
| `crates/cdna-browser/scripts/build-domain.sh` | `984c9b8ed190593bca41e1a57a7c35c660d4744aa3727c5ba21962b92a01f2a8` |
| `crates/cdna-browser/scripts/install-bindgen.py` | `3e3dd94d5400d3a22993b5c826bf2d23ae99fcc81f43308cbb927b939a8de29c` |
| `crates/cdna-browser/scripts/prepare.py` | `1649b77e5e411b47c11c678e148f5a1bead351dd1df7487f39615e1953de5b65` |
| `crates/cdna-browser/scripts/retry-test.mjs` | `17234a6c332d25be7ec5d1388079abd43ebee3aca1d8410bc9e4809e63b0e23d` |
| `crates/cdna-browser/scripts/scoring-smoke.mjs` | `b5ca69149ef1addaf840a5b0b679f98927a183629a60677e30aa2f04eaf18d0c` |
| `crates/cdna-browser/scripts/smoke.mjs` | `ad84fc6f5c95753b19fbf03424958f98784c80e47edd5cbdd54eb4590e1d4436` |
| `crates/cdna-browser/src/lib.rs` | `7c7b10e15c8238d836629434172c3818fa5ff34f5304961dc7622958e1de8806` |
| `crates/cdna-browser/worker.ts` | `a4e4bf90940bc20d77175649c040dc8ba894dbce1cd0b65412e6cd41ab0a7419` |
| `crates/cdna-domain/src/lib.rs` | `ef10c67bf77adac9d226b359a75653ae32fd6bcbc6d0c2ce482f80a16c7310e3` |
| `learner/src/cdna_learner/worker.py` | `4db7d5852b18ee2ac8e9728b721594ff0613ab5ff4b0aaafa5e87f14981b39dd` |

## Initial release gate (2026-09-22)

`DEPLOY: GO` for the assets-only initial publication; hosted acceptance passed below.
`MODE: BASELINE`. Owner: c-dna release coordinator. Evidence is valid for this
artifact and configuration only, until this release finishes or either changes.

- Account: `1b0d66d05b34ffe694010807c0c360c7`, verified using existing Wrangler OAuth.
- Worker: `cdna-playground`; read-only account inventory found no existing collision.
- Existing account subdomain: `sasuketorii-business`, read from Cloudflare API.
- Intended URL: `https://cdna-playground.sasuketorii-business.workers.dev`.
- Authorization: user requested a completed public static playground; coordinator
  confirmed the current build is ready and approved this non-conflicting name.
- Configuration SHA-256: `2f045fc5eab2667701db991dd09af1de369d08a8ce02bdf82d89c611f8464342`.
- Artifact SHA-256: `515cef0edffdcfa76965249e2d81aa232d0009798efa0f95c58073db799bd7c3`.
  This hashes the ordered JSON file inventory emitted by the preflight command.
- 28 files, 37,909,357 bytes total; largest file 14,029,750 bytes (SciPy wheel).
- Wrangler `4.136.1` fixed explicitly, latest npm stable observed on this date;
  its deployment dry-run succeeded with no bindings.

## Scope, cost, and security

Only `apps/desktop/dist-static` is uploaded. There is no application Worker
entrypoint, Functions directory, API, SSR, origin server, route/custom domain,
database, KV, R2, D1, Durable Object, Queue, AI binding, cron, or secret binding.
Missing paths use a static 404 rather than an API or SPA fallback.
Desktop/server source and local credential files are outside the uploaded tree.

The preflight verifies runtime manifest hashes, strict asset limits, security
headers, absence of desktop API routes and common credential markers, and an
allowlist of Wrangler configuration keys. Marker scanning is not a proof that
arbitrary secrets cannot exist; the runtime manifest and public-build provenance
are also required. Runtime code loads Rust/WASM, Pyodide, learner code, and package
wheels from the same origin. CSP restricts connections and workers to that origin
and permits WASM compilation without general JavaScript unsafe-eval.
The preflight walks the selected packages' transitive lockfile dependencies and
checks every local wheel hash, including normalized underscore/hyphen package names.
An isolated frozen fixture passed; symlink assets and an added Worker entrypoint
were both rejected in negative checks.

| Meter | Expected | 10x | Bot/crawler | Client retry loop |
| --- | --- | --- | --- | --- |
| Static asset requests/storage | $0 incremental | $0 | $0 | $0 |
| Application server CPU / data products | Not provisioned | Not provisioned | Not provisioned | Not provisioned |

Cloudflare documents static requests as free/unlimited and asset storage at no
additional cost. This does not claim the entire existing account costs $0.
Account-wide subscriptions returned HTTP 403 with the existing OAuth scope;
unrelated paid products/plans and account budget alerts were not audited or changed.
They are outside this assets-only change and do not change its static pricing.
There is no billable application meter requiring a new budget alert. Browser
CPU, memory, and download bandwidth remain client costs; training is lazy and
bounded by the browser runtime. Public source/runtime files are intentionally
downloadable. Data remains ephemeral; provisional output grants no authorization.

## Reproduce and operate

From the repository root, after the authoritative runtime/frontend build is ready:

```sh
node apps/desktop/scripts/deploy-preflight.mjs
npx --yes wrangler@4.136.1 deploy --config apps/desktop/wrangler.jsonc --dry-run
```

Freeze the validated directory and its config into an isolated release directory
before publishing so concurrent builds cannot alter the upload. Compare every
file's bytes/hash against the preflight receipt and deploy the copied config using
the same fixed Wrangler version. Do not rebuild between approval and upload.
Any configuration, target, runtime, or artifact change requires a fresh gate.

Monitoring: the release operator checks HTTPS status, CSP, asset hashes, browser
console and same-origin runtime requests after each deployment. Any mismatch,
runtime error, unexpected outbound request, or non-static binding blocks acceptance.
No telemetry or hosted logging product is provisioned by this release.

Kill switch: disable the `cdna-playground` workers.dev public endpoint in the
Cloudflare dashboard (Workers & Pages → cdna-playground → Settings → Domains &
Routes); preview URLs are disabled. Verify the URL is no longer serving the app.
For the initial publication there was no prior version; subsequent releases retain the known accepted rollback version recorded above.
For subsequent releases, select the prior recorded version in Deployments and
roll back, then recheck the served hashes. There is no server data to migrate or
restore. Browser tab memory is not a backup and disappears on refresh/close.

## Initial-version hosted acceptance (historical)

Published URL: <https://cdna-playground.sasuketorii-business.workers.dev>.
Version: `a71ffff4-1a10-426f-93a1-45bb65918a55` (2026-09-22).

All 26 publicly served files matched the frozen artifact byte-for-byte by SHA-256.
`_headers` and the empty `_redirects` are deployment metadata, not public files.
HTTPS HTML returned 200 with the expected CSP, nosniff, no-referrer, and permissions
headers. `/api/command`, `/_headers`, and `/.env` returned 404. Initial Python
urllib retrieval returned 403; curl and the native browser succeeded, so this
does not claim that every HTTP client or geography was tested.
Cloudflare read-only settings readback confirmed zero bindings, no observability
configuration, the workers.dev endpoint enabled, and preview URLs disabled.

The native Orca hosted page successfully saved one fictional observation,
confirmed it (revision 2), trained in the browser, created a provisional model,
and returned four raw preview scores. The screen retained the unverified
accuracy, unapproved model, no-execution-authority, and ephemeral-data labels.
Console capture contained no messages. Orca's network view exposed the browser
worker request but not its complete nested fetch trace; same-origin runtime
closure is additionally supported by CSP, source/lock inspection, complete hosted
asset readback, and successful browser execution. This is not a full network
packet audit or independent model validation.
Reloading the page returned both confirmed-observation and candidate-model counts
to zero, verifying the documented ephemeral storage behavior for this session.

Concurrent learner source edits occurred after the frozen publication. The
strengthened freshness gate correctly rejects the current workspace with
`Stale learner source`; this does not change the immutable published version.
The follow-up rebuilt/staged the runtime and passed preflight, as recorded above.
This initial version does not include those later edits.

## Official sources

Checked 2026-09-22:

- [Static Assets billing and limitations](https://developers.cloudflare.com/workers/static-assets/billing-and-limitations/)
- [Static asset file count and 25 MiB per-file limits](https://developers.cloudflare.com/workers/platform/limits/)
- [Assets configuration and bindings](https://developers.cloudflare.com/workers/static-assets/binding/)
- [Static response headers and CSP](https://developers.cloudflare.com/workers/static-assets/headers/)
- [Wrangler configuration](https://developers.cloudflare.com/workers/wrangler/configuration/)
