# kickoff検証

2026-09-22。`kickoff`はソースと開発方針の保存であり、R1の完成版ではない。

## 対象と検証範囲

macOS上で、Rustのdomain/store/inference/MCP/evolution/browser/appをまとめて検査する。Pythonの全learner testとRuff、desktopのTypeScriptを検査する。bootstrapからteacher質問・本人回答・確認・学習までのSDK／private bridge統合fixtureを含む。実ChatGPT／Claudeアプリの人物理解や実LLM対話品質は、このfixtureの対象ではない。

Linux CIは`.woodpecker/check.yaml`に固定し、移植可能なRustコアとPython learnerを検査する。macOS Keychainとnative appはLinux CIに含まない。CIはpushイベント・対象main・対象commitの成功を確認してからoriginへ送る。CIによるデプロイは設定しない。

## 公開物の点検

保管庫・秘密鍵・環境変数ファイル・モデル実体・Python配布物・node_modules・ビルド成果物は追跡しない。Gitleaksで公開差分を検査する。既存receiptの`lockKey`はcanonical rootのSHA256であり認証秘密ではないことをharness実装で確認した。tokenizerのSHA256と合わせ、フィールド名と64桁hexが一致するものだけを`.gitleaks.toml`で除外する。その他の秘密検出規則は維持する。

upstreamライセンスは元のCRLFを保持し、その一ファイルだけを`.gitattributes`の`cr-at-eol`対象にする。Markdownのローカル絶対パス、未解決リンク、要件ID集合、差分の空白を点検する。

## 実行結果

最終run `cdna-kickoff-frozen.json` はrequired heavy guardを取得し、終了値0・`sourceChanged=false`。Rust150 tests、Python46 tests、Ruff、desktop TypeScriptの全てが成功した。CLIの`bootstrap prompt`は正本プロンプトとbyte一致し、prompt/import/show/revise/shareの公開コマンドを確認した。

Gitleaksの最終差分scanは検出0。Markdownのリンク先欠落とローカル絶対パスは0、要件書と監査JSONの271 ID集合は完全一致、staged diffの空白検査は成功した。5MiB超の新規ファイルはない。

前段の実行ではRustとPythonは成功したが、最後のTypeScript呼出でPATH不足、途中のstagingによりguardがsourceChangedを検出した。PATHを修正し、stageを固定して上記を再実行した。前段runを最終合格には用いていない。

この文書の検証結果追記は試験完了後に行った。実装・test・lockの内容は最終run後に変更していない。CIの実行番号とcommitの対応はWoodpeckerの対象pushイベントを正本とする。

本人の独立評価、Jev比較、実クライアントの接続、公証・署名済みアプリ、復旧実機試験は未実施または別の受入が必要。過去の性能値は [ENGINE_REVIEW.md](ENGINE_REVIEW.md) のsource bindingに限定される。
