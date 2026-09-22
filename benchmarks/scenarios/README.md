# 合成選好シナリオ v1

`preference_cases.jsonl` は、明示的な効用規則に対して学習・順位付け・入力拒否・保留の挙動を調べる **合成テスト専用** データです。実在人物の回答、経営上の正解、本人再現精度の根拠ではありません。自然文と仮定はエージェントが作成し、数値は固定seed `20260922` と明示した計算式で構成しています。特定人物の選好は模倣していません。

全142ケースです。通常120ケースと診断22ケースを分けています。

| split | ケース数 | family数 | 用途 |
| --- | ---: | ---: | --- |
| train | 60 | 60 | 合成効用の学習 |
| validation | 20 | 20 | 合成データ上の設定選択 |
| calibration | 20 | 20 | 合成データ内の校正実験。本人確率ではない |
| holdout | 20 | 20 | 設定決定後の合成汎化確認 |
| challenge | 22 | 19 | 境界、情報不足、矛盾、単位、同点、文脈による逆転 |

各通常splitには6領域すべてを含めます。通常ケースは、資源配分、開発・提供、顧客・取引、組織・委任、成長戦略、リスク・信頼を各20件含みます。テンプレートと生成規則はsplit間で共有されるため、これは未知の実務分布への転移評価ではありません。各領域の校正例は20 familyに達しておらず、製品の独立校正要件を満たしません。

## JSONL契約

外側はベンチマーク用のメタデータです。`Record.model_validate` に渡すのは `row.record` **だけ**です。外側の `source` や `oracle` を通常学習器へ直接投入しません。

```text
schema_version: "1.0"
case_id: 一意なケースID
split: train | validation | calibration | holdout | challenge
source:
  kind: synthetic_oracle
  human_label: false
  personal_accuracy_evidence: false
  authorship: agent_authored_scenarios_with_deterministic_arithmetic
  seed: 20260922
record: learner の Record schema
oracle:
  profile_id: contextual_tradeoff_v1
  weights17: 既知の係数17個
  utilities: candidate_id -> number | null
  preferred_ids: 効用最大の候補ID集合
  expected: rank | tie | abstain | reject
  reason: このケースの正解規則・制約
expectation:
  record_valid: Record構造検証の期待結果
  feature_valid: 特徴抽出までの期待結果
  production_train_eligible: false
  benchmark_eligible: 通常120ケースのみtrue
```

すべての `record.verification_state` は `unconfirmed`、`case_kind` は `hypothetical` です。通常の製品学習APIはこれらを除外しなければなりません。本人確認を偽装するために `confirmed` へ書き換えてはいけません。ベンチマーク担当は別の明示的な入口で数値ルーチンを実行し、結果に `synthetic_only` の来歴を残してください。合成学習の重みを本人用モデルとして登録・適用しないでください。

`benchmark_eligible=false` のケースを通常の順位正答率へ混ぜません。`oracle.expected=reject` は拒否を成功と数え、`abstain` は安全側の保留を別に数えます。未知値であっても現在の特徴抽出はゼロ／missingへ写像できる場合があります。したがって `feature_valid=true` と `expected=abstain` は矛盾しません。入力可能性と回答可能性を分けています。

## 既知の効用

通常の全120ケースは同一の既知プロファイルを使います。異なる人物の好みを混ぜたものではありません。特徴順序は既存 `feature_version=1.0` に一致します。

| index | 特徴 | 係数 |
| --- | --- | ---: |
| 0 | cost | -0.65 |
| 1 | effort | -0.35 |
| 2 | speed | 0.30 |
| 3 | reuse | 0.45 |
| 4 | customer_impact | 0.60 |
| 5 | irreversibility | -1.10 |
| 6–11 | 上記6属性それぞれのmissing indicator | 各-0.20 |
| 12 | deadline_pressure × speed | 1.40 |
| 13 | asset_importance × reuse | 1.00 |
| 14 | loss_tolerance × irreversibility | 1.05 |
| 15 | context.customer_impact × speed | 0.45 |
| 16 | budget_pressure × cost | -1.20 |

`U(candidate, context) = Σ weights17[i] × features[i]` です。通常の選択ラベルは効用最大の候補です。費用・工数・不可逆性を通常は嫌い、便益・速度・再利用性を好む一方、期限、資産化の重要性、損失許容度、顧客影響、予算圧力に応じて取引条件が変わる仮想プロファイルです。`customer_impact` 属性はこのデータ内では顧客への**便益**を意味します。損害の大きさではありません。

通常値はすべて0〜1の相対値で、金額・日数の実測値ではありません。交互作用の背景値には必ず `unit=ratio` を付けます。候補属性には現行schema上の単位欄がないため、候補の6属性も同じ正規化尺度として明示しています。係数の大小に普遍的な経営上の妥当性はありません。

既知関数が線形であるため、線形モデルに有利な構成です。このデータのみでLightGBM等のモデル群より優れている、自然文を理解できる、実運用で高精度である、と結論づけてはいけません。oracle係数・効用・preferred_idsを学習入力に加えることも禁止です。学習側はrecordの観測ラベルと入力特徴だけを使用してください。

## 診断ケース

- 必須背景5種のunknown、および候補の費用・不可逆性の欠落：保留。
- 全候補の特徴が同一：同点。IDや表示位置で正解を作らない。
- 0、1、-1の正規化境界：-1は特徴契約の符号付き下限を試す抽象入力で、通常の業務シナリオには含めない。
- `percent`単位、単位なし、背景値1.01：Record検証で拒否。
- 候補値1.01、金額オブジェクト：Record検証は通るが数値特徴抽出で拒否。金額をratioとみなさない。
- 納期圧力だけを0→1へ変える同一二候補：低費用案から高速案へ順位が逆転。
- 損失許容度だけを0→1へ変える同一二候補：可逆案から高便益・高不可逆案へ順位が逆転。
- 最後の2行は同一入力・同一familyで異なる観測ラベルを与えた故意の矛盾。oracleは変えず、後者のラベルだけを誤らせている。双方とも教師信号には使わず、矛盾の検出・隔離を検証する。

関連する逆転ペアと矛盾ペアは同じfamilyに属し、同じchallenge splitに固定しています。他のfamilyもsplitをまたぎません。候補位置は固定seedで入れ替えています。通常ケースには生成テンプレートを共有する限界があるので、family分離だけを「実世界の完全な独立性」と解釈しないでください。

## 検証済み範囲

作成時に142行すべてを現行 `learner/src/cdna_learner/worker.py` の `Record.model_validate` と `features` で検査しました。

- Record／特徴抽出の成否がexpectationと一致。
- rank／tieの保存効用と現行17特徴の内積が誤差1e-9未満で一致。
- family139群でsplit重複0。
- 2種類の文脈逆転ペアでoracleの優先候補が実際に逆転。
- 全142行がunconfirmedかつproduction_train_eligible=false。

これはschema・数値・分割の検証です。モデル学習後の結果、保留率、矛盾検出、Rustとの全例一致、本人再現精度の合格を示すものではありません。
