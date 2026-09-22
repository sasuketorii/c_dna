# Implementation ownership map

確認日: 2026-09-22。所有は人名ではなく、機能を変更する際の正本パスを示す。
受入状況はこの索引で複製せず、各契約とリリース証拠を参照する。

| 機能・境界 | 所有パス | 呼出先・契約 |
| --- | --- | --- |
| 日本語画面、質問と確認、比較表示 | `apps/desktop/src/App.tsx`、`apps/desktop/src/PolicyPanel.tsx` | `apps/desktop/src/api.ts` |
| UI command transport | `apps/desktop/src/api.ts` | static buildはstatic adapter、local buildは認証付きcommand endpoint |
| 静的体験の状態・epoch・出力境界 | `apps/desktop/src/static/adapter.ts` | `STATIC_PLAYGROUND.md`、`adapter.test.mjs` |
| 静的入力検証 | `apps/desktop/src/static/validation.ts` | browser runtime → domain WASM |
| Workerの操作・停止・期限 | `crates/cdna-browser/runtime.ts` | `crates/cdna-browser/worker.ts` |
| WASM検証、Python初期化・共有学習呼出 | `crates/cdna-browser/src/lib.rs`、`crates/cdna-browser/worker.ts` | `STATIC_RUNTIME.md`、browser scripts |
| native windowとsidecar起動 | `apps/desktop/src-tauri/src/main.rs` | `apps/desktop/src-tauri/README.md` |
| ローカルHTTP/CLI起動、セッション境界 | `crates/cdna-app/src/main.rs` | `crates/cdna-app/src/lib.rs` のEngine |
| アプリ操作・snapshot学習・preview | `crates/cdna-app/src/lib.rs` | `crates/cdna-app/CONTRACT.md` |
| 初期人物理解のプロンプト・Source取込 | `crates/cdna-app/src/bootstrap.rs`、`docs/prompts/bootstrap-profile-ja.md` | `BOOTSTRAP.md`、`contracts/bootstrap-profile.schema.json` |
| LLM教師・本人回答・出典と質問の接続 | `crates/cdna-app/src/with_ai.rs`、`crates/cdna-store/src/coaching.rs` | `WITH_AI.md`、`TEACHING_LOOP.md` |
| 学習適格snapshotと領域指定 | `crates/cdna-app/src/training_snapshot.rs` | confirmed paging → Python worker |
| 会話選択・取込ページ・再送 | `crates/cdna-app/src/import_workflow.rs`、`importer.rs` | `IMPORT.md` |
| 自然言語抽出の入力・期限・結果検証 | `crates/cdna-app/src/natural_language.rs`、`local_provider.rs` | `NATURAL_LANGUAGE.md`。意味精度は未受入 |
| 署名済みモデルの推論適用 | `crates/cdna-app/src/inference_runtime.rs`、`evolution_bridge.rs`、`evolution_cli.rs` | `learned-inference-acceptance.md`、`EVOLUTION.md` |
| Agent grant由来のauthority | `crates/cdna-app/src/mcp_bridge.rs` | Engine、Store |
| MCP transportとtool schema | `crates/cdna-mcp/src/lib.rs` | `crates/cdna-mcp/CONTRACT.md` |
| proposal/rank型・意味検証 | `crates/cdna-domain/src/lib.rs` | `contracts/` の生成schema、browser WASMからも再利用 |
| 暗号化保管・改訂・削除・モデル失効 | `crates/cdna-store/src/lib.rs` | `crates/cdna-store/CONTRACT.md` |
| 学習objective・特徴量・評価 | `learner/src/cdna_learner/worker.py` | native subprocess / Pyodideが同じsourceを実行 |
| native推論・特徴抽出 | `crates/cdna-inference/src/lib.rs` | `crates/cdna-inference/CONTRACT.md` |
| 方針の保存前検査と照合 | `crates/cdna-app/src/policy.rs` | Engine policy commands |
| 進化契約 | `crates/cdna-evolution/src/lib.rs` | `crates/cdna-evolution/CONTRACT.md`、`EVOLUTION.md` |
| UI素材の出典と配布判断 | `apps/desktop/SOURCE.md` | `apps/desktop/SHADCN-LICENSE.md` |

静的経路は `App → api → static adapter → runtime → Worker → Rust domain / shared Python learner`。
ローカル経路は `App → api → loopback command → Engine → Store / inference / learner subprocess`。
MCP経路は `stdio → MCP dispatcher → app bridge → grant由来authority → Engine`。
初期対話経路は `bootstrap prompt → 外部AIのJSON → bootstrap Source → 共有許可 → teacher profile → 質問 → 本人回答 → confirm → train`。理由から意味表現への蒸留は未完成。

次の開発順序は `docs/release/KICKOFF.md`、固定ソースの検証は `docs/release/KICKOFF_VERIFICATION.md`。公開プレイグラウンドは保存済みの試作であり、追加開発を停止している。

native暗号化保管とブラウザ一時メモリは別の保存方式。静的体験にnative保管の
保証を転用しない。方針などstatic adapterの未対応操作は明示エラーになり、
成功を模擬しない。Tauri shellの配布・起動契約は同ディレクトリのREADMEを
参照する。この索引だけでnative packageや公開URLの受入済みとは扱わない。
