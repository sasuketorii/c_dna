# Project learnings

- ID: LEARN-CDNA-004  日付: 2026-09-22  状態: promoted
  観測: 「高速」を学習中のLLM利用まで抑える制約と解釈すると、理由を記録するだけの固定特徴学習へ縮退する。
  改善: 速度目標は学習済み判断と自然文からの全体応答へ置く。本人とLLMの対話、意味表現の改善、独立評価を中央の循環にする。まず一領域で本人の訂正が次の未知事例へ効くことを確かめ、第二領域で横展開を検証する。
  同乗先: `docs/architecture/TEACHING_LOOP.md`、要件§6.2／§7.5／§9.5／§10.4／§11.4／§16.4／§29.5。

- ID: LEARN-CDNA-001  日付: 2026-09-22  状態: promoted
  観測: Storeのモデル一覧はIDと値のtupleだった一方、UIはModel配列を想定していた。またstatic previewのreport展開から未検証pairwise_probabilityが流出し得た。
  原因: 内部表現を公開境界へそのまま渡し、利用側の型と出力意味を境界で確定していなかった。
  改善案: Engineとstatic adapterはModel配列に正規化。previewは出力allowlistを使い、全確率null・実行権限noneを固定し、候補の件数・同一性・有限スコアを検査する。
  同乗先: `crates/cdna-app/src/lib.rs`、`apps/desktop/src/static/adapter.ts`。`adapter.test.mjs` は確率漏出と重複候補拒否を含む状態回帰27 assertions合格。tuple修正はsource/typecheck確認であり、その実HTTP応答試験とは区別する。

- ID: LEARN-CDNA-002  日付: 2026-09-22  状態: promoted
  観測: worker初期化途中のdomain/Pythonをキャッシュすると、後続段階の失敗後に未完成runtimeが再利用される経路があった。
  原因: 初期化の完了前に共有参照を公開していた。
  改善案: 依存package・learner source・bridgeの準備がすべて成功した後だけキャッシュする。成功時の再利用は維持する。
  同乗先: `crates/cdna-browser/worker.ts`、`crates/cdna-browser/scripts/retry-test.mjs`。実worker moduleへ6段階の故障注入を行い、失敗→再初期化成功→成功状態再利用が合格。実ブラウザ試験ではない。

- ID: LEARN-CDNA-003  日付: 2026-09-22  状態: promoted
  観測: 学習中の改訂やロックで、開始時に妥当だったsnapshotが無効になる。
  原因: 学習開始時の確認済み状態だけでは、非同期結果の保存時点の有効性を保証できない。
  改善案: 変更ごとにepochを進めてモデルを失効し、結果保存時もepochを再照合する。lockはruntime resetとデータ破棄を行う。
  同乗先: `apps/desktop/src/static/adapter.ts`、`apps/desktop/src/static/adapter.test.mjs`。実Rust WASM検証と実adapter状態でpending→confirm→revise→delete、stale結果拒否、再送、lock/resetを検証。学習reportは競合制御用stubであり、精度受入には使わない。
