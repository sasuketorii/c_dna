# C-DNA user interface

React 19, Vite 8, TypeScript 7, Tailwind 4, Base UI/shadcn and TanStack Query. Exact dependency versions and the pnpm lockfile are checked in. UI text is maintained in `src/ja.ts`.

## Local application

From this directory:

```sh
pnpm install --frozen-lockfile
pnpm build
```

The repository's local HTTP application serves `dist`. Open the startup URL containing its session fragment. The fragment is removed immediately and the token remains in memory only; refreshing requires reopening the startup URL. Vite's development server alone does not provide the application API.

## Static browser playground

```sh
pnpm build:static
```

This produces `dist-static` with the browser adapter instead of the HTTP API transport. The Vite `static` mode sets the compile-time transport flag; no secret or environment file is required. Prepare the shared browser runtime assets before the deploy build using the instructions in the repository's browser runtime package. Generated assets live under `public/runtime` and are not committed.

The playground keeps hypothetical observations in browser memory only. Reloading loses its records. Training runs in a dedicated worker using the shared learner; a failed or unavailable runtime is shown as an error, not replaced with simulated output. Provisional scores are never represented as calibrated probabilities or action permissions.

Both distributions implement recording, explicit human confirmation, revision-bound correction/deletion, confirmed-memory matching, candidate-model training and preview, and a learning coverage map. Local policy forms support draft creation, explicit owner approval, read-only inspection, revocation and policy checks through the existing application boundary. The static playground explicitly marks policies unsupported; connections and personality remain informational screens. The browser entry point does not provide a native Tauri package.

Public component provenance and MIT attribution are documented in `SOURCE.md` and `SHADCN-LICENSE.md`.
