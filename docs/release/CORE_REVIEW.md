# Core boundary independent review

2026-09-22。対象は実 `Engine`、SQLCipher `Store`、export importer。所有変更は `crates/cdna-app/tests/adversarial.rs` と本書のみ。製品全体・配布物・実本人精度の合格判定ではない。

## Follow-up: 2026-09-22

親担当の shared `parse_json_with_limit` と revision source 保持の修正後、既存 **14 adversarial tests は全成功**した。`cdna-core-adversarial-followup.json` は cargo exit 0 / sourceChanged=true / guard外側 exit 50。局所テスト成功を確認したが、共有worktree全体の固定検証とはしない。以下のCORE-01/02/03は、このfollow-upのテスト範囲では解消した。

新規 `onboarding_workflow.rs` の初回4件は全成功（`cdna-onboarding-followup.json`、cargo/guard exit 0）。preview無変更、Source-only保存、取得・削除・再import、Agentの9操作拒否、別workspaceの取得拒否、assessment同意・本人確認・共有禁止・export/学習非混入、質問数/プール数/workspace/既出family/決定性を実Engine経由で確認した。

### CORE-04: Source削除後も派生観測が学習に残る（P1、再現済み）

`import_sources(save) → human source_idをartifact_idとするimported観測をpropose/confirm → source_delete → prepare_train` で削除後も学習入力が得られた。`deleting_imported_source_cannot_leave_its_confirmed_label_trainable` が失敗した。Store契約もsource削除を公開する前のhost lineage cascadeを要求している。現在epochが進んで旧モデルが無効化されるだけでは、次の再学習が削除対象の派生内容を再採用することを防げない。最小修正は依存観測をatomicに無効化するか、依存が残るSourceの削除を拒否すること。

`cdna-onboarding-lineage.json` は5件中4成功/1失敗、cargo exit 101、sourceChanged=true。失敗とsource driftを別々に記録した。この追加テストでは非空のtraining inputにassessmentを追加してもinputが完全一致することも確認した。CORE-04修正と再検証が必要であり、現時点のfollow-up判定は未完了。

## Follow-up receipt binding

| Receipt | Test exit | sourceChanged | stdout SHA256 |
|---|---:|---|---|
| `cdna-core-adversarial-followup.json` | 0 | true | `20968c51092ee0445567d46d4464ccea67b83a8acb5594a2eca0c6671d7979a5` |
| `cdna-onboarding-followup.json` | 0 | false | `f4c3a3696bd0885581c1b0f2121ba3bf0bfce3651b710e1c4fd952e2b9d8a1f9` |
| `cdna-onboarding-lineage.json` | 101 | true | `6e1cee28722a3f824e249c3c599dd3b81b70995909888cecedcc18af81aefd06` |

Engine benchmarkは同一形状の旧1/100/1000 matching fixtureと比較し、新1001 matching/1000 total・1 matchingも追加した。[測定と限界](../../benchmarks/core/RESULTS.md)を参照。14シナリオのassertionは完了したが、build/run中のsource変更によりrunnerはexit2で無効化扱いとした。固定binaryの観測と最終sourceの性能合格を区別する。

## 初回レビュー時の判定（履歴）

第4回の局所検証は **14件中12件成功、2件失敗**。未解消の出典境界があるため、この試験範囲もまだ GO ではない。基準 HEAD は `61b48f9895114fc156381f70ae9eaab0f06252b9`、未コミット・並行更新中の worktree を使用した。

### CORE-01: importer が重複 speaker を黙って選ぶ（P1、再現済み）

- 入口: `importer::parse_export(Provider::Claude, bytes, &[])`。
- 入力: `[{"uuid":"c","chat_messages":[{"uuid":"m","sender":"assistant","sender":"human","text":"AI answer"}]}]`。
- 実結果: `complete=true`、`Speaker::Human`、`speaker_raw="human"`。同じメッセージの矛盾する発話者を、後勝ちで本人発言へ変換する。
- 影響: REQ-IMP-002/003 の発話者保全を破る。`review_pending=true` と `training_eligible=false` は維持され、即時の学習・権限獲得までは再現していない。
- テスト: `importer_does_not_silently_choose_between_conflicting_speakers`。
- 最小修正: Value 化で重複キーが失われる前に拒否する。既存 domain の重複検出 parser を import の byte 上限付きで再利用し、独自の二重 parser を増やさない。

### CORE-02: 通常 revision が出典 identity を置換する（P1相当の出典契約要確認、再現済み）

- 入口: Human の `propose → confirm → revise`。
- 入力: imported/artifact A の観測を、同じ record/family の human_app/artifact B へ訂正する。
- 実結果: 新 head が artifact B と human_app を返し、元の出典 identity を示すリンクを payload 内に持たない。
- 影響: 現行 head の利用者に誤った由来を提示できる。Store の旧 revision は保持されるため、全履歴の物理消失とは主張しない。本人による明示的な出典訂正を許す仕様なら、その専用操作と来歴表示で修正する選択肢もある。
- テスト: `revision_retains_original_source_identity`。
- 最小修正: 通常 revise では source identity を保持するか、明示的な出典訂正と元 source lineage を記録する。

### CORE-03: Store が自分で読み戻せない JSON を commit（再現後、修正確認）

- 入力: null を150個の配列で包む304-byte Value。巨大入力ではない。
- 実結果（第2回）: `insert_documents_batch` が commit した後、`get_document` は serde の depth 制限で失敗する。list も同じ decoder を使用する。
- 影響: 内部永続化契約の破綻と対象一覧の読出し障害。raw importer は深い JSON を拒否するため、未認証の外部入力から到達可能とは主張しない。
- 担当者修正後（第4回）: batch / put_document / propose / save_model の4経路で「拒否して未変更、または成功して読み戻し可能」を確認。
- テスト: `store_never_commits_json_it_cannot_read_back` と `*_write_never_commits_unreadable_json`。

## 成功した境界

- AI-generated を human_app に revision しても学習 snapshot に入らない。
- model_exposure=true を false に revision しても学習 snapshot に入らない。
- family ID の revision 差し替えを拒否する。
- 削除前 epoch の学習 completion / model 保存 / activation を拒否する。
- Agent の workspace 越境 read/propose/rank、別 workspace の record ID 操作を拒否する。
- request byte上限・型不正・負のoffset・未知のenvelope fieldを拒否し、epochを変更しない。Status/Lock の unit variant 問題は担当者修正後に回帰確認した。
- importer の深いJSONを拒否し、過大メッセージを隔離する。
- importer の自己申告 trust flags を採用しない。
- backup の後で消した observation / source document が、両 tombstone 集合を渡した restore と再importで復活しない。

モデル露出のある観測の recall evidence に exposure が含まれないことは source で確認したが、独立精度の評価に使用される consumer まで実証していないため、追加 P1 として数えていない。

## 実行証拠

コマンド: `cargo test -p cdna-app --test adversarial -- --test-threads=1`。各回とも `revh command run --guard required --class heavy` を使用。receipt と隣接 stdout/stderr log はセッションの一時証拠で、リポジトリに永続保存したCI証拠ではない。以下は receipt basename。guard の外側 exit 50 と cargo exit 101 を混同しない。

| Receipt | Cargo exit | Test result | sourceChanged | 意味 |
|---|---:|---|---|---|
| `cdna-core-adversarial-first.json` | 101 | 6成功/1失敗 | false | Status の未知field受入を検出 |
| `cdna-core-adversarial-second.json` | 101 | 8成功/1失敗 | true | Store深度を実再現、同時にsource更新あり |
| `cdna-core-adversarial-third.json` | 101 | 未実行 | true | 並行Store編集の `matching.rs` 未作成時にcompile失敗 |
| `cdna-core-adversarial-fourth.json` | 101 | 12成功/2失敗 | false | CORE-01/02再現、Store修正を回帰確認 |

第4回: `2026-09-22T09:08:19.769Z`〜`09:08:26.912Z`。worktree digest `sha256:c4ddabebf39c5633f9b880ed7f09a37b16946f378ef579aeef2b1b46fbc2e962`。stdout SHA256 `ada033780c95ccd7c32cb19351b2fbb39e2cd69bb01a81311de4bfb167a2f9b8`。sourceChanged=false は当該runに限り、後続編集や本書を含む最終成果物の固定証明ではない。

## 構造確認と限界

rev_skills の codebase-graph で同じ repository を index し、Engine/Store/importer の入口を search_graph で検索した。prepare_train inbound depth=1 は `main::command` と `Engine::train` を返し、実sourceの呼出しと照合した。trace は exit 4、`language_not_evaluated` と `edge_evidence_unverified`、complete=false/freshness=unverified を返したため、Rust の完全な呼出し証明として採用していない。index の parse_partial は CSS 1ファイルだった。

この検証はlocal/synthetic native統合であり、CI、署名済み配布、macOS実機UI、電源断復旧、本人の再現精度、全workspace操作、全import形式の受入ではない。activation は現行実装が fail closed であることを確認しただけで、正式モデルの昇格成功を証明していない。
