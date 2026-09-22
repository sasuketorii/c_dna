# 自然言語入力の抽出境界

自由文を型付き候補へ変換する境界は `crates/cdna-app/src/natural_language.rs`。
これは本人モデルの代わりではなく、入力を確認可能な提案へ変換する部品である。
キーワードから固定シナリオを選ぶ処理、架空のLLM応答による意味精度の合格判定、
未確認の生成事実を `explicit` にする処理は実装しない。

## API と責務

1. ホストが認証・保管庫状態・workspace・操作権限を確認し、`TrustedHost` を構築する。
   workspace/request ID、時刻、mode、domain、抽出/ランキング権限はホスト専用。
2. ホストが許可モデル、送信先・本文・workspace・目的に限定した同意、単一呼び出しの
   予算予約、期限を `ExtractionPolicy` に設定する。予約は共有予算から原子的に確保する。
3. `extract(adapter, text, host, policy, cancellation)` は `preflight` 後に一度だけ呼ぶ。
   `ExtractionAdapter::extract(ExtractionCall)` が `AdapterOutput` を返す。
4. `accept_output` は生バイトを既存の `cdna_domain::parse_json` で検査し、
   `ExtractionProposal` を deserialize して既存 `RankRequest::validate` に渡す。
5. 結果は `ExtractionResult`。生成文脈は `inferred`、null は `unknown`。
   `requires_confirmation=true`、`semantic_accuracy_verified=false` を必ず保持する。
   ホストは確認工程または不足情報の表示へ送り、抽出だけで本人の結論と表示しない。

`preflight` / `accept_output` は transport なしで検証できる。
テストは実在するこの境界を実行し、LLMが日本語を正しく理解した証拠にはしない。
`EXTRACTION_SYSTEM_PROMPT` と `extraction_json_schema()` が transport と評価の共通契約。
JSON Schema は生成制約のヒントであり、Rustの実行時検証が正本である。

provider が出せるのは `facts`, `unknown_fields`, `candidates` だけ。
各factは `key,value,unit,source_span`、各candidateは既存の `id,text,attributes`。
数値featureの候補属性は既存6種の [-1,1] またはnullに限定する。
本文は独立したuntrusted user messageとして送る。ツール利用は0、再試行は0。
入力内の命令は権限を変更できず、生成されたscore、承認、source label、時刻、workspaceは拒否する。
候補の並び順や自然言語の指示を本人モデルの順位に転用しない。

## 出典と欠損

原文を改変せず `context.summary` に保持する。source span はUTF-8のbyte rangeであり、
境界・長さを検査するが、引用位置の一致は解釈の正しさの証明ではない。
すべての解釈を未検証と表示し、候補属性も含め生成提案として扱う。
否定・条件・通貨・期間を正しく解釈したかは別の実モデル評価が必要。
null factはunknown一覧へ追加し、既知factとunknown一覧の矛盾を拒否する。
モデルが未報告の欠損を網羅的に検知できる保証はない。

現在の `cdna-inference::extract_features` は `explicit` な文脈factだけを相互作用に使う。
したがって今回の `inferred` factから文脈依存の個人化が成立したとは説明できない。
数値がない自由文を勝手なratioに置き換えないため、属性が空なら欠損として残る。
確認済みの数値への昇格、一般的な自由文からの有用なfeature抽出、本人向け即時結論は
別途の設計・評価が必要。既存スコアラーの意味をこの境界が変更することはない。

## 上限・キャンセル

入力16 KiB、生成JSON64 KiB以下、出力tokenは最大8192かつhost設定値以下。
既存domainのcontext/candidate/fact上限も適用する。文字列は切り詰めず拒否する。
重複キーは全階層で拒否し、不正UTF-8、末尾JSON、NaN、未知キーも拒否する。
金額のminor unitとISO通貨、数値unit、正規化featureのratioを既存domainで検査する。
一般数値の単位の意味や、円/銭の誤変換まで自動で正しいと証明するものではない。

Tokio deadlineまたはwatch cancellation（sender消失を含む）でfutureをdropする。
transport実装はdropでI/Oを中止し、detachした生成を残さず、読み取り中からbyte上限を守る。
この境界は悪意ある任意のRust adapterのCPU時間・内部確保を隔離しない。
ホスト側で同時実行数を制限し、モデルプロセスのCPU/メモリ/コンテキスト枠を制御する。
usage不明は `None` のまま保持し、0料金とはしない。報告値が予算を超えれば拒否するが、
事後拒否で課金を取り消せないため、transportが事前予約・token制限を守る必要がある。
抽出のcloud同意を後段のcloud rankingへ転用せず `RankRequest.allow_cloud=false` とする。

## ローカルtransportの提案と実測範囲

2026-09-22の初期probeで `llama-server` は0.4.0/build10809/commit5266f24da、
Darwin arm64版がPATH上に存在した。ollama / mlx_lmはPATH上になく、loopback 8080のhealthは
connection refusedだった。一般的なモデルcacheディレクトリも確認できなかった。
これは端末全体にモデルがないという証明ではなく、この時点で生成可能性は未確認だった。
本担当は大きなモデルを導入していない。別担当の実モデル測定は
`benchmarks/natural-language` 側の現行証跡を参照し、この境界テストと混同しない。

推奨する最小接続は既存llama.cppをloopbackだけで起動し、ホスト固定モデルへの
`POST /v1/chat/completions` を1回実行するnative HTTP adapter。
JSON Schemaによる `response_format`、非stream、制限した生成token、独立system/user messageを使う。
接続先はユーザー本文から受け取らず、redirect/proxy経由の外部送信を防ぎ、認証情報を渡さない。
クライアントはdeadlineと読み取り上限を持ち、cancelで接続を解放する。
モデル側で実際に計算が停止するかも負荷・取消の実測対象であり、HTTP切断だけで保証しない。
モデル採否にはrevision/hash/tokenizer/license/dtype/最大長のmanifestと、日本語の否定・条件・金額・
期限を含む評価、cold/warm latency、peak RSSが必要。未測定なら「即時」と表示しない。

公式資料確認: [llama.cpp server](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md)
はchat completionとschema制約を提供する。導入buildで同じ契約を実測する必要がある。
代替の[Ollama structured outputs](https://docs.ollama.com/capabilities/structured-outputs)も
schema生成に対応するが、今回の初期probeでは実行ファイル未確認のため新規導入しない。
fastembedは検索候補の取得に限り、意味的な事実同値性・否定の理解・本人の選好の証明には使わない。
Codex/Claude内蔵adapterの公式認証・usage・cancel検証は別責務であり、このlocal提案で完了扱いしない。

## 再利用と調査の境界

library-firstとして既存serde/serde_json、domain strict parser/validation、Tokio timer/watchを採用。
独自JSON parser、ranking、一般agent frameworkは追加しない。
codebase-graphの既存indexでRankRequestを照会するとRust structとTSの型/関数が3件返った。
Rustの呼び出し解決は同skillの検証済み対象外であり、index鮮度・全呼び出し網羅性を保証しない。
Rustの所有境界とscorerの意味は実コードを照合した。
要件の参照元は `exec_plan/C-DNA_Requirements_v1.0.0_2026-09-21.md` の11、12、13.4、15章。
