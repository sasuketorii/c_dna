# Astra teaching review — 2026-09-22

独立レビューと Fable への一往復の批評。対象は現在の作業中ファイルの静的確認であり、実装完了・実機性能の証明ではない。共有仕様の変更は coordinator が担当する。

結論：CEO と Codex / Claude Code が質問・回答・反例・訂正を反復して判断の境界を教えることを製品の中心にする。Python はその記録から表現と student を学習・評価し、Rust / Mojo は蒸留済みの判断を速く返す。LLM を推論の全要求から外すことと、学習から外すことを混同しない。

| 衝突箇所 | 修正すべき契約 |
|---|---|
| README.md:132 / README.en.md:132 の工程分離、および要件 §29.5 末尾「循環の一部をLLMに任せる場合」 | 分離は速度・権限境界であって中心性の否定ではない。teacher loop を標準導線として明記する |
| 要件 §15.1:1646 の完全ローカル・テンプレート | 未接続時の記録・復習・既存モデル利用は維持するが、テンプレートだけで意味理解を伴う完全な学習体験を達成したとはしない |
| learner/src/cdna_learner/worker.py:149,208 と crates/cdna-app/src/inference_runtime.rs:8-9,25-42 | 固定 6 属性＋欠損 6＋相互作用 5 は baseline。CEO の独自概念を捨てる恒久的な語彙上限にしない |
| crates/cdna-app/src/with_ai.rs:1-2,40,96 | AI は候補作成だけでなく、理由・逆転条件を確認する対話を担う。現在の回答記録は rationale_explicit=None、reversal_conditions=[] で意味を回収できない |
| crates/cdna-app/src/inference_runtime.rs:26 および返却 JSON | inferred の拒否自体は妥当だが、確認済み入力へ移す契約が必要。順位 JSON のみでは相談の答えにならない |

実装する最小の一周：CEO の実例 → LLM が選択理由と最小の逆転条件を質問 → CEO が訂正 → 出典付き確認記録 → Python が仮説ルール・意味表現・student を候補学習 → 未使用の本人回答で比較 → 次の識別質問。初回から「今回はAを選びます。理由は確認済みのXです。Yが変わるなら再確認します」のような案件固有の自然文と根拠を返す。根拠がなければ、その不足を特定した質問を返し、CEO の回答を捏造しない。

表現 bundle は representation_id、schema、抽出器/tokenizer/embedding の版と hash、ルール AST、特徴辞書・正規化、student、校正、学習 snapshot、出典 lineage、削除 epoch を結合する。LLM が自由に runtime コードを生成するのではなく、既存 §8.1 の型付きルール AST と版付き extractor 契約へコンパイルし、fit/transform 一致・失効を確認する。長文・独自概念には意味特徴や検索を候補にし、必要性を ablation で選ぶ。

出題は単なる件数補充ではなく、仮説が分かれる反実仮想ペア、近い反例、未学習領域、時点変化を扱う。期待される不確実性減少を CEO の回答秒数で割り、同じ問いの反復を抑える。CEO は自由文、二案比較、理由訂正、スキップを選べる。

合成例は人の確認済み実例・方針を anchor とし、parent case、生成モデル/prompt 版、変換内容、仮想前提、未確認ラベル、確認者を保存する。仮想シナリオに明示した前提と、現実に起きた事実は別軸。人が選んだ仮想例の preference は人由来だが、LLM の疑似ラベルは別重みの補助データ。疑似例を独立した本人件数や heldout に数えず、同じ anchor の派生物を同じ split に閉じる。

routing は検証済み表現・対象領域・校正範囲が一致すれば student、未知概念・競合・不足なら LLM に許可済み train evidence と共に escalation、判断不能なら CEO 質問。LLM 案を student の承認済み結論に偽装しない。自然文は選択・根拠・条件に束縛し、生成による順位変更は別の提案として表示する。

初日からの provisional learning use は記憶・確認方針・LLM 仮説を表示して訂正できる状態であり、本番判断権限を持たない。最低100回答や強い統計閾値を対話開始条件にはしない。本番 student 昇格は既存の署名、本人承認、独立評価、対象範囲、epoch、atomic pointer を維持し、領域単位で許可する。

受入は (1) CEO 回答分数と独立 anchor 数ごとの学習曲線、(2) coverage と回答した部分の一致率・誤り・保留、領域別分母と区間、(3) ランダム出題/固定17特徴/記憶検索/本人プロフィール付きLLMとの同条件比較、(4) student p50/p95 と新規文章から自然文までの全体 p50/p95、LLM route率、CPU/RSS、(5) CEO の訂正回数・回答秒数・スキップ率を別々に測る。初期 pilot は少数例でも動かし不確実性を表示する。本番閾値は用途別の誤判断コストと保留許容量から事前設定し、全保留で合格しない。

本人再現、事業成果、汎用知能は別指標。Jev や LLM を超えることは未検証の目標であり現在の優位性ではない。heldout 本文・答えは teacher の検索・要約・表現選択にも渡さず、閲覧・訂正した評価例は以降 training 側へ退役して新しい評価を用意する。

討論記録と最終提案は末尾に記載。

補足確認：`natural_language.rs:221-225` は非 null 抽出値を一律 inferred にする。source span は文字位置の存在しか保証しないため、自動で explicit に変える修正は不可。引用の直接抽出・正規化の意味・人の確認状態を分離し、確認操作後の再推論を受入に加える。

`with_ai_profile` はモデルが存在する場合だけ train evidence を構築する（with_ai.rs:24-38）。モデル未作成でも、事前に train 専用と割り当てた確認実例から teacher を始められる bootstrap 契約が必要。`with_ai_enqueue` は各問いに新しい family を要求する（同:64-65）ため、派生問題の同一 anchor grouping は別の lineage/group ID として保存し、UUID の新規性で独立評価とみなさない。

件数制約：worker.py:180 の最大2000件、:277 の test 境界85%、:337 の評価200件条件は、通常の均等な時刻分布で全体 test 約300件となる。六領域それぞれ200件を同時要求する計画とは整合しない（同時刻境界では比率も変わるため厳密な最大300ではない）。一領域から開始し、複数領域を本番化する前に snapshot/evaluation 予算を設計し直す。現在の provisional training 分岐（:274-284）は存在するが、それだけで自然文の暫定利用導線が存在する証明ではない。

補助構造確認：rev_skills の codebase-graph で learner の features を search/1-hop trace し、pairs/evaluate/memory_baseline などの候補を取得した。candidate_only、鮮度・全数は未検証。固定特徴の指摘は本文照合に基づく。重いテスト・コード変更は行っていない。

仕様の最小修正箇所を追加する：§9.5:838-844 は LLM を QuestionGenerator の文章化だけに配置している。teacher は GapDetector / QuestionPlanner / AnswerInterpreter にも仮説提案・意味確認として参加し、validator / permission / split はコードが所有する、と役割を明示する。§11.4:1131 の「利用できる」は標準 teacher と条件付き inference escalation の二つに分ける。REQ-UNC-008 の順位保護、REQ-AL-010 の意味確認、REQ-EVO-002 のAI回答非本人扱いは保持する。

最初の縦実装の具体例：CEO に「短納期でも再利用性を優先した最近の判断」を一つ聞き、LLM がその理由を要約し、締切だけを変えた二案を質問する。CEO が「既存顧客の障害復旧なら即応」と訂正したら、数値 speed/reuse だけに圧縮せず、その例外条件と出典を版付きルール候補として保持する。翌回の未知の言い換えに対し、理由・例外を踏まえた答えを出せるかを確認する。この会話で見せた反例は評価用の未使用例には数えない。

追加ユーザー方針 — 既存 ChatGPT / Claude からの bootstrap：最初から質問票を埋め直させない。ユーザーが既存アプリへ貼るプロンプトと、返却 JSON の preview/import を用意する。アプリ間の履歴アクセス能力を仮定せず、今アクセスできる文脈だけを対象にし、知らないことは unknown にする。

仕様 §6.2:450-475 に標準プロンプトは既に存在する。新しい並行プロンプトを製品正本にせず、これをコピー可能にし、bootstrap 担当の正式 schema に接続することが修正である。REQ-IMP-001 に従い、出典のない要約を本人が確認しても実行事実にはせず endorsed_statement とする。初稿の独自 JSON 案は採用しない。

bootstrap 担当の提案する parse_profile / import_profile / revise_profile と、Source document の id+revision を接続点にする。profile は provider/model、参照範囲、criteria の tradeoffs/examples/exceptions/reversal_conditions/evidence、contradictions/unknowns を持つ。外側の authority=ai_generated_hypothesis、training_eligible=false、heldout_eligible=false は本人の編集だけでは変わらない。別の明示確認から endorsed_statement または出典のある観測を作り、元 Source はそのまま保つ。正式な schema の詳細は bootstrap 実装が所有する。

importer は入力サイズ・配列数・文字数を上限付きで検証し、無効 JSON は位置付き修正案を示す。quoted でも本人確認済みにならず、自由文は実行しない。provider/model と confidence は外部サービスの自己申告であり、認証済み提供者や校正済み確率とは表示しない。

preview では「確認／修正／不明／除外」を claim 単位に選べる。数十項目の全件承認を初回条件にせず、最も情報価値が高い3件程度から始める。確認した方針は方針として、実行事実の出典を確認した過去選択は観測として保存し、文章要約から架空の選択ラベルを作らない。未確認 export は teacher の仮説作成に使えても、本人の正解・production 学習適格データにはしない。派生質問は元 export/claim lineage を継承し、import だけで heldout 精度を増やさない。既存 import→preview→確認→snapshot の所有経路に接続し、二つ目の記憶DBや並行学習基盤を作らない。

並行実装の更新（担当者報告、ここでは再検証していない）：withAI 担当から理由・逆転条件・model_exposure の保存と question_origins/同一 TRAIN family の派生制約を修正済みとの連絡を受けた。上記の旧行番号は初回調査時の欠落を指す。輸送・保存の修正完了と、teacher の自然文品質・意味表現の蒸留完了は別である。


Fable との直接討論（Orca: 独立案 msg_fcf020b11bab、相手批評 msg_7952fa082dcd、Astra 反論 msg_8f5479c9efa8）：

| 争点 | Astra の最終提案 |
|---|---|
| 新しい DSL が必要か | 不要。既存 §8.1 の AST を再利用する。障害復旧の例外は既存演算子の組合せと承認済み辞書キーで表せる。辞書 hash だけでなく extractor/tokenizer の版も bundle に含める |
| 疑似ラベルを学習へ入れるか | 明示的な distillation role/source を仕様 §7.5 に追加する。本人正解にはせず TRAIN の人 anchor に束縛し、anchor 単位の総重みと生成量に上限を置く。teacher disclosure の AiGenerated 除外だけを根拠に、learner 全体が蒸留禁止と断定しない |
| inferred を感度検査すれば即答してよいか | 暫定 advisor の参考出力として採用可。推定値域自体が誤る・未知概念を落とす場合は順位が安定していても誤るため、本番の唯一のゲートには不可。校正された抽出契約と自然文入力からの独立評価を別途要求し、hard-rule/OOD/失効の条件を残す |
| 毎回答の失効を避けるか | 回答は即保存し、session の草稿 revision と全体の privacy epoch を分けて残りの問いを再検証・rebaseする。削除・訂正・権限変更をセッション commit まで遅らせない。train-only retrieval は snapshot/split/revision/epoch を束縛し、件数・token・page を制限する |
| 本番を外部 MCP だけに限定するか | 不可。画面かMCPかではなく承認状態・用途・実行権限で分ける。CEO画面も opt-in MCP learning client も暫定 advisor を使えるが、承認済み本人判断や実行許可と偽装しない |
| 蒸留を除くと評価値は不変か | 学習データを除けば予測と評価値は変わり得る。守るのは heldout membership/labels/分母への合成例非混入であり、指標の不変性ではない |

採用順：既存 §6.2 prompt/bootstrap Source → train-only teacher context → 本人の明示確認/endorsed_statement と理由・逆転条件付き回答 → 既存 AST/辞書候補 → 一領域の provisional advisor → 意味表現・蒸留の独立比較 → 用途別の本番昇格。最初の縦断に新しい DSL、全領域同時合格、常駐 Python を要求しない。自然文回答の受入は具体案件で選択・根拠・例外・不足質問を返し、CEOの訂正が次の問いと次回回答へ反映される往復で確認する。

Fable の初日導線案を採用：export だけの保管庫でも、承認された発言の検索、承認済み Principle、または LLM 仮説から、出典を区別した答えと必要なら一つの確認質問を返す。student が未学習なら、student の答えとはしない。Fable の「20 heldout groups / coverage 0.3」は pilot 評価の候補値としてのみ扱い、対話開始の前提にはしない。teacher に見せるデータは開示前に train 専用へ割り当てるが、既存の hidden evaluation family を本人確認時に勝手に train へ再割当てしない。

最終収束（Fable msg_8d020236ccc7、Astra msg_4eb6f1849164、coordinator 決定 msg_84da8c57512c）：既存 AST、版付き表現、明示蒸留契約、即時 privacy epoch、用途・権限による本番区分、評価分母保護に合意した。推定入力の暫定出力を未検証のまま常時既定表示するかは両者に差が残ったが、coordinator が「明示的に入った teaching mode 内で暫定助言を既定表示、前提を可視化し、hard-rule/OOD/情報不足では一つの確認質問、本番は抽出＋student の全体評価を維持」と決定した。この限定を最終提案に採用する。感度検査だけで本番へ上げない。

escalation の元回答は仮説として保存し続ける。後日の明示的な本人確認は、teacher exposure と正しい endorsed_statement/仮想/実例の区別を持つ別 anchor を作り、そこから蒸留例を派生させる。元の AI 回答をその場で本人の真実へ書き換えない。重み上限や pilot 数値は実測前の設計値であり、20件/coverage 0.3 を認証値や初回利用条件にしない。

レビュー完了。担当ファイルのみ変更し、コード・共有仕様・重いテストには手を加えていない。現在の transport 修正と固定17特徴は、意味を学ぶ teacher loop と版付き意味表現の蒸留の完成ではない。実装・実クライアント往復・自然文品質・本人の独立評価・全体 latency は、それぞれ別途受入が必要である。

Fable 最終確認 msg_9332502e9977：coordinator の teaching-mode 限定を採用し、数値範囲の創作禁止、本人確認から抽出評価を集める方針にも同意。両者の残余意見差は解消した。
