# 受け入れ状況

この記録は開発途中のスナップショットです。**R1全要件の完了判定は未成立**です。静的プレイグラウンドの動作、native保管庫の局所試験、本人の再現精度、署名済み配布物の実機受入を別々に扱います。

**後続更新:** 以下の個別監査は08:57 UTCの履歴です。後続の署名済み推論、bootstrap、withAI、学習改善を含む現在のチェックポイントは [KICKOFF.md](KICKOFF.md) と [統合検証](KICKOFF_VERIFICATION.md) に記載します。以下の件数を現在の実装完成数として転用しません。

基準: [要件書](../../exec_plan/C-DNA_Requirements_v1.0.0_2026-09-21.md)。全271 ID（P0 270件、P1 1件）を [requirements-status.json](requirements-status.json) に保持しました。正本のID集合との完全一致を検査しています。

## 記録時点と読める範囲

- 記録日時: `2026-09-22T08:57:30.543151+00:00`。基準commit: `61b48f9895114fc156381f70ae9eaab0f06252b9`。
- 未コミット・並行実装中。JSON内のsource hashesはこの記録時点のファイルを示し、以前のreceiptと同一の最終リリースSHAを保証しません。
- `local_verified` は記載receipt時点の局所実行証拠、`source_implemented` は具体的なsource確認、`source_partial` は一部実装です。`not_implemented`、`external_unverified`、`not_assessed`を区別します。
- 各要件の `acceptance.result=blocked` は「当該要件全体の最終受入が未閉鎖」を意味します。全機能が存在しない、全試験が失敗した、という意味ではありません。未実施をpassに変換していません。
- source/testファイルの存在だけを実行成功にしません。現時点の集計は以下です。

| 状態 | 件数 | 証拠の意味 |
|---|---:|---|
| `local_verified` | 25 | 記録された局所実行あり。本人精度や製品全経路の合格ではない。 |
| `source_implemented` | 21 | sourceで該当実装を確認。最新統合受入は別。 |
| `source_partial` | 185 | 関連実装はあるが要件の一部または証拠が残る。 |
| `not_implemented` | 25 | 必要な製品経路が未完成。未接続表示は代替合格にしない。 |
| `external_unverified` | 4 | 実機・配布署名・外部条件が未確認。 |
| `not_assessed` | 11 | 要件全体としての監査をこの限定パスで完了していない。 |

## 手元にある実行証拠

| 証拠 | 結果 | 適用範囲と限界 |
|---|---|---|
| domain ordered-policy receipt | exit 0、10 tests | 型/サイズ/重複/非有限値/推測事実/空contextの不正比較。Rust coreの局所試験。 |
| evolution final2 receipt | exit 0、5 tests | 合成データ、署名/内容hash/epoch/承認/rollback。実評価者、本人精度、store統合は含まない。 |
| learner ratio-final receipt | exit 0、報告22 tests | native Pythonの当該snapshot。後から追加された校正コードの合格に転用しない。 |
| browser smoke receipt | exit 0 | Node上の実Pyodide WASM学習と反対回答での順位変化、Rust WASM契約。GUI操作や本番配信の証拠ではない。 |
| UI static-build receipt | build結果をJSONに記録 | typecheck/bundle。表示・IME・読み上げ・本番動作の証拠ではない。 |
| 親担当からの17:53 JSTブラウザ報告 | staticで保存→確認→学習 | 1 source、model `d1001156`。本担当は再実行していない。学習前resource名に外部hostがないという観測は、全依存の無送信証明ではない。 |

receiptのSHA256、時刻、argv、guard取得、sourceChanged、stdout hashはJSONへ転記しました。receipt自体はセッション内の一時成果物で、リポジトリに保存したCI証跡ではありません。最終受入時は正本artifactとして保管し、実際のリリースSHAに結び直してください。

## P0の未閉鎖事項と実行順

以下は対策の優先順です。要件のP0優先度を下げる意味ではありません。

1. **データと権限の境界を閉じる。** native/static両方の確認・訂正・削除・exposure、MCPの実grant取消し/返却直前再認可、lockと学習競合を統合試験する。最新の修正を含むhash固定runが必要です。`DAT`、`MCP`、`API`、`SEC`、`DEL`、`DB`。
2. **取り込み→抽出→本人確認を製品経路としてつなぐ。** parser/暗号化保存が存在しても出典、当時候補、話者、extractor版、重複、再開、キャンセルまでの仕様は別です。実export fixture、破損/ZIP異常/再取り込み/本人訂正保全を検証する。`IMP`、`SEC-006/007`。
3. **評価と昇格を実データ・永続状態に接続する。** evolutionの局所coreを実験承認、信頼評価者、最終test露出台帳、署名鍵管理、encrypted-storeのatomic active pointerへ接続する。全モデルの適用domain/epoch/readbackを確認する。未測定の本人一致率・Jev優越は表示しない。`EVO`、`EVL`、`ML`、`UNC`。
4. **質問・プロフィール・任意性格結果の必須範囲を実装する。** 能動学習のfamily/seed/選定来歴、確認用問い、プロフィールの根拠と差分、性格結果の手動import/由来/独立権限/削除は未閉鎖。質問票の権利未確認を他経路の未実装理由にしない。`AL`、`MEM`、`PER`、`CON-006/007`。
5. **復旧・配布・操作受入を実機で確認する。** 暗号化backup/restoreの局所実装に加え、電源断/空き容量/壊れたbackup/旧backupと新tombstone、独立復旧、最古/現行OSを試す。署名・公証・runtime同梱・更新rollback・license/SBOMが必要。`BAK`、`ENV`、`REL`、`DEP`、`NFR`。
6. **外部機能の未接続を正確に維持する。** provider/agent session、cloud同意、診断API/期限/receipt/改善packageは未完。公式アカウント・許諾・配信先確認が必要な箇所だけを外部未検証とし、内部の未実装とは区別する。`AGT`、`PRV`。
7. **日本語とアクセシビリティを実測する。** 960×640、拡大、両テーマ、IME/長押し/dialog/input競合、keyboard-only、screen reader、reduced motion、失敗時の備考/未保存復旧を確認する。`UI`、`GAME`、`NFR-010/011/012`。

## 明確に未完成・外部未検証としたID

### 必要な製品経路が未完成

- `REQ-AGT-002`, `REQ-AGT-006`, `REQ-AGT-009`
- `REQ-AL-004`, `REQ-AL-005`, `REQ-AL-006`, `REQ-AL-008`, `REQ-AL-009`
- `REQ-CON-006`, `REQ-CON-007`
- `REQ-DEP-003`, `REQ-DEP-009`
- `REQ-ENV-002`, `REQ-ENV-003`
- `REQ-PER-003`, `REQ-PER-006`, `REQ-PER-009`, `REQ-PER-010`
- `REQ-PRV-003`, `REQ-PRV-008`, `REQ-PRV-009`, `REQ-PRV-010`
- `REQ-REL-002`, `REQ-REL-003`, `REQ-REL-004`

### 外部・実機条件が未確認

- `REQ-ARC-010`
- `REQ-DEP-002`
- `REQ-ENV-001`
- `REQ-REL-001`

この一覧以外も、`source_partial`、sourceのみ、局所試験のみの場合は全体受入が未閉鎖です。全270 P0の個別remaining/source/evidenceはJSONにあります。ここに書かれた件数を「残る実装タスク数」へ読み替えないでください。

## 最終判定の更新条件

現在の総合結果は **blocked**。R1完成と表明しません。既知の局所バグ修正と本番受入を混同しません。

完了判定を更新する担当は、要件ごとに全条項へ対応するsourceと否定ケース、実行結果、対象SHA、必要な実機/外部条件を確認します。失敗はfail、外部条件不足/未実施はblocked、非該当は具体的理由と承認されたscopeを添えます。ソフトウェアfixtureの成功を本人データ精度へ置き換えません。
