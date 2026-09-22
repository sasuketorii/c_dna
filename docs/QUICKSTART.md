# C-DNA を動かす

現在は実装・受入検証中です。本人再現の精度、Jevとの優劣、R1全体の完成を主張しません。個別要件の状態は [受入一覧](release/ACCEPTANCE.md) を参照してください。

## エンジンを起動する

開発はエンジンへ集中しています。現時点と次の方針は [kickoff](release/KICKOFF.md)。macOSでは次のCLIを使い、画面のbuildなしで学習基盤とMCPを起動できます。

```sh
uv sync --project learner --locked
cargo build --locked -p cdna-app
target/debug/cdna bootstrap prompt
target/debug/cdna init
target/debug/cdna serve
```

`bootstrap prompt`の全文を普段のChatGPT／Claudeへ貼り、JSON回答をファイルに保存します。`init`が返すworkspace IDを使って `bootstrap import --workspace WORKSPACE_UUID --preview profile.json` を確認し、`--preview`を外して保存します。本人による訂正とteacherへの共有は [bootstrap手順](architecture/BOOTSTRAP.md)、Codex／Claude Codeとの接続と質問・回答は [withAI手順](architecture/WITH_AI.md) を参照してください。取り込んだAIの人物理解は仮説であり、それだけで本人精度を検証済みにはしません。

`serve`は前面で動作します。別ターミナルからCLI操作を行い、終了はCtrl-Cです。既存の保管庫に再度`init`を行わないでください。

## 公開プレイグラウンド（追加開発停止）

[ブラウザ体験版を開く](https://cdna-playground.sasuketorii-business.workers.dev)。経営課題を選ぶと、学習済みの架空CEOモデルが判断・理由・判断が変わる条件を返します。条件を変更してすぐ再計算できます。通常の推論はブラウザ内のRust WASMで実行し、外部APIやPythonの初期化を待ちません。自由入力は対応する例題の検索です。

「自分の判断を学習させる」から学習スタジオへ進めます。スタジオは回答、本人確認、学習、候補モデルの試用をブラウザ内で行う一時セッションで、再読み込みすると記録が消えます。実際の機密情報は入力せず、架空事例で試してください。公開版のartifactと検証範囲は [配布記録](release/STATIC_DEPLOYMENT.md) を参照してください。

## ローカル開発

Rust は `rust-toolchain.toml`、Python は `learner/pyproject.toml`、JavaScript は `apps/desktop/package.json` と各lockを使用します。macOS の実保管庫はKeychainへ鍵を保存します。

```sh
uv sync --project learner --frozen
pnpm --dir apps/desktop install --frozen-lockfile
pnpm --dir apps/desktop build
cargo run -p cdna-app -- playground --demo
```

表示されたsession付きURLを開きます。`--demo` は使い捨ての暗号化保管庫です。学習は本人確認済み・モデル未閲覧の回答から行い、未検証候補モデルを正式適用しません。

実保管庫を作る場合：

```sh
cargo run -p cdna-app -- init
cargo run -p cdna-app -- playground
```

CLI入口 `exec` は標準入力からJSON操作を1件受け取ります。

```sh
printf '%s\n' '{"operation":"status"}' | cargo run -p cdna-app -- exec
```

## 会話エクスポートを取り込む

ChatGPTのmapping形式、Claudeのchat_messages形式のJSONを扱います。ZIPをそのまま渡せません。入力は最大16MiB、1ページ最大1,000件です。`--conversation`を繰り返して会話IDを選択でき、`next_cursor`が返ったら同じ入力に`--cursor`を指定して続行します。preview・再送・途中の扱いは [取り込み契約](architecture/IMPORT.md) を参照してください。

```sh
cargo run -p cdna-app -- import --provider chatgpt --workspace WORKSPACE_UUID conversations.json
```

原文は確認待ちのSourceとして保存されます。人・AIの発話を自動で本人の判断ラベルへ変換しません。対応形式はfixture検証済みで、全世代のexport形式への互換保証ではありません。

## 静的版をビルドする

```sh
python3 crates/cdna-browser/scripts/prepare.py
bash crates/cdna-browser/scripts/build-domain.sh
pnpm --dir apps/desktop build:static
```

配布対象は `apps/desktop/dist-static`。Rust WASMは入力検証、Pyodideは同じPython学習器を実行します。ランタイムは同一サイト配信です。詳細は [静的実行基盤](architecture/STATIC_RUNTIME.md) を参照してください。

## 検証

```sh
cargo test -p cdna-domain -p cdna-store -p cdna-inference -p cdna-mcp -p cdna-evolution -p cdna-app
uv run --project learner pytest
pnpm --dir apps/desktop typecheck
```

ネイティブshellは別途OSのTauriビルド依存が必要です。署名・公証・Python同梱は [shell手順](../apps/desktop/src-tauri/README.md) の状態を確認してください。合成データの評価結果は個人精度の証拠に使いません。
