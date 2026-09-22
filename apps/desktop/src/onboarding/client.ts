export type Provider = "chat_gpt" | "claude";
export type Speaker = "human" | "assistant" | "system" | "tool" | "unknown";
export type ImportedSource = {
  source_id: string;
  provider: Provider;
  conversation_id: string | null;
  message_id: string | null;
  speaker: Speaker;
  speaker_raw: string;
  text: string;
  source_pointer: string;
  quote_range: [number, number];
  occurred_at: string | null;
  timestamp_certainty: string;
  extractor_version: string;
  artifact_hash: string;
  review_pending: true;
  training_eligible: false;
};
export type ImportBatch = {
  sources: ImportedSource[];
  issues: { source_pointer: string; code: string }[];
  duplicates: number;
  skipped_conversations: number;
  complete: boolean;
  artifact_hash: string;
  extractor_version: string;
};
export type Transport = <T>(operation: string, fields?: Record<string, unknown>) => Promise<T>;
export type SourceDocument = {
  id: string;
  workspace_id: string;
  kind: "source";
  revision: number;
  payload: ImportedSource;
};
export const MAX_WEB_BYTES = 256 * 1024;
const encoder = new TextEncoder();

// Only validate the transport envelope here. Rust owns format parsing and provenance.
export function importFields(workspace_id: string, provider: Provider, export_json: string, preview: boolean) {
  if (provider !== "chat_gpt" && provider !== "claude") throw new Error("対応していない提供元です。");
  if (!export_json || encoder.encode(export_json).length > MAX_WEB_BYTES) throw new Error("Web 取り込みは 256 KiB 未満の JSON が対象です。大きいファイルは CLI（最大 16 MiB）を使用してください。");
  let parsed: unknown;
  try { parsed = JSON.parse(export_json); } catch { throw new Error("JSON を読み取れません。ZIP ではなく会話の JSON を選択してください。"); }
  if (!Array.isArray(parsed) || parsed.length === 0) throw new Error("会話を含む JSON 配列が必要です。");
  const fields = { workspace_id, provider, export_json, preview };
  if (encoder.encode(JSON.stringify({ operation: "import_sources", ...fields })).length > MAX_WEB_BYTES) throw new Error("送信時のサイズが 256 KiB を超えます。小さい JSON または CLI を使用してください。");
  return fields;
}

export function assertSourceOnly(batch: ImportBatch): ImportBatch {
  if (!Array.isArray(batch.sources) || !Array.isArray(batch.issues) || batch.sources.some(source =>
    source.review_pending !== true || source.training_eligible !== false ||
    !["human", "assistant", "system", "tool", "unknown"].includes(source.speaker)
  )) throw new Error("出典の確認状態が不正です。取り込み結果を表示できません。");
  return batch;
}

export function createOnboardingClient(transport: Transport, sessionOnly: boolean) {
  return {
    createWorkspace: (name: string) => transport<{ id: string; name: string }>("workspace_create", { name: name.trim() }),
    sources: (workspace_id: string, offset = 0) => transport<{ sources: SourceDocument[]; offset: number; limit: number }>("sources", { workspace_id, offset }),
    sourceGet: (workspace_id: string, id: string) => transport<SourceDocument>("source_get", { workspace_id, id }),
    sourceDelete: (workspace_id: string, id: string, expected_revision: number) => transport<{ deleted: string }>("source_delete", { workspace_id, id, expected_revision }),
    async importSources(workspaceId: string, provider: Provider, json: string, preview: boolean) {
      if (sessionOnly) throw new Error("静的モードの取り込みは未対応です。ローカルバックエンドで開いてください。");
      return assertSourceOnly(await transport<ImportBatch>("import_sources", importFields(workspaceId, provider, json, preview)));
    },
  };
}

// Invalidates local file reads and preview responses; it does not undo a sent save.
export class PreviewGeneration {
  private generation = 0;
  next() { return ++this.generation; }
  isCurrent(token: number) { return token === this.generation; }
  cancel() { this.generation++; }
}
