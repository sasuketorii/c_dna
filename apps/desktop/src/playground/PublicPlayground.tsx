import { useEffect, useRef, useState } from "react";
import { ArrowDown, ArrowRight, Check, CircleHelp, Clock3, Sparkles } from "lucide-react";
import { Button } from "../components/ui/button";
import { Textarea } from "../components/ui/textarea";
import { decide, matchScenarios, scenarios, warmup } from "../demo-engine";
import "./playground.css";

type Decision = Awaited<ReturnType<typeof decide>>;
type Scenario = (typeof scenarios)[number];

export function PublicPlayground() {
  const [selected, setSelected] = useState<Scenario | null>(null);
  const [result, setResult] = useState<Decision | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [input, setInput] = useState("");
  const [matches, setMatches] = useState<readonly Scenario[] | null>(null);
  const [overrides, setOverrides] = useState<Record<string, number>>({});
  const request = useRef(0);
  const answer = useRef<HTMLElement>(null);
  const formatCondition = (value: number, unit: string) => unit === "正規化値" ? (value <= -0.6 ? "とても低い" : value < -0.2 ? "低い" : value <= 0.2 ? "中程度" : value < 0.6 ? "高い" : "とても高い") : unit === "比率" ? `${Math.round(value * 100)}%` : `${value}${unit}`;

  useEffect(() => {
    void warmup().catch(() => { /* A choice retries initialization and displays failures. */ });
    return () => { request.current += 1; };
  }, []);

  useEffect(() => {
    if (selected && window.matchMedia("(max-width: 680px)").matches) {
      answer.current?.scrollIntoView({ block: "start", behavior: "instant" });
    }
  }, [selected]);

  async function run(scenario: Scenario, values: Record<string, number> = {}) {
    const id = ++request.current;
    setSelected(scenario);
    setOverrides(values);
    setResult(null);
    setBusy(true);
    setError("");
    try {
      const next = await decide(scenario.id, values);
      if (id === request.current) setResult(next);
    } catch {
      if (id === request.current) setError("判断を読み込めませんでした。もう一度お試しください。");
    } finally {
      if (id === request.current) setBusy(false);
    }
  }

  function findScenario() {
    setMatches(matchScenarios(input.trim()));
  }

  return (
    <div className="public-playground">
      <header className="pg-header">
        <a href="?" className="pg-brand" aria-label="CEO-DNA トップ"><span className="pg-mark" aria-hidden="true">C</span> CEO-DNA</a>
        <a className="pg-studio-link" href="?studio=1">自分の判断を学習させる <ArrowRight size={14} aria-hidden="true" /></a>
      </header>
      <main className="pg-main">
        <section className="pg-intro">
          <span className="pg-eyebrow"><span /> CEO-DNA PLAYGROUND</span>
          <h1>この経営判断、<br className="pg-mobile-break" />どう決める？</h1>
          <p>課題を選ぶだけ。経営者の判断と、その理由がわかります。</p>
          <div className="pg-persona"><span className="pg-demo-label">DEMO</span><span>架空の経営者モデル・学習済みの合成データを使用。あなた本人のクローンではありません。</span></div>
        </section>
        <div className="pg-workspace">
          <section className="pg-question-panel" aria-labelledby="pg-question-title">
            <div className="pg-section-heading"><span className="pg-step">01</span><h2 id="pg-question-title">経営課題を選ぶ</h2><span className="pg-heading-note">選ぶとすぐに回答</span></div>
            <div className="pg-scenarios">
              {scenarios.map((scenario, index) => (
                <button key={scenario.id} className={`pg-scenario ${selected?.id === scenario.id ? "is-selected" : ""}`} aria-pressed={selected?.id === scenario.id} onClick={() => { setMatches(null); void run(scenario); }}>
                  <span className="pg-scenario-top"><span>{scenario.category}</span><span aria-hidden="true">{selected?.id === scenario.id ? <Check size={15} /> : String(index + 1).padStart(2, "0")}</span></span>
                  <span className="pg-scenario-title">{scenario.title}</span>
                  <ArrowRight size={15} className="pg-card-arrow" aria-hidden="true" />
                </button>
              ))}
            </div>
            <div className="pg-manual">
              <label htmlFor="pg-problem">または、課題を入力する</label>
              <form onSubmit={event => { event.preventDefault(); findScenario(); }}>
                <Textarea id="pg-problem" value={input} maxLength={500} onChange={event => { setInput(event.target.value); setMatches(null); }} placeholder="例：新しい人を採用するか迷っています" aria-describedby="pg-input-help" />
                <div className="pg-input-bottom"><p id="pg-input-help">キーワードから、対応する例題を探します。</p><Button type="submit" variant="outline" disabled={!input.trim()}>例題を探す <ArrowRight aria-hidden="true" /></Button></div>
              </form>
              {matches !== null && <div className="pg-followup" role="status"><CircleHelp size={18} aria-hidden="true" /><div><strong>{matches.length ? "どの状況に近いですか？" : "判断する状況を、もう少し教えてください。"}</strong><p>{matches.length ? "入力文から条件は推測しません。近い例題を選び、表示された条件を調整してください。" : "このデモは上の例題に対応しています。採用・価格などの言葉で探すか、近い課題を選んでください。"}</p>{matches.map(scenario => <Button key={scenario.id} variant="outline" onClick={() => { setMatches(null); void run(scenario); }}>{scenario.title}<ArrowRight aria-hidden="true" /></Button>)}</div></div>}
            </div>
          </section>
          <section className="pg-answer-panel" ref={answer} aria-labelledby="pg-answer-title" aria-busy={busy}>
            <div className="pg-section-heading"><span className="pg-step">02</span><h2 id="pg-answer-title">CEO-DNAの判断</h2><Sparkles size={18} className="pg-answer-icon" aria-hidden="true" /></div>
            {!selected ? <div className="pg-empty"><div className="pg-empty-icon"><Sparkles size={28} aria-hidden="true" /></div><h3>迷いに、判断の軸を。</h3><p>気になる課題を選ぶと、<br />判断・理由・判断が変わる条件を表示します。</p><span><ArrowDown size={14} aria-hidden="true" /> まずは、ひとつ選んでみてください</span></div> : <>
              <div className="pg-result" aria-live="polite" aria-atomic="true">
                {busy && <p className="pg-loading">判断を計算しています…</p>}
                {error && <div role="alert"><p>{error}</p><Button variant="outline" onClick={() => { void run(selected, overrides); }}>もう一度試す</Button></div>}
                {result && <><span className="pg-result-label">この条件での判断</span><h3 className="pg-decision">{result.decision}</h3><div className="pg-reasons"><h4>そう判断する理由</h4><ul>{result.rationale.map((reason, index) => <li key={index}><Check size={16} aria-hidden="true" /><span>{reason}</span></li>)}</ul></div><div className="pg-reversal"><h4>判断が変わるのは？</h4><ul>{result.reversalConditions.map((condition, index) => <li key={index}>{condition}</li>)}</ul></div><div className="pg-runtime"><Clock3 size={13} aria-hidden="true" /><span>判断の計算時間（実測） {result.elapsedMs.toLocaleString("ja-JP", { maximumFractionDigits: 2 })} ms</span></div></>}
              </div>
              <div className="pg-context"><span>今回の条件</span><h3>{selected.title}</h3><p>{selected.description}</p>
                {!!selected.conditions?.length && <details className="pg-controls"><summary>条件を変えて試す</summary>{selected.conditions.map(condition => <label key={condition.key}><span>{condition.label}<strong>{formatCondition(overrides[condition.key] ?? condition.value, condition.unit)}</strong></span><input type="range" aria-valuetext={formatCondition(overrides[condition.key] ?? condition.value, condition.unit)} min={condition.min} max={condition.max} step={condition.step} value={overrides[condition.key] ?? condition.value} onChange={event => { void run(selected, { ...overrides, [condition.key]: Number(event.target.value) }); }} /><span className="pg-range-bounds"><span>{formatCondition(condition.min, condition.unit)}</span><span>{formatCondition(condition.max, condition.unit)}</span></span></label>)}<p className="pg-controls-hint">条件を変えると、判断も再計算されます。</p></details>}
              </div>
            </>}
          </section>
        </div>
        <footer className="pg-footer"><span>CEO-DNA</span><p>架空の経営者の判断傾向を体験するデモです。実際の経営判断を保証するものではありません。</p></footer>
      </main>
    </div>
  );
}
