# Fable teaching review — 2026-09-22

独立初稿(claude-fable-5-1、effort low)。対象は現在の作業ツリーの静的確認であり、実装完了・実測性能の証明ではない。共有仕様・コードの変更は coordinator と各所有者が行う。本文書は自分の所有ファイルのみ。

## 0. 結論

現在の仕様とコードは「LLM は取り込み時と説明時の周辺サービス、本人モデルは固定 17 特徴の線形スコアラー」という前提で書かれている。ユーザーの中核意図(CEO と高性能 LLM の反復対話で暗黙ルール・反実仮想・例外を露出させ、それを蒸留して反射速度の個人判断を返す)とは、次の三点で構造的に食い違う。

1. **教師対話の産物が学習に届かない。** `rationale_explicit` と `reversal_conditions` は自由文で保存されるが、learner 契約は「Text, IDs, domain strings, outcomes and reasons are never features」(learner/CONTRACT.md)であり、CEO が言語化した判断境界は訓練時に捨てられる。学習しているのは四択の勝敗と 17 個の数値だけである。
2. **語彙が固定されている。** `crates/cdna-inference/src/lib.rs:12` の `DIM = 17`、`learner/src/cdna_learner/worker.py:20-27` の `KEYS`/`INTERACTIONS` は設計者が事前に決めた 6 属性と 5 相互作用で、CEO 固有の概念(「既存顧客の障害復旧」「社内資産化」など)は表現できない。REQ-ML-007 の「小さく固定」は初期版としては正しいが、拡張経路が仕様に無い。
3. **有用な推論がほぼすべて保留か確認 UI に落ちる。** 自然文からの抽出値は一律 `inferred`(docs/architecture/NATURAL_LANGUAGE.md、natural_language.rs)、`inferred` は相互作用に使われず(cdna-inference `extract_features` は `explicit` のみ)、かつ「Recognized context facts explicitly marked unknown/inferred/null … also trigger abstention」(CONTRACT.md)。つまり自然文で相談した場合、CEO が数値を確認するまで学習モデルは必ず保留する。これは「商談中に間に合う」という README の約束と両立しない。

提案の骨子は「教師対話 → 型付き知識表現(ルール AST + 誘導特徴辞書 + 事例)→ 蒸留 student → 実行時 routing」を標準導線とし、確認ゲートを**実行権限・校正主張を伴う本番用途**に限定して、CEO 本人と opt-in 学習クライアントには初日から暫定出力を見せることである(「本番」の定義は討論で権限・用途基準に修正。§8 参照)。

## 1. 衝突箇所と修正文言

| 箇所 | 現在の文言 | 問題 | 提案文言 |
|---|---|---|---|
| README.md「全体構成」冒頭 / README.en.md:132 | 「学習・推論・LLM による文章理解を、別の工程として切り離す」 | 分離を「LLM は学習に不要」と読める。ユーザー意図は逆(学習時の LLM コストは歓迎)。 | 「LLM 呼び出しを**実行時の毎回の判断**から切り離す。学習時には LLM を教師として積極的に使い、その費用は学習の費用として扱う。速さは蒸留の結果であり、LLM を訓練から除外する理由ではない。」 |
| 要件 §9.5:838-844 | LLM の位置は `QuestionGenerator` の「テンプレート／LLMで文章化」のみ | GapDetector・AnswerInterpreter に LLM が居ないので、備考の意味を型付き観測へ変換する主体が無い。 | `TeacherLoop` を新設し、GapDetector(仮説の言語化)・QuestionPlanner(反実仮想ペア設計)・AnswerInterpreter(理由・例外を型付き候補へ変換)へ LLM 参加を明記。Validator / permission / split の所有はコード。 |
| 要件 §11.4:1131 / REQ-UNC-010 | 「LLM は候補作成、文章理解、質問生成、説明、改善仮説に利用できる」「LLMフォールバック」 | 「できる」「フォールバック」という語が LLM を任意の補助に格下げしている。 | 「LLM は(a)**標準の教師**、(b)条件付きの**推論 escalation** の二役。(a) は学習フェーズの既定 ON。(b) は同意・予算・権限がある場合に student が未知概念・競合・不足を返した時に呼び、その回答は `ai_generated` 提案として表示する。」 |
| 要件 §15.1:1646 | 「完全ローカル・テンプレート:必須。外部AIなしで基本学習と推論を使用」 | テンプレートで「基本学習」が成立すると読める。成立するのは記録・検索・既存モデル推論であり、意味理解を伴う教示ではない。 | 「外部 AI なしでは、記録・検索・承認済み方針・既存 student 推論を使える。**教師対話による新規学習は LLM(ローカル含む)を必要とする。**」 |
| 要件 §7.5 表 + REQ-EVO-002 + crates/cdna-app/src/training_snapshot.rs:39(`AiGenerated` を訓練 snapshot から除外)+ lib.rs:109(`AiGenerated` は確認済み観測になれない) | 「AIが生成した回答:本人ラベルにはしない」「改善エージェントが作った質問・説明を…自動追加しない」 | 「本人ラベルにしない」は正しいが、実装は「AI 生成は一切訓練に入れない」まで踏み込んでいる。蒸留(KD)を禁止すると student は人間回答の件数でしか育たない。 | 「AI 生成回答は本人ラベルとして**数えない**。ただし本人確認済み実例・方針を anchor とする**蒸留データ**として、`source_kind=ai_generated`、別重み、評価には不使用、anchor と同一 split、という条件で student の学習に使ってよい。」 |
| learner/CONTRACT.md「Text … reasons are never features」 + REQ-ML-008 | 回答後の理由を特徴にしない | REQ-AL-011(答えを含む理由からの漏洩)は正しい。しかし「理由からルール候補を誘導し、**別の**事例に適用する」ことまで禁じている読みになっている。 | 「同一事例の理由をその事例の特徴には使わない。理由・逆転条件から誘導した**型付きルール候補と辞書キー**は、本人承認後、他事例の特徴・ルール層として使う。」 |
| learner/CONTRACT.md「Recognized context facts explicitly marked … inferred … trigger abstention」 | inferred で必ず保留 | 自然文相談は常に保留になる。 | 「`inferred` の fact は値域を持つ推定入力として採点に使う。保留は、推定値の妥当範囲内で順位が反転する場合(感度検査)に限る。出力には `input_evidence: inferred` と反転条件を添える。」 |
| CONTRACT.md「200 test cases, 100 non-abstained, coverage .6, agreement .9, Wilson lower .85」領域ごと必須 + worker.py:180 の 2000 件上限 | 六領域すべてで満たすには数千件の本人回答が要る(Astra の試算とも整合) | 実質的に student が永久に「暫定」のまま。 | 「`statistically_eligible` は報告ラベルとして残す。**内部 advisory 昇格**は paired-baseline(記憶検索・固定17)に対する優位と区間表示だけを条件とし、**実行権限・校正主張を伴う本番用途への昇格**にだけ数値閾値を課す。閾値は用途別の誤判断コストから事前設定し、領域単位で許可する。」 |
| docs/architecture/WITH_AI.md「A human answer is itself an input change… invalidates the rest of that batch」 | 1 回答ごとにバッチ全体が失効 | 教師対話が「1 問答えるたびに profile 再取得 → 再提案」になり、反復対話として使い物にならない。 | 「回答は即時永続化する。削除・訂正の privacy epoch は遅らせない。教示セッションは別の draft revision を持ち、残りの質問は更新後の状態へ **rebase** する(バッチ全体の失効にしない)。」(討論で修正。§8 参照) |
| WITH_AI.md「Do not add `decision:read`… to a teaching-only client」「at most 32 confirmed evidence records」 | 教師が過去事例を検索できず、32 件しか見えない | 反実仮想・近い反例を設計するには過去事例へのアクセスが要る。 | 「教師 grant には `decision:read_train` を新設し、TRAIN 分割かつ確認済みの事例だけを paginated で読める。validation/test 分割は列挙自体を拒否する。」 |

## 2. 知識表現と特徴誘導(固定 17 からの出口)

- **v1.0 = 17 特徴は baseline として維持する。** REQ-ML-010(新しい特徴版は旧モデルに投入しない)と REQ-MOJ-001 の同値性試験のために、固定次元の版は必要。
- **v2.x = 誘導特徴辞書。** `feature_version` に辞書 hash を含め、辞書は `{key, kind(numeric|bool|categorical), unit, source(claim_id|question_id), approved_at, revision}` を持つ。キーは教師対話で CEO が使った概念(例: `existing_customer_outage`, `asset_internalization_goal`)から LLM が提案し、CEO が承認したものだけ辞書に入る。REQ-ML-007 の「自動で全組み合わせを追加しない」はそのまま守れる。
- **ルール層は既存 §8.1 の型付き AST を再利用する。** 新しい DSL を作らない。教師は `rationale_explicit` と `reversal_conditions` を `hypothesis` 状態の Principle 候補(AST)へ変換し、CEO が `approved` にした時点で推論のルール層に入る。ルールは student より先に評価され、`unknown` は REQ-POL-003 どおり素通りしない。
- **student = 辞書上の線形/木モデル + ルール層 + 事例検索**の三層で、出力は「どの層が決めたか」を必ず返す(REQ-UNC-007 の根拠 ID 対応)。
- **Rust 側の DIM 固定は `feature_version` ごとの schema 読み込みへ置き換える。** 実行時に辞書を読み、`extract_features` は辞書駆動にする。Mojo adapter は数値配列だけを受け取るので影響しない。

## 3. 蒸留・合成カリキュラム・出所

- `source_kind` に `ai_generated_distill` を追加し、`anchor_case_ids[]`、`teacher_model`、`prompt_version`、`transform`、`weight` を必須にする。anchor は本人確認済み TRAIN 事例に限り、anchor 1 件あたりの蒸留行の重み合計に上限を設ける。人由来の `human_app`/`human_confirmed` と決して混ぜない。
- 合成事例の split は anchor の split を継承する。`with_ai_enqueue` の「新しい family 必須」は独立性の証明ではないので、`lineage_group_id` を別に保存する(Astra 指摘に同意)。
- CEO が LLM の推論を見た後に答えた場合は `teacher_exposure=true` を付ける。これは**訓練から除外しない**(教示対話の本質)。`model_exposure=true` は student の答えを見た場合に限定し、主評価から除く現在の扱いを維持する。

## 4. 実行時 routing と保留

```text
入力(構造化 or 自然文)
 → 抽出(LLM or テンプレ)。値は explicit / inferred / unknown を保持
 → ルール層(承認済み AST)。hard 条件は unknown で停止
 → 事例検索(確認済み TRAIN のみ、権限フィルタ先行)
 → student 採点(辞書 v2 + 17 baseline 並列)。inferred は感度検査付き
 → 三者が整合 → 即答(根拠 ID + 入力の証拠状態 + 反転条件)
 → 不整合 / 未知概念 / 辞書外キー → LLM escalation(同意・予算内)。結果は ai_generated 提案
 → escalation 不可 → 保留理由 enum + CEO への 1 問
```

保留は「答えない」ではなく「何が分かれば答えられるか」を必ず添える。README の三モード(本人再現/改善提案/保留・質問)はこの routing の出力ラベルとして残す。

## 5. 評価と漏洩防止

- テスト分割は**時間前方 + 教師盲検**。教師 grant は test/validation family の本文・ID を一切受け取らない(現在の TRAIN allowlist を維持)。
- 教師が閲覧・要約した事例はその時点で train 側へ退役する(Astra と同意)。
- 主指標は「学習曲線(CEO 回答分数 × 独立 anchor 数 → coverage と一致率)」と「baseline 比較(明示ルールのみ / 記憶検索のみ / 固定17 / プロファイル付き LLM / student)」。**Jev や LLM を超えるという主張は、同条件比較の実測が出るまで文書に書かない。**
- `statistically_eligible` の 200/100/.6/.9/.85 は外部公開用の閾値として残し、内部 advisory と学習ループの前提にしない。

## 6. Python / Rust / Mojo の役割

| 層 | 役割 | しないこと |
|---|---|---|
| Python | 教師対話ログからのルール候補・辞書候補の誘導、student の fit、校正、評価、学習曲線 | 常駐推論、保管庫への直接書込 |
| Rust | 認可・同意・epoch、ルール AST 評価、事例検索、辞書駆動の特徴抽出、student 採点、routing、LLM escalation adapter | 学習アルゴリズムの再実装 |
| Mojo | 実測で効果が出た数値ループのみ(REQ-MOJ-005) | R1 の受入条件に登場すること |

LLM adapter(Codex App Server / Claude Agent SDK / llama.cpp)は Rust の一つの port であり、教師対話と escalation の両方が同じ port を使う。

## 7. Phase 1 で今実装できる最小縦断

1. 教師 grant に `decision:read_train`(paginated、TRAIN のみ)を追加。
2. 教示セッションの draft revision を privacy epoch と分離し、回答後は残り質問を rebase する(WITH_AI.md の per-answer バッチ失効を廃止)。
3. `with_ai_respond` の `rationale_explicit`/`reversal_conditions` から、LLM が Principle 候補(既存 AST)と辞書キー候補を生成し `hypothesis` として保存。CEO 承認で `approved`。
4. `extract_features` を辞書駆動化し、`feature_version="2.0+<dict_hash>"` を導入。17 は `1.0` として並走。
5. `inferred` fact の感度検査付き採点。保留は反転時のみ。
6. `source_kind=ai_generated_distill` の受入と、anchor split 継承。
7. 受入テスト: (a) 同じ自然文で辞書 v2 が v1 より保留率を下げ、一致率が下がらない、(b) 教師が test family を取得できない、(c) 蒸留データを抜いても heldout の**構成・ラベル・分母**が変わらない(評価値は変わってよい。Astra の訂正を採用)、(d) export のみの保管庫で自然文相談が、ラベル付き回答か 1 問の確認のどちらかを返し、素の保留を返さない。

## 8. Astra との討論結果(1 往復、2026-09-22)

### 合意した点

1. **DSL は作らない。** Astra の「検証済み DSL」は新言語ではなく検証済みルール契約の意味だった。Phase 1 は §8.1 AST + 承認済み辞書版 + extractor 版で行く。「既存顧客の障害復旧なら即応」は AST の conjunction で表現できる。
2. **蒸留の明示例外を §7.5 に書く。** `ai_generated_distill`、本人確認済み TRAIN anchor 限定、anchor あたり重み上限、ablation 報告、AI ラベルは本人真値ではない。私の初稿は with_ai.rs:40 を訓練除外の根拠にしていたが、それは教師開示の除外であり、正しい根拠は training_snapshot.rs:39 と lib.rs:109(修正済み)。
3. **教師の TRAIN 検索 scope を新設する。** split/revision/epoch を pin、val/test の ID・件数も漏らさない、削除で cursor を失効、件数・token・ページ上限。
4. **epoch は二層に分ける。** 私の「セッション commit まで epoch を遅らせる」は撤回。回答は即時永続化し、削除・訂正の privacy epoch は遅らせない。教示セッションは別の draft revision を持ち、残りの質問は更新後の状態へ rebase する(失効ではなく rebase)。
5. **「本番」は権限・用途で定義する。** 「外部 MCP 消費者のみ」という私の定義は撤回。CEO 画面も実業務の行動を起こし得る。暫定 advisor 出力は CEO と明示 opt-in の MCP 学習クライアントに常時見せてよいが、実行権限と校正済みという主張は持たない。
6. **受入 (c) の訂正。** 蒸留を抜けば heldout 予測は正当に変わる。検査対象は heldout の membership・label・denominator の不変性。
7. **TRAIN 割当は開示前に行うが、遡及しない。** 教師に開示する前に family を train pool へ割り当てる。既存の隠し評価 family を確認時に再分割しない。時間前方 heldout はそのまま残す。
8. **Ladder A の数値(coverage ≥ 0.3、heldout lineage_group ≥ 20/領域)は pilot の測定提案であり、初回利用の前提条件ではない。** 履歴ゼロの保管庫でも教示と暫定表示は始められる。
9. **export のみの保管庫での自然文回答**(承認済み claim の検索、承認 AST、LLM 提案)は合意。出典ラベルと「確認すると答えが変わる 1 問」を必ず添え、student 出力を捏造しない。

### Root 裁定後の最終結論(2026-09-22)

- **論点 A(inferred 感度ゲートの既定表示)は Root 裁定で決着。** 暫定 advisory 回答は**明示的に入った教示モード**で既定とし、日常の承認済み順位 API では既定にしない。inferred の前提は出力に明示し、hard rule / OOD の欠損は保留ではなく**明確化の質問**として返す。感度検査のための**数値範囲を捏造しない**(私の初稿の「妥当範囲内で反転するか」は、CEO が範囲を与えた場合か抽出 envelope が検証済みの場合に限る)。本番には抽出器 + student の end-to-end 受入が必要。Astra の「未検証の感度 student 出力は任意の実験であり、抽出証拠は CEO の確認・訂正が供給する」を採用し、私の「既定表示でなければ学習ループが回らない」は撤回する。
- **論点 B(escalation 回答の蒸留保存)は決着。** escalation 回答は `hypothesis` として自動保存のみ。後日 CEO が明示的に確認した時に、教師露出付きの**別の人間 anchor**(endorsed_statement / hypothetical / actual の出所を正しく付ける)が生まれ、そこから派生する蒸留行が lineage を保持する。元の escalation 記録を in-place で昇格させない。架空の過去事象を作らない。真値の自動ラベルは決してしない。
- **論点 C(数値の placeholder)は決着。** anchor あたり重み上限、coverage 0.3、heldout 20 群/領域は pilot の測定提案であり、認証基準でも初回利用の前提でもない。既存の領域別本番ゲート(200/100/.6/.9/.85)は維持する。
- **その他の Root 採用事項**: 既存 §8.1 AST + 版付き表現、教師には TRAIN 限定文脈、AI 蒸留は人間ラベルと区別・同一 anchor split・重み上限、security epoch は遅延しない、本番は権限・用途で定義。

残る未解決の相違は無い。仕様の修正は Root が行う。

## 9. 使用モデルの申告

本レビューは要求どおり claude-fable-5-1(effort low)で作成した。Astra 側のモデルは未確認。
