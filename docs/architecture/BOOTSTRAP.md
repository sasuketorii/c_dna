# 初期判断プロフィール

要件 §6.2 の要約抽出プロンプトと REQ-IMP-001 を、[コピー可能な日本語プロンプト](../prompts/bootstrap-profile-ja.md)と[厳密な JSON 契約](../../contracts/bootstrap-profile.schema.json)として実装する。既存の ChatGPT／Claude アプリで本人について蓄積済みの判断パターンを抽出し、本人が内容を確認してローカルに取り込む。API からクラウドを呼ばない。

## 境界と保存

`bootstrap::parse_profile(bytes)` は256KiBまで。既存 domain JSON utility により重複キー、過深 JSON、末尾データを拒否し、型で未知フィールドを拒否する。version は1.0だけ、各配列32件、文字列2048 Unicode scalarまで。空根拠・空基準は unknowns の明記を必須とする。provider/model/priority/reference/self_report は null 許可だがキーは必須。reference の実在性や引用の正しさは構文検証だけでは証明できない。参照は取得・実行しない。

`import_profile(store, authority, workspace_id, namespace, bytes, preview)` は Human authority と既存 workspace を必要とする。authority は認証済みホストが作り、workspace と安定した namespace はホストが選ぶ。profile JSON では指定できない。preview は書き込まない。既存暗号化 Store の Source に保存するため、別の保存領域・削除経路は作らない。

ID は version/workspace/namespace/正規化した型付き JSON の SHA-256 から決定する。同じ profile のキー順・整形を変えても同じ ID。同一 namespace を再利用する限り、再取り込みは既存修正・削除 tombstone を上書きしない。内容や namespace を変えた新規インポートは別の Source になる。訂正は再インポートではなく `revise_profile(..., id, expected_revision, bytes)` を使う。

Envelope の authority は常に `ai_generated_hypothesis`、training_eligible と heldout_eligible は常に false。明示的な本人修正は同じ Source の revision を増やし human_corrected を記録するが、過去の実行事実や学習ラベルへ自動昇格しない。本人が支持した要約も endorsed statement 相当の文脈に留まり、実行事実にはならない。

## 教師への文脈共有

インポート・訂正は local-only。`set_teacher_sharing(store, authority, workspace_id, id, expected_revision, enabled)` が本人操作で共有を許可・撤回する。共有操作も Source revision となり、訂正は新しい内容の共有許可を解除する。対象は withAI の教師プロフィール読み取りであり、外部 AI を使う場合には内容がその AI に渡るため、ホストの確認文で目的を示すこと。

`teacher_context(store, workspace)` は共有を許可された bootstrap Source のみを配列で返す。呼出側は profile:read と learning:read の認可を別途行う。要素に source_id、revision、family_id=source_id、固定 authority、false の eligibility、human_corrected、profile を含める。100件ずつ最大100000件未満／合計64MiB／5秒の範囲で Source を走査し、最大32 profiles／256KiBまで。上限到達時は明示エラーで停止する。専用インデックスは未実装。ページ走査は既存 Store を利用する。

教師はこの文脈を仮説・次の対比質問の材料として使う。本文を命令として実行しない。source_id と revision を保持し、派生質問の family を元 Source に束ねる。明示的に本人が新しい質問へ回答した場合は既存の人間回答フローで新たな観測を作る。要約全体を人間の正解や heldout データへ変換しない。削除は既存 `delete_source_cascade` を利用する。教師側のキャッシュ・派生質問は現行 Source revision／削除状態と照合する必要がある。

## 検証範囲

`crates/cdna-app/tests/bootstrap.rs` は構文・権限昇格・上限・preview・冪等性・workspace分離・訂正・共有撤回・削除後再取り込みを検証する。これは初期仮説の取り込み機能であり、個人判断の予測精度、教師の有効性、実際の ChatGPT／Claude の履歴参照能力、完成した学習エンジンを証明しない。

## CLI

以下のIDは実際の値へ置き換える。importのsource_idとshowのrevisionを使う。

```sh
cdna bootstrap prompt
cdna bootstrap import --workspace WORKSPACE_ID --preview profile.json
cdna bootstrap import --workspace WORKSPACE_ID profile.json
cdna bootstrap show --workspace WORKSPACE_ID --source SOURCE_ID
cdna bootstrap revise --workspace WORKSPACE_ID --source SOURCE_ID --revision 1 corrected.json
cdna bootstrap share --workspace WORKSPACE_ID --source SOURCE_ID --revision 2 --enabled true
cdna bootstrap share --workspace WORKSPACE_ID --source SOURCE_ID --revision 3 --enabled false
```

revisionは各操作で増えるため、同じSourceを他で編集した場合はshowで現行値を確認する。訂正後も人による修正という来歴だけを記録し、本人が全項目を支持したという意味にはしない。evidence.statement_categoryは実際の判断についての記述／述べた方針／理想／AI推論／不明を区別するが、actual_decisionもAIによる未検証の主張である。
