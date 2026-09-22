# Project findings

- ID: PROJECT-CDNA-012  日付: 2026-09-22  状態: documented / partially repaired
  観測: 要件§6.2に既存ChatGPT／Claudeへ渡す初期化プロンプトがあるのに、実装は空の質問体験から始めていた。READMEはLLMを周辺処理へ寄せ、理由・逆転条件を保存しても17特徴の学習器は意味を使わなかった。
  原因: 取り込み・意味理解・学習・高速推論を個別に実装し、本人が理解されるまでの一周を受入単位にしなかった。
  改善: 専用JSONプロンプトとSource取込、共有、teacher質問、本人回答を接続。Fable 5.1とAstraの直接討論で中央のteacher loop、版付き表現、anchor付き蒸留を設計へ反映。
  残ること: 意味表現の学習、教師開示の永続台帳、質問batchのrebase、実LLMとの自然文往復と本人の独立評価。`docs/release/KICKOFF.md`を次の実装順序とする。

- ID: PROJECT-CDNA-013  日付: 2026-09-22  状態: open
  観測: withAIは現在のモデルのTRAIN familyだけを開示するが、その開示履歴を将来の再分割へ固定する永続台帳がない。同時刻・少数familyの暫定学習後に再分割すると、既に教師が見た例を評価へ戻し得る。
  改善: disclosureをsplit/revision/epochへ束縛し、開示済みfamilyと派生物を以後TRAIN専用にする。削除時は本文を保持せず失効状態を整合させる。修正前は教師経由データを独立評価済みとして本番昇格しない。

## PROJECT-CDNA-001 — 要件と実装の隔たり

- 状態: confirmed
- 根拠: 初期コミット `61b48f9` はREADMEと要件書のみで、実行コード・依存lock・試験なし。
- 影響: 設計記述を実装済みとして扱えない。R1完成には270件のP0と受入試験の確認が必要。
- 改善: `docs/release/requirements-status.json` に証拠を紐付け、未検証と実装済みを区別する。

## PROJECT-CDNA-002 — 初期化・公開応答の境界

- 状態: fixed; scoped regression passed
- 観測: static workerの途中初期化失敗後に未完成runtimeを再利用でき、previewの内部report展開から未検証pairwise_probabilityが漏れ得た。
- 修正: 完全初期化後のみ共有参照を公開。previewは出力allowlist、全確率null、候補同一性検査。
- 証拠: `crates/cdna-browser/scripts/retry-test.mjs`、`apps/desktop/src/static/adapter.test.mjs`。故障注入／状態回帰と実精度評価は別扱い。

## PROJECT-CDNA-003 — 改訂時の来歴・予測閲覧履歴

- 状態: fixed; independent regression in progress
- 観測: UIで予測を閲覧後に元payloadから訂正するとexposureが消え、API改訂ではsourceの差し替えも可能だった。
- 修正: UIはworkspace/family/record別のセッション閲覧履歴を保存時に反映。APIは旧exposureをORし、familyとsourceを維持する。
- 証拠: `apps/desktop/tests/exposure.test.ts`、`crates/cdna-app/tests/adversarial.rs`。原回答の遡及変更は行わない。
- 改善: 来歴と学習適格性を全クライアントの共通保存境界で検査する。

## PROJECT-CDNA-004 — importの重複JSONキー

- 状態: fixed; independent regression in progress
- 観測: 同じmessageのsenderをassistant/humanで重複させると通常JSON変換が最後の値を採用し、発話者が曖昧になる。
- 修正: `parse_json_with_limit` で既存strict parserを共有し、importの16MiB境界でも全階層の重複キーを拒否する。
- 証拠: `importer_does_not_silently_choose_between_conflicting_speakers`。
- 改善: untrusted JSONはValueへの変換前に重複を検査する。

## PROJECT-CDNA-005 — 保存可能でも読み戻せない深いJSON

- 状態: fix under independent verification
- 観測: 150階層の小さいValueを保存でき、通常serde readerで読めなくなる経路を異常系試験が再現。
- 改善: 全永続化境界で読戻し可能性をtransaction前に検証し、不正入力ではepochを進めない。
- 証拠: `crates/cdna-app/tests/adversarial.rs` のStore write/readback回帰。

## PROJECT-CDNA-006 — 公開プレイグラウンドの目的不一致

- 状態: implementation published; further work stopped by user, latest hosted acceptance incomplete
- 観測: 学習・確認・管理UIを公開入口にしたため、利用者がCEO-DNAの即時判断を体験できなかった。ユーザーの明示要件は「経営課題を選択または入力→判断回答」。
- 修正: 公開defaultは学習済み合成CEOモデルの課題選択・判断・理由・条件変更にする。学習スタジオは別導線。入力文は対応例題検索と明記する。
- 改善: 機能完成と体験目的の達成を別々に検証する。公開受入は初見の問題選択から回答表示まで実測する。

## PROJECT-CDNA-007 — WASM再生成だけではsource更新にならない

- 状態: fixed; actual scoring smoke passed
- 観測: build-domain.shが既存wasmをwasm-bindgen処理するだけで、Rust変更後も古い実装を再配布できた。
- 修正: スクリプト内で対象crateのwasm releaseをビルドしてからbindingを生成する。
- 証拠: `crates/cdna-browser/scripts/scoring-smoke.mjs` は実WASMのスコア・寄与和・重複JSON拒否を検証する。
- 改善: ビルド入口1個でsourceから配布物まで閉じ、生成だけの手順を「build」と呼ばない。

## PROJECT-CDNA-008 — 取り込み元削除の派生回答残存

- 状態: fixed; Store17件と対象アプリ回帰が合格
- 観測: Sourceの削除だけでは、その出典に由来する確認済み回答が学習対象に残った。
- 修正: 過去revisionの出典も対象に原子的に削除し、tombstone・検索index・モデル・改善状態を失効する。復元と再取り込みにも適用。
- 証拠: `crates/cdna-store/src/source_deletion.rs`、`crates/cdna-app/tests/onboarding_workflow.rs`。

## PROJECT-CDNA-009 — 学習対象外の件数による学習停止

- 状態: fixed; scoped tests passed, combined verification pending
- 観測: 最初の1,000件を無条件に読み、確認済み・予測未閲覧の判定前に件数超過で停止していた。
- 修正: DBで最新の確認済みrevisionを抽出し、keyset方式で分割取得。適格回答だけを数え、epoch・時間・byte上限を照合する。2,000適格回答／8MiBの上限は明示エラーとして残る。
- 証拠: `crates/cdna-app/src/training_snapshot.rs`。大量pendingに有効回答を追加し、訂正によるpending化・再確認・別workspaceを検証。

## PROJECT-CDNA-010 — 学習・評価の境界不整合

- 状態: fixes undergoing independent verification
- 観測: 同時刻の20件以上が全てtest分割に入り、学習0件のゼロ重みを成功として返した。署名済み評価でも、同じfamilyの反復や1件だけの領域を独立した精度の根拠にできた。
- 修正方針: 時系列分割できなければ未評価の仮モデルとして学習し、昇格には独立familyと各領域の必要件数・品質を要求する。
- 証拠: `benchmarks/engine/learner_probes.py`、`benchmarks/engine/src/bin/signed.rs`、独立レビュー報告。

## PROJECT-CDNA-011 — 自由文理解と学習支援の未接続

- 状態: open; integration in progress
- 観測: 即時採点は構造化済みの数値候補を前提とする。LLMによる自由文抽出とwithAIの回答分析・質問生成は別工程で、存在するだけでは入力から回答まで完成しない。
- 改善: 生成内容と本人の発言・回答を分離し、自然言語の意味精度、学習支援による独立評価の改善、全工程の待ち時間を別々に検証する。
