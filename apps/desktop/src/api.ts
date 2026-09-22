export type Candidate = {
  id: string;
  text: string;
  attributes: Record<string, number>;
};
export type Response = {
  response_type: string;
  candidate_id?: string;
  candidate_ids?: string[];
  left_id?: string;
  right_id?: string;
  outcome?: string;
  fields?: string[];
};
export type Proposal = {
  schema_version: string;
  request_id: string;
  workspace_id: string;
  family_id: string;
  case_kind: string;
  domain: string;
  context: {
    as_of: string;
    summary: string;
    facts: {
      key: string;
      value: number;
      evidence_status: string;
      unit: string;
    }[];
    unknown_fields: string[];
  };
  candidates: Candidate[];
  response: Response;
  source: {
    artifact_id: string;
    source_kind: string;
    occurred_at: string | null;
    observed_at: string;
  };
  rationale_explicit: string | null;
  reversal_conditions: string[];
  model_exposure: boolean;
};
export type Memory = {
  id: string;
  revision: number;
  status: string;
  workspace_id: string;
  payload: Proposal;
};
export type Model = {
  id: string;
  state: string;
  created_at: string;
  evaluation: Record<string, unknown>;
  counts: Record<string, unknown>;
  lineage: { id: string; revision: number }[];
};
export type Ranking = {
  selected_candidate_id?: string | null;
  abstained?: boolean;
  abstention_reasons?: string[];
  execution_authorization: string;
  model_status?: string;
  ranking: {
    candidate_id: string;
    raw_score: number | null;
    probability: number | null;
    memory_match?: boolean;
  }[];
  evidence?: { record_id: string; revision: number }[];
  probability?: number | null;
};
export const staticMode = import.meta.env.VITE_CDNA_STATIC === "true";
let session =
  new URLSearchParams(window.location.hash.slice(1)).get("session") || "";
if (window.location.hash)
  window.history.replaceState(
    null,
    "",
    window.location.pathname + window.location.search,
  );
export const hasSession = () => staticMode || !!session;
export const clearSession = () => {
  session = "";
};
export async function command<T>(
  operation: string,
  fields: Record<string, unknown> = {},
): Promise<T> {
  if (staticMode) {
    const adapter = await import("./static/adapter");
    return adapter.command<T>(operation, fields);
  }
  if (!session)
    throw new Error(
      "接続用のセッションがありません。起動時に表示された専用URLを開いてください。",
    );
  const response = await fetch("/api/command", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${session}`,
    },
    body: JSON.stringify({ operation, ...fields }),
    cache: "no-store",
    credentials: "omit",
    signal: AbortSignal.timeout(operation === "train" ? 45000 : 15000),
  });
  if (!response.ok)
    throw new Error(
      `接続できませんでした (${response.status})。ローカルアプリの起動状態を確認してください。`,
    );
  const envelope = await response.json();
  if (!envelope.ok)
    throw new Error(
      `${envelope.error?.message || "操作を完了できませんでした。"} [${envelope.error?.code || "UNKNOWN"}]`,
    );
  return envelope.result as T;
}
export function rankRequest(p: Proposal) {
  return {
    schema_version: "1.0",
    request_id: crypto.randomUUID(),
    workspace_id: p.workspace_id,
    mode: "imitate",
    domain: p.domain,
    context: p.context,
    candidates: p.candidates,
    include_evidence: true,
    allow_cloud: false,
  };
}
