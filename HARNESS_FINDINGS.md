# Harness findings

- ID: HARNESS-CDNA-003  日付: 2026-09-22  状態: recovered
  観測: Fable 5.1のOrca worker起動がworkspace trust画面で停止。最初の矢印＋Enterをtext promptとしてまとめた操作では選択が変わらず、Noを確定して終了した。
  改善: `terminal read --screen`で状態を確認し、raw text-onlyで矢印→選択のreadback→bare Enterと分ける。failed dispatchは公式release/retry-ofで回収し、実際のFable 5.1表示とturn_startedを確認してから討論開始と報告した。
  結果: `docs/architecture/reviews/FABLE_TEACHING_REVIEW.md`とAstra reviewに直接討論・反論・統合判断を記録。起動要求やinput_acceptedだけではモデル稼働の証拠にしない。

- ID: HARNESS-CDNA-001  日付: 2026-09-22  状態: reproduced
  観測: 親が取得した `cdna-store-batch.json` は子コマンド exitCode=0、sourceChanged=true。作業ツリーdigestは5456cae8からcf87fd35へ変化した。親の実行記録ではguard終了値50。
  原因: receiptが共有作業ツリー全体を束縛するため、試験中の別担当の編集もソース変化になる。これはcdna-store試験失敗の証拠ではなく、最終ソースへの合格証拠が成立しない状態。
  改善案: 最終検証時に編集を収束するか隔離checkoutで実施し、子コマンド終了値とsourceChangedを別々に確認する。guard自体の欠陥とは判定しない。
  同乗先: HARNESS_LEARNINGS.md の LEARN-HARNESS-CDNA-001。receiptはセッション一時証拠でありリポジトリ同梱物ではない。

- ID: HARNESS-CDNA-002  日付: 2026-09-22  状態: observed
  観測: 完了済みOrcaペインのfollow-up worker-startでagent_readiness timeoutが2件発生。receiptにはfailed、入力未送信、residualResourcesなし。新規ペインの同task retry-ofでready/input_accepted/turn_startedを確認した。
  改善案: 起動要求を成功とみなさず、非同期起動receiptを必ず回収する。失敗時はtask/dispatchを維持した公式retryを使い、二重editorを避ける。root causeがペイン再利用全般にあるとは未断定。
  同乗先: Orca orchestration recovery guide。実装変更なし。
