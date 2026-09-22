/** Synthetic pre-trained demo. All feature extraction and scoring runs in shared Rust WASM. */
import fixtureData from './demo-data/scenarios.json';
import artifact from './demo-data/model.json';

export type DemoCandidate = { id: string; text: string; attributes: Record<string, number | null> };
export type DemoContext = { as_of: string; summary: string; unknown_fields: string[]; facts: { key: string; value: number | null; evidence_status: string; unit: string }[] };
export type DemoScenario = { id: string; title: string; category: string; summary: string; description: string; keywords: string[]; context: DemoContext; candidates: DemoCandidate[]; conditions: { key: string; label: string; min: number; max: number; step: number; value: number; unit: string }[] };
const factLabels: Record<string, string> = { deadline_pressure: '納期の切迫度', asset_importance: '再利用の重要度', loss_tolerance: '損失の許容度', customer_impact: '顧客影響の重大度', budget_pressure: '予算の厳しさ' };
const featureLabels = ['費用', '工数', '速さ', '再利用性', '顧客への効果', '元に戻しにくさ', '費用の欠測', '工数の欠測', '速さの欠測', '再利用性の欠測', '顧客効果の欠測', '不可逆性の欠測', '納期の切迫度 × 速さ', '再利用の重要度 × 再利用性', '損失の許容度 × 不可逆性', '顧客影響の重大度 × 速さ', '予算の厳しさ × 費用'];
const attributeKeys = ['cost', 'effort', 'speed', 'reuse', 'customer_impact', 'irreversibility'];
export const personaLabel = artifact.personaLabel;
export const demoProvenance = artifact.provenance;
export const scenarios: DemoScenario[] = fixtureData.map(s => ({ ...s, description: s.summary,
  conditions: s.context.facts.filter(f => f.value !== null).map(f => ({ key: f.key, label: factLabels[f.key] ?? f.key, min: -1, max: 1, step: 0.05, value: f.value!, unit: '正規化値' })) }));

/** Keyword lookup suggests existing fixtures; never interprets arbitrary text as model input. */
export function matchScenarios(query: string): DemoScenario[] {
  const q = query.trim().toLocaleLowerCase();
  if (!q || q.length > 500) return [];
  return scenarios.filter(s => s.title.toLocaleLowerCase().includes(q) || s.keywords.some(k => q.includes(k.toLocaleLowerCase())));
}

type WasmResult = { scores: number[]; features: number[][]; contributions: number[][]; engine: 'rust_wasm' };
type Wasm = { default: () => Promise<unknown>; score_candidates: (json: string) => string };
let loaded: Promise<Wasm> | undefined;
/** Preload only the small Rust WASM facade, never Pyodide or the learner. */
export async function warmup(): Promise<void> { await wasm(); }
function wasm(): Promise<Wasm> {
  loaded ??= (async () => {
    const base = import.meta.env?.BASE_URL ?? '/';
    const url = `${base}runtime/domain/cdna_browser.js`;
    const module = await import(/* @vite-ignore */ url) as Wasm;
    await module.default();
    if (typeof module.score_candidates !== 'function') throw new Error('デモ推論エンジンを読み込めませんでした');
    return module;
  })().catch(error => { loaded = undefined; throw error; });
  return loaded;
}
function score(module: Wasm, scenario: DemoScenario, context: DemoContext): WasmResult {
  return JSON.parse(module.score_candidates(JSON.stringify({ feature_version: artifact.model.feature_version, weights: artifact.model.weights,
    request: { schema_version: '1.0', request_id: '11111111-1111-4111-8111-111111111111', workspace_id: '22222222-2222-4222-8222-222222222222', mode: 'imitate', domain: 'product_delivery', context, candidates: scenario.candidates, include_evidence: true, allow_cloud: false }
  }))) as WasmResult;
}
export type DemoContribution = { feature: string; featureValue: number; weight: number; contribution: number; advantage: number };
export type DemoDecision = { scenarioId: string; decision: string; rationale: string[]; reversalConditions: string[]; elapsedMs: number; inferenceMs: number; selectedCandidate: DemoCandidate | null; ranking: { candidate: DemoCandidate; score: number }[]; contributions: DemoContribution[]; explanation: string; abstained: boolean; abstentionReasons: string[]; personaLabel: string; engine: 'rust_wasm' };

export async function decide(scenarioId: string, overrides: Record<string, number> = {}): Promise<DemoDecision> {
  const decisionStart = performance.now();
  const scenario = scenarios.find(s => s.id === scenarioId);
  if (!scenario) throw new Error('登録済みの課題を選んでください');
  if (Object.keys(overrides).length > 5) throw new Error('条件が多すぎます');
  for (const [key, value] of Object.entries(overrides)) {
    if (!Object.hasOwn(factLabels, key) || !Number.isFinite(value) || Math.abs(value) > 1) throw new Error('条件は登録された項目の -1〜1 で指定してください');
  }
  const context: DemoContext = { ...scenario.context, unknown_fields: scenario.context.unknown_fields.filter(k => !Object.hasOwn(overrides, k)),
    facts: scenario.context.facts.map(f => Object.hasOwn(overrides, f.key) ? { ...f, value: overrides[f.key]!, evidence_status: 'explicit' } : { ...f }) };
  const module = await wasm();
  // Cold asset loading is excluded. This measures synchronous shared Rust inference + JSON IO.
  const start = performance.now();
  const raw = score(module, scenario, context);
  const inferenceMs = performance.now() - start;
  const indices = raw.scores.map((_, i) => i).sort((a, b) => raw.scores[b]! - raw.scores[a]!);
  const first = indices[0]!;
  const second = indices[1]!;
  const missing = context.facts.filter(f => f.value === null || f.evidence_status !== 'explicit').map(f => `${factLabels[f.key]}が未確認`);
  for (const c of scenario.candidates) for (const [i, key] of attributeKeys.entries()) if (c.attributes[key] == null) missing.push(`「${c.text}」の${featureLabels[i]}が未確認`);
  const margin = raw.scores[first]! - raw.scores[second]!;
  const abstentionReasons = [...missing, ...(margin <= artifact.model.abstention_margin ? ['候補の差が判断基準に届きません'] : [])];
  const abstained = abstentionReasons.length > 0;
  const selectedCandidate = abstained ? null : scenario.candidates[first]!;
  const contributions = featureLabels.map((feature, i) => ({ feature, featureValue: raw.features[first]![i]!, weight: artifact.model.weights[i]!, contribution: raw.contributions[first]![i]!, advantage: raw.contributions[first]![i]! - raw.contributions[second]![i]! })).sort((a, b) => Math.abs(b.advantage) - Math.abs(a.advantage));
  const rationale = abstained ? abstentionReasons : contributions.filter(c => Math.abs(c.advantage) > 0.0001).slice(0, 3).map(c => {
    const i = featureLabels.indexOf(c.feature);
    const attributeIndex = i < 6 ? i : ({ 12: 2, 13: 3, 14: 5, 15: 2, 16: 0 } as Record<number, number>)[i];
    if (attributeIndex === undefined) return `${c.feature}が判断に影響しています。`;
    const larger = raw.features[first]![attributeIndex]! > raw.features[second]![attributeIndex]!;
    const comparisons = [larger ? '費用が大きい' : '費用が小さい', larger ? '必要な工数が多い' : '必要な工数が少ない', larger ? '提供までが早い' : '提供までに時間がかかる', larger ? '成果を再利用しやすい' : '成果を再利用しにくい', larger ? '顧客への効果が大きい' : '顧客への効果が小さい', larger ? '後から元に戻しにくい' : '後から元に戻しやすい'];
    const setting = i >= 12 ? `${c.feature.split(' × ')[0]}を踏まえると、` : '';
    return `${setting}次点と比べて${comparisons[attributeIndex]}ことが、${c.advantage >= 0 ? 'この案を選ぶ理由になっています' : 'この案の弱点です。他の利点がこの弱点を上回っています'}。`;
  });
  const reversalConditions: string[] = [];
  if (!abstained) {
    // Bounded 5 x 2 local WASM evaluations, not a handwritten sensitivity ranker.
    for (const fact of context.facts) {
      for (const value of [-1, 1]) {
        const changed = { ...context, facts: context.facts.map(f => f.key === fact.key ? { ...f, value } : f) };
        const counter = score(module, scenario, changed);
        const order = counter.scores.map((_, i) => i).sort((a, b) => counter.scores[b]! - counter.scores[a]!);
        if (order[0] !== first && counter.scores[order[0]!]! - counter.scores[order[1]!]! > artifact.model.abstention_margin) {
          reversalConditions.push(`${factLabels[fact.key]}だけを${value > 0 ? '最も高い状態' : '最も低い状態'}にすると「${scenario.candidates[order[0]!]!.text}」が首位になります（他の条件は同じ）。`);
          break;
        }
      }
    }
    if (!reversalConditions.length) reversalConditions.push('各条件を一つずつ最も低い状態・高い状態に変更した範囲では首位は変わりません。複数条件や候補の内容が変わる場合は別途評価が必要です。');
  } else reversalConditions.push('未確認の条件・候補属性を確認するまで、選択は保留します。');
  return { scenarioId, decision: selectedCandidate?.text ?? '情報が足りないため、判断を保留します', rationale, reversalConditions, elapsedMs: performance.now() - decisionStart, inferenceMs,
    selectedCandidate, ranking: indices.map(i => ({ candidate: scenario.candidates[i]!, score: raw.scores[i]! })), contributions: abstained ? [] : contributions,
    explanation: rationale.join('\n'), abstained, abstentionReasons, personaLabel, engine: raw.engine };
}
