# Harness learnings

- ID: LEARN-HARNESS-CDNA-003  日付: 2026-09-22  状態: promoted
  TUI確認画面ではtext-plus-Enterのprompt送信とキー操作を分ける。起動receiptがfailedになった後にUIを解決しても、過去のdispatchが成功へ戻るとは仮定しない。公式retryでtask実行のreceiptを取り直す。user_takeoverのペインは完了しても強制閉鎖しない。

- ID: LEARN-HARNESS-CDNA-001  日付: 2026-09-22  状態: validated
  観測: HARNESS_FINDINGS.md の HARNESS-CDNA-001。
  原因: 並行実装の成功と、固定された最終ソースへの検証成立を同一視できない。
  改善案: 並行中の試験は局所フィードバックに使い、最終receiptは編集停止後の同一ソースに対して取得する。sourceChangedを無視して成功報告へ変換しない。
  同乗先: 本台帳。グローバルハーネス変更は未実施。
