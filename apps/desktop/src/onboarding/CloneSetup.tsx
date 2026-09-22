import { useEffect, useRef, useState } from "react";
import { command, staticMode } from "../api";
import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";
import { Label } from "../components/ui/label";
import { Badge } from "../components/ui/badge";
import { createOnboardingClient, MAX_WEB_BYTES, PreviewGeneration } from "./client";
import type { ImportBatch, Provider, Speaker } from "./client";

const api = createOnboardingClient(command, staticMode);
const roleNames: Record<Speaker, string> = { human: "人間（未確認）", assistant: "AI", system: "システム", tool: "ツール", unknown: "不明" };
export type CloneSetupProps = {
  workspaceId?: string;
  onWorkspaceCreated?: (workspace: { id: string; name: string }) => void;
  onImported?: (batch: ImportBatch) => void;
};

export function CloneSetup(props: CloneSetupProps) {
  return <SetupFlow key={props.workspaceId || "new"} {...props} />;
}

function SetupFlow({ workspaceId, onWorkspaceCreated, onImported }: CloneSetupProps) {
  const [workspace, setWorkspace] = useState(workspaceId || "");
  const [name, setName] = useState("");
  const [provider, setProvider] = useState<Provider>("chat_gpt");
  const [json, setJson] = useState("");
  const [filename, setFilename] = useState("");
  const [batch, setBatch] = useState<ImportBatch | null>(null);
  const [busy, setBusy] = useState<"create" | "preview" | "save" | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [page, setPage] = useState(0);
  const generation = useRef(new PreviewGeneration());
  const fileInput = useRef<HTMLInputElement>(null);
  const mutation = useRef(false);
  useEffect(() => () => generation.current.cancel(), []);

  function reset() {
    generation.current.cancel();
    setJson(""); setFilename(""); setBatch(null); setPage(0); setError("");
    if (fileInput.current) fileInput.current.value = "";
  }
  async function readFile(file: File) {
    reset(); setNotice("");
    const token = generation.current.next();
    setBusy("preview");
    try {
      if (!file.name.toLowerCase().endsWith(".json")) throw new Error("ZIP ではなく .json ファイルを選択してください。");
      if (file.size > MAX_WEB_BYTES) throw new Error("Web 取り込みは最大 256 KiB です。大きいファイルは CLI（最大 16 MiB）を使用してください。");
      const text = await file.text();
      if (!generation.current.isCurrent(token)) return;
      const preview = await api.importSources(workspace, provider, text, true);
      if (!generation.current.isCurrent(token)) return;
      setJson(text); setFilename(file.name); setBatch(preview);
    } catch (cause) {
      if (generation.current.isCurrent(token)) setError(cause instanceof Error ? cause.message : "読み取りに失敗しました。");
    } finally { if (generation.current.isCurrent(token)) setBusy(null); }
  }
  async function create() {
    if (mutation.current || !name.trim()) return;
    mutation.current = true; setBusy("create"); setError("");
    try {
      const created = await api.createWorkspace(name);
      setWorkspace(created.id);
      onWorkspaceCreated?.(created);
    } catch (cause) { setError(cause instanceof Error ? cause.message : "作成できませんでした。"); }
    finally { mutation.current = false; setBusy(null); }
  }
  async function save() {
    if (mutation.current || !batch || !json) return;
    mutation.current = true; setBusy("save"); setError("");
    try {
      const saved = await api.importSources(workspace, provider, json, false);
      reset(); setNotice(`${saved.sources.length} 件の出典を保存しました。確認済み回答・学習データには追加していません。`);
      onImported?.(saved);
    } catch (cause) {
      // A network failure does not prove rollback. Do not offer an automatic retry.
      reset();
      setError(`${cause instanceof Error ? cause.message : "保存結果を確認できません。"} 保存済みの出典を確認してから再度取り込んでください。`);
    } finally { mutation.current = false; setBusy(null); }
  }
  return <section className="flex flex-col gap-4 rounded-xl border p-5" aria-label="クローンの初期設定" aria-busy={busy !== null}>
    <div><h2 className="text-lg font-semibold">会話からクローンの準備を始める</h2>
      <p className="text-sm text-muted-foreground">1. ワークスペースを作成 → 2. JSON を選択 → 3. 出典を確認して保存</p></div>
    {staticMode && <p role="note" className="text-sm">静的モードはこのセッション内のみです。再読み込みでデータは消えます。会話の取り込みはローカルバックエンドで利用してください。</p>}
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    {notice && <p role="status" className="text-sm">{notice}</p>}
    {!workspace ? <form className="flex flex-col gap-3" onSubmit={event => { event.preventDefault(); void create(); }}>
      <Label htmlFor="clone-name">クローンの名前</Label>
      <Input id="clone-name" value={name} onChange={event => setName(event.target.value)} maxLength={80} placeholder="マイワークスペース" disabled={busy !== null} />
      <Button type="submit" disabled={busy !== null || !name.trim()} className="self-start">{busy === "create" ? "作成中…" : "ワークスペースを作成"}</Button>
    </form> : <>
      <p className="text-sm">ワークスペースの準備ができました。取り込みは任意です。</p>
      <Label htmlFor="clone-provider">エクスポート元</Label>
      <select id="clone-provider" className="rounded-md border bg-background p-2 text-sm" value={provider} disabled={staticMode || busy !== null} onChange={event => { reset(); setNotice(""); setProvider(event.target.value as Provider); }}>
        <option value="chat_gpt">ChatGPT</option><option value="claude">Claude</option>
      </select>
      <Label htmlFor="clone-export">会話 JSON（送信データ全体で最大 256 KiB）</Label>
      <Input id="clone-export" ref={fileInput} type="file" accept=".json,application/json" disabled={staticMode || busy !== null} onChange={event => { const file = event.target.files?.[0]; if (file) void readFile(file); }} />
      <p className="text-sm text-muted-foreground">対応形式: ChatGPT の mapping / Claude の chat_messages を含む会話配列。ZIP・画像・添付ファイルには対応していません。認証情報や秘密情報を含まないファイルを選んでください。</p>
      {busy === "preview" && <Button variant="outline" className="self-start" onClick={() => { reset(); setBusy(null); setNotice("プレビューをキャンセルしました。出典は保存していません。"); }}>プレビューをキャンセル</Button>}
      {batch && <>
        <div className="flex flex-wrap gap-2"><Badge variant="secondary">{filename}</Badge><Badge variant="outline">出典 {batch.sources.length} 件</Badge><Badge variant="outline">重複 {batch.duplicates} 件</Badge></div>
        <p className="text-sm">全発言は未確認の出典です。AI の発言を本人の回答として扱いません。人間の発言も保存だけでは確認済みになりません。</p>
        <details className="text-xs"><summary>解析の来歴</summary><p className="break-all">SHA-256: {batch.artifact_hash}</p><p>解析器: {batch.extractor_version}</p></details>
        {batch.issues.length > 0 && <div role="note" className="text-sm"><p>{batch.issues.length} 件を取り込めませんでした。保存対象は下記の解析済み出典のみです。</p><ul className="list-inside list-disc">{batch.issues.slice(0, 20).map((issue, index) => <li key={index} className="break-all">{issue.source_pointer}: {issue.code}</li>)}</ul>{batch.issues.length > 20 && <p>問題の表示は先頭 20 件です。</p>}</div>}
        <ol className="flex flex-col gap-3">{batch.sources.slice(page * 20, (page + 1) * 20).map(source => <li key={source.source_id} className="rounded-lg border p-3">
          <Badge variant="outline">{roleNames[source.speaker]}</Badge><span className="ml-2 text-xs text-muted-foreground">元の役割: {source.speaker_raw}</span>
          <p className="mt-2 whitespace-pre-wrap break-words text-sm">{source.text.slice(0, 2000)}{source.text.length > 2000 ? "…（表示を省略）" : ""}</p>
          <details className="mt-2 text-xs"><summary>出典の詳細</summary><dl className="break-all"><dt>会話 / メッセージ ID</dt><dd>{source.conversation_id ?? "不明"} / {source.message_id ?? "不明"}</dd><dt>JSON の位置</dt><dd>{source.source_pointer}</dd><dt>引用範囲（本文 UTF-8 バイト）</dt><dd>{source.quote_range.join("–")}</dd><dt>日時</dt><dd>{source.occurred_at ?? "不明"} ({source.timestamp_certainty})</dd></dl></details>
        </li>)}</ol>
        {batch.sources.length > 20 && <div className="flex items-center gap-3"><Button variant="outline" disabled={page === 0 || busy !== null} onClick={() => setPage(page - 1)}>前へ</Button><span className="text-sm">{page + 1} / {Math.ceil(batch.sources.length / 20)}</span><Button variant="outline" disabled={(page + 1) * 20 >= batch.sources.length || busy !== null} onClick={() => setPage(page + 1)}>次へ</Button></div>}
        <div className="flex gap-3"><Button disabled={busy !== null || batch.sources.length === 0} onClick={() => void save()}>{busy === "save" ? "保存中…" : "出典として保存"}</Button><Button variant="outline" disabled={busy !== null} onClick={() => { reset(); setNotice("取り込みをキャンセルしました。出典は保存していません。"); }}>キャンセル</Button></div>
        {busy === "save" && <p role="status" className="text-sm">保存結果を確認しています。この画面を開いたままお待ちください。</p>}
      </>}
    </>}
  </section>;
}
