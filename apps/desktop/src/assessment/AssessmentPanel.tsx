import { useEffect, useRef, useState } from "react";
import { command } from "../api";

const instruments = {
  bfi_2_j: "BFI-2-J", ipip: "IPIP", hexaco_pi_r: "HEXACO-PI-R",
  tipi_j: "TIPI-J", bfi_2: "BFI-2", neo_pi_3: "NEO-PI-3", mbti: "MBTI",
} as const;
type Score = { dimension: string; value: number | null; minimum: number; maximum: number; unit: string; missing_reason: string | null };
export type AssessmentResult = {
  schema_version: "1.0";
  instrument: keyof typeof instruments;
  instrument_version: string;
  locale: string;
  confirmed_by_user: boolean;
  assessed_on: string;
  scores: Score[];
  source: { kind: "manual_result" | "imported_result"; reference: string };
  consent: { store_locally: boolean; mcp_read: false; external_send: false };
  validity: "user_reported_unverified";
};
type Document = { id: string; revision: number; payload: AssessmentResult };
const emptyScore = (): Score => ({ dimension: "", value: null, minimum: 0, maximum: 1, unit: "", missing_reason: "" });
const emptyResult = (): AssessmentResult => ({ schema_version: "1.0", instrument: "bfi_2_j", instrument_version: "", locale: "ja-JP", confirmed_by_user: false, assessed_on: "", scores: [emptyScore()], source: { kind: "manual_result", reference: "" }, consent: { store_locally: false, mcp_read: false, external_send: false }, validity: "user_reported_unverified" });
const inputClass = "w-full rounded-md border border-input bg-background px-3 py-2 text-sm";
const buttonClass = "rounded-md border px-3 py-2 text-sm disabled:opacity-50";

/** Import shape checks only. The Rust validator is authoritative before every write. */
export function parseAssessmentImport(text: string): AssessmentResult {
  if (new TextEncoder().encode(text).length > 32768) throw new Error("結果JSONは32 KiB以下にしてください。");
  const v = JSON.parse(text) as AssessmentResult;
  if (!v || typeof v !== "object" || v.schema_version !== "1.0" || !Object.hasOwn(instruments, v.instrument) ||
      v.validity !== "user_reported_unverified" || !Array.isArray(v.scores) || !v.scores.length || v.scores.length > 64 ||
      typeof v.instrument_version !== "string" || typeof v.locale !== "string" || typeof v.assessed_on !== "string" ||
      !v.source || typeof v.source.reference !== "string" || !["manual_result", "imported_result"].includes(v.source.kind) ||
      !v.consent || typeof v.consent.store_locally !== "boolean" || v.consent.mcp_read !== false || v.consent.external_send !== false ||
      typeof v.confirmed_by_user !== "boolean" ||
      v.scores.some(s => !s || typeof s.dimension !== "string" || typeof s.unit !== "string" ||
        typeof s.minimum !== "number" || typeof s.maximum !== "number" ||
        !(s.value === null || typeof s.value === "number") ||
        !(s.missing_reason == null || typeof s.missing_reason === "string"))) {
    throw new Error("対応する結果形式ではありません。尺度・版・日付・由来・得点範囲を確認してください。");
  }
  // Consent in an imported file never counts as consent from the current user.
  return { ...v, confirmed_by_user: false, source: { ...v.source, kind: "imported_result" }, consent: { store_locally: false, mcp_read: false, external_send: false } };
}

export function AssessmentPanel({ workspaceId }: { workspaceId: string }) {
  const [rows, setRows] = useState<Document[]>([]);
  const [offset, setOffset] = useState(0);
  const [draft, setDraft] = useState<AssessmentResult>(emptyResult);
  const [editing, setEditing] = useState<Document | null>(null);
  const [importText, setImportText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [deleting, setDeleting] = useState<string | null>(null);
  const generation = useRef(0);
  const request = useRef(0);
  const inFlight = useRef(false);
  useEffect(() => {
    const token = ++generation.current;
    const sequence = ++request.current;
    setRows([]); setDraft(emptyResult()); setEditing(null); setImportText(""); setError(""); setNotice(""); setDeleting(null);
    setBusy(true);
    command<{ assessments: Document[] }>("assessments", { workspace_id: workspaceId, offset })
      .then(result => { if (generation.current === token && request.current === sequence) setRows(result.assessments); })
      .catch(() => { if (generation.current === token) setError("結果を読み込めませんでした。"); })
      .finally(() => { if (generation.current === token) setBusy(false); });
    return () => { ++generation.current; };
  }, [workspaceId, offset]);
  const update = <K extends keyof AssessmentResult>(key: K, value: AssessmentResult[K]) => setDraft(d => ({ ...d, [key]: value }));
  const score = (index: number, values: Partial<Score>) => setDraft(d => ({ ...d, scores: d.scores.map((s, i) => i === index ? { ...s, ...values } : s) }));
  async function mutate(action: "save" | "delete", row?: Document) {
    if (inFlight.current) return;
    inFlight.current = true;
    const token = generation.current;
    setBusy(true); setError(""); setNotice("");
    try {
      if (action === "save") await command("assessment_save", { workspace_id: workspaceId, id: editing?.id ?? crypto.randomUUID(), expected_revision: editing?.revision ?? 0, result: draft });
      else if (row) await command("assessment_delete", { workspace_id: workspaceId, id: row.id, expected_revision: row.revision });
      if (token !== generation.current) return;
      setDraft(emptyResult()); setEditing(null); setDeleting(null); setImportText("");
      setNotice(action === "save" ? "結果を保存しました。" : "結果を削除しました。");
      const result = await command<{ assessments: Document[] }>("assessments", { workspace_id: workspaceId, offset });
      if (token === generation.current) setRows(result.assessments);
    } catch {
      if (token === generation.current) setError("操作を完了できませんでした。入力、同意、版の競合を確認し、必要ならページを再読み込みしてください。");
    } finally { inFlight.current = false; if (token === generation.current) setBusy(false); }
  }
  return <section className="space-y-6" aria-label="任意の性格結果">
    <div><h2 className="text-xl font-semibold">性格結果（任意）</h2>
      <p className="text-sm text-muted-foreground">ご自身の結果を原資料どおり保存できます。未登録や削除後も判断記憶・比較学習を利用できます。</p>
      <p className="text-sm text-muted-foreground">質問票は収録未有効です。採点や百分位の生成、判断モデルへの入力は行いません。保存結果の妥当性・本人再現精度は未検証で、医療診断や採用・信用判断には使えません。</p>
    </div>
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    {notice && <p role="status" className="text-sm">{notice}</p>}
    <div className="space-y-3"><h3 className="font-medium">保存した結果</h3>
      {!rows.length && <p className="text-sm">{busy ? "読み込み中…" : "保存結果はありません。"}</p>}
      {rows.map(row => <article key={row.id} className="space-y-2 rounded-lg border p-4">
        <h4 className="font-medium">{instruments[row.payload.instrument]} · {row.payload.instrument_version}</h4>
        <p className="text-sm">実施日: {row.payload.assessed_on} / 言語: {row.payload.locale} / 版: {row.revision}</p>
        <p className="break-words text-sm">由来: {row.payload.source.reference}（{row.payload.source.kind === "manual_result" ? "手動入力" : "JSON取込"}）</p>
        <p className="text-sm">本人持込・妥当性未検証 / ローカル保存のみ / MCP参照なし / 外部送信なし</p>
        <ul className="space-y-1 text-sm">{row.payload.scores.map(s => <li key={s.dimension}>{s.dimension}: {s.value === null ? `欠損（${s.missing_reason}）` : s.value} {s.unit}（元の範囲 {s.minimum}〜{s.maximum}）</li>)}</ul>
        <div className="flex gap-2"><button className={buttonClass} disabled={busy} onClick={() => { setEditing(row); setDraft({ ...row.payload, confirmed_by_user: false, consent: { store_locally: false, mcp_read: false, external_send: false } }); setError(""); }}>編集</button>
          <button className={buttonClass} disabled={busy} onClick={() => setDeleting(row.id)}>削除</button></div>
        {deleting === row.id && <div className="space-x-2"><span className="text-sm">この結果を削除しますか？</span><button disabled={busy} className={buttonClass} onClick={() => void mutate("delete", row)}>削除を確定</button><button className={buttonClass} disabled={busy} onClick={() => setDeleting(null)}>戻る</button></div>}
      </article>)}
      <div className="flex gap-2"><button className={buttonClass} disabled={busy || offset === 0} onClick={() => setOffset(n => Math.max(0, n - 100))}>前へ</button><button className={buttonClass} disabled={busy || rows.length < 100} onClick={() => setOffset(n => n + 100)}>次へ</button></div>
    </div>
    <details className="rounded-lg border p-4"><summary>構造化JSONから入力</summary><p className="my-2 text-sm">下のフォームで作った形式をコピーして利用できます。取込後、内容と保存同意を確認してください。</p>
      <textarea aria-label="結果JSON" className={inputClass} rows={6} maxLength={32768} value={importText} onChange={e => setImportText(e.target.value)} />
      <button className={buttonClass} disabled={busy} onClick={() => { try { setDraft(parseAssessmentImport(importText)); setEditing(null); setError(""); } catch (e) { setError(e instanceof Error ? e.message : "JSONを確認してください。"); } }}>フォームへ取り込む</button>
      <details><summary>現在のフォームのJSON形式</summary><pre className="overflow-auto text-xs">{JSON.stringify(draft, null, 2)}</pre></details>
    </details>
    <form className="space-y-4 rounded-lg border p-4" onSubmit={e => { e.preventDefault(); void mutate("save"); }}>
      <h3 className="font-medium">{editing ? "結果を編集" : "結果を手動入力"}</h3>
      <fieldset disabled={busy} className="space-y-4">
        <div className="grid gap-3 sm:grid-cols-2">
          <label>尺度<select className={inputClass} value={draft.instrument} onChange={e => update("instrument", e.target.value as AssessmentResult["instrument"])}>{Object.entries(instruments).map(([id, label]) => <option value={id} key={id}>{label}</option>)}</select></label>
          <label>原資料の版<input required maxLength={128} className={inputClass} value={draft.instrument_version} onChange={e => update("instrument_version", e.target.value)} /></label>
          <label>実施日<input required type="date" className={inputClass} value={draft.assessed_on} onChange={e => update("assessed_on", e.target.value)} /></label>
          <label>言語<input required maxLength={35} className={inputClass} value={draft.locale} onChange={e => update("locale", e.target.value)} /></label>
        </div>
        <label className="block">出典・結果資料の参照名<input required maxLength={2048} className={inputClass} value={draft.source.reference} onChange={e => update("source", { ...draft.source, reference: e.target.value })} /></label>
        {draft.scores.map((s, index) => <fieldset className="grid gap-2 rounded-md border p-3 sm:grid-cols-2" key={index}><legend>得点 {index + 1}</legend>
          <label>領域・下位尺度名<input required maxLength={128} className={inputClass} value={s.dimension} onChange={e => score(index, { dimension: e.target.value })} /></label>
          <label>原資料の単位<input required maxLength={64} className={inputClass} value={s.unit} onChange={e => score(index, { unit: e.target.value })} /></label>
          <label>範囲の最小値<input required type="number" step="any" className={inputClass} value={s.minimum} onChange={e => score(index, { minimum: e.target.valueAsNumber })} /></label>
          <label>範囲の最大値<input required type="number" step="any" className={inputClass} value={s.maximum} onChange={e => score(index, { maximum: e.target.valueAsNumber })} /></label>
          <label>得点（欠損は空欄）<input type="number" step="any" min={s.minimum} max={s.maximum} className={inputClass} value={s.value ?? ""} onChange={e => score(index, { value: e.target.value === "" ? null : e.target.valueAsNumber, missing_reason: e.target.value === "" ? "" : null })} /></label>
          {s.value === null && <label>欠損理由<input required maxLength={256} className={inputClass} value={s.missing_reason ?? ""} onChange={e => score(index, { missing_reason: e.target.value })} /></label>}
          <button type="button" className={buttonClass} disabled={draft.scores.length === 1} onClick={() => update("scores", draft.scores.filter((_, i) => i !== index))}>この得点を除く</button>
        </fieldset>)}
        <button type="button" className={buttonClass} disabled={draft.scores.length >= 64} onClick={() => update("scores", [...draft.scores, emptyScore()])}>得点を追加</button>
        <label className="flex items-start gap-2"><input type="checkbox" required checked={draft.confirmed_by_user} onChange={e => update("confirmed_by_user", e.target.checked)} />本人の結果と原資料の版・範囲・得点・欠損を確認しました。</label>
        <label className="flex items-start gap-2"><input type="checkbox" required checked={draft.consent.store_locally} onChange={e => update("consent", { store_locally: e.target.checked, mcp_read: false, external_send: false })} />この性格結果をローカル保管庫へ保存することに同意します。後から削除できます。</label>
        <p className="text-sm text-muted-foreground">MCP参照と外部送信はそれぞれ無効です。通常の判断プロフィールとは別に保管します。</p>
        <div className="flex gap-2"><button className={buttonClass} type="submit">{busy ? "処理中…" : "保存"}</button><button type="button" className={buttonClass} onClick={() => { setDraft(emptyResult()); setEditing(null); }}>入力をリセット</button></div>
      </fieldset>
    </form>
  </section>;
}
