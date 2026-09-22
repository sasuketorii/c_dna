import type { Memory, Model, Proposal, Ranking } from '../api';
import { call, reset } from '../../../../crates/cdna-browser/runtime';
import { validateProposal, validateRank } from './validation';


const workspaces = new Map<string, string>([[crypto.randomUUID(), 'ブラウザ体験用']]);
const records = new Map<string, Memory>();
const models = new Map<string, Model>();
const requests = new Map<string, { signature: string; result: Memory }>();
const deleted = new Set<string>();
let epoch = 0;
let locked = false;
let training = false;
let historyBytes = 0;
const domains = ['resource_allocation', 'product_delivery', 'customer_commercial', 'organization_delegation', 'growth_strategy', 'risk_reputation'];
const fail = (code: string): never => { throw new Error(`操作を完了できませんでした。入力と状態を確認してください。 [${code}]`); };
const copy = <T>(value: T): T => structuredClone(value);
function workspace(value: unknown): string { if (typeof value !== 'string' || !workspaces.has(value)) fail('SCOPE_DENIED'); return value as string; }
function uuid(value: unknown): string { if (typeof value !== 'string' || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value)) fail('INPUT_INVALID'); return value as string; }
function invalidate() { epoch++; models.clear(); }
function get(w: string, id: unknown) { const record = records.get(uuid(id)); if (!record || record.workspace_id !== w) fail('INPUT_INVALID'); return record as Memory; }
function stable(value: unknown): string { if (Array.isArray(value)) return `[${value.map(stable).join(',')}]`; if (value && typeof value === 'object') return `{${Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).map(([k,v]) => `${JSON.stringify(k)}:${stable(v)}`).join(',')}}`; return JSON.stringify(value); }
function write(w: string, id: string, expected: number, requestId: string, payload: Proposal | undefined, confirm: boolean) {
  if (deleted.has(id)) fail('INPUT_INVALID');
  const signature = stable({ id, expected, payload: payload ?? null, confirm });
  const key = `${w}:${requestId}`; const prior = requests.get(key);
  if (prior) { if (prior.signature !== signature) fail('CONFLICT'); return prior.result; }
  const signatureBytes = new TextEncoder().encode(signature).length;
  if (requests.size >= 5000 || historyBytes + signatureBytes > 8 * 1024 * 1024) fail('BUDGET_EXCEEDED');
  const old = records.get(id);
  if (old && old.workspace_id !== w) fail('SCOPE_DENIED');
  if (!Number.isSafeInteger(expected) || expected < 0 || (old?.revision ?? 0) !== expected) fail('REVISION_CONFLICT');
  if (!old && records.size >= 500) fail('BUDGET_EXCEEDED');
  if (confirm && (!old || old.status !== 'pending' || old.payload.source.source_kind === 'ai_generated')) fail('INPUT_INVALID');
  const result: Memory = { id, workspace_id: w, revision: expected + 1, status: confirm ? 'confirmed' : 'pending', payload: copy(payload ?? old!.payload) };
  historyBytes += signatureBytes; records.set(id, result); requests.set(key, { signature, result }); invalidate(); return result;
}
async function execute(operation: string, fields: Record<string, unknown>): Promise<unknown> {
  if (operation === 'status') return { schema_version: '1.0', workspaces: [...workspaces.keys()], workspace_details: [...workspaces], demo: true, locked, cloud_enabled: false, personal_accuracy: 'unverified', storage: 'ephemeral_browser_memory', capabilities: ['observations', 'confirmation', 'revisions', 'ranking', 'learning', 'export'] };
  if (operation === 'lock') { locked = true; reset(); training = false; records.clear(); models.clear(); requests.clear(); deleted.clear(); workspaces.clear(); historyBytes = 0; epoch++; return { locked: true }; }
  if (locked) fail('VAULT_LOCKED');
  if (operation === 'workspace_create') { const name = fields.name; if (typeof name !== 'string' || !name.trim() || new TextEncoder().encode(name).length > 256) fail('INPUT_INVALID'); if (workspaces.size >= 16) fail('BUDGET_EXCEEDED'); const id = crypto.randomUUID(); workspaces.set(id, name as string); return { id, name }; }
  if (operation === 'propose') { const p = await validateProposal(fields.proposal); if (locked) fail('VAULT_LOCKED'); return write(workspace(p.workspace_id), uuid(p.request_id), 0, uuid(p.request_id), p, false); }
  if (operation === 'rank') { const request = await validateRank(fields.request); workspace(request.workspace_id); if (locked) fail('VAULT_LOCKED'); return rank(request); }
  const w = workspace(fields.workspace_id);
  const rows = () => [...records.values()].filter(r => r.workspace_id === w).sort((a,b) => a.id.localeCompare(b.id));
  const offset = fields.offset ?? 0;
  if (!Number.isSafeInteger(offset) || Number(offset) < 0 || Number(offset) > 1000000) fail('INPUT_INVALID');
  switch (operation) {
    case 'list': return { records: rows().slice(Number(offset), Number(offset) + 100), offset, limit: 100 };
    case 'confirm': if (typeof fields.expected_revision !== 'number') fail('INPUT_INVALID'); return write(w, uuid(fields.id), fields.expected_revision as number, uuid(fields.request_id), undefined, true);
    case 'revise': { const p = await validateProposal(fields.proposal); if (locked) fail('VAULT_LOCKED'); if (p.workspace_id !== w) fail('SCOPE_DENIED'); if (typeof fields.expected_revision !== 'number' || !fields.expected_revision) fail('INPUT_INVALID'); return write(w, uuid(fields.id), fields.expected_revision as number, uuid(fields.request_id), p, false); }
    case 'delete': { const r = get(w, fields.id); if (r.revision !== fields.expected_revision) fail('REVISION_CONFLICT'); if (deleted.size >= 5000) fail('BUDGET_EXCEEDED'); records.delete(r.id); deleted.add(r.id); for (const [key,v] of requests) if (v.result.id === r.id) { historyBytes -= new TextEncoder().encode(v.signature).length; requests.delete(key); } invalidate(); return { deleted: r.id, epoch }; }
    case 'gaps': return { domains: domains.map(domain => { const n = rows().filter(r => r.status === 'confirmed' && r.payload.domain === domain).length; return { domain, confirmed_records: n, status: n ? 'recorded_unverified' : 'unlearned', accuracy: null }; }), counts_truncated: false };
    case 'export': return { format: 'cdna-jsonl-1', sensitive: true, jsonl: rows().slice(Number(offset), Number(offset) + 100).map(r => JSON.stringify(r)).join('\n'), offset, limit: 100 };
    case 'models': return { models: [...models.values()].filter(m => (m as Model & { workspace_id: string }).workspace_id === w) };
    case 'train': {
      if (training) fail('RATE_LIMITED'); const snapshot = copy(rows().filter(r => r.status === 'confirmed' && !r.payload.model_exposure && r.payload.source.source_kind !== 'ai_generated'));
      if (!snapshot.length) fail('MODEL_NOT_READY'); const generation = epoch; training = true;
      try { const eligible = snapshot.filter(r => ['choose_one', 'choose_set', 'pairwise', 'acceptability'].includes(r.payload.response.response_type));
        if (!eligible.length) fail('MODEL_NOT_READY');
        const input = eligible.map(({ payload: p }) => {
          const response = p.response;
          const base = { family_id: p.family_id, timestamp: p.source.observed_at, domain: p.domain, context: p.context, candidates: p.candidates, verification_state: 'confirmed', model_exposure: false, case_kind: p.case_kind, answer_kind: response.response_type };
          if (response.response_type === 'choose_one') return { ...base, chosen_ids: [response.candidate_id] };
          if (response.response_type === 'choose_set') return { ...base, chosen_ids: response.candidate_ids };
          if (response.response_type === 'pairwise') return { ...base, left_id: response.left_id, right_id: response.right_id, preference: response.outcome, chosen_ids: response.outcome === 'left' ? [response.left_id] : response.outcome === 'right' ? [response.right_id] : [] };
          return { ...base, chosen_ids: [response.candidate_id], acceptability_outcome: response.outcome, conditions: p.reversal_conditions };
        });
        const report = await call('train', { operation: 'train', feature_version: '1.0', records: input });
        if (report.ok !== true) fail('MODEL_NOT_READY');
        const model = { ...report, id: crypto.randomUUID(), workspace_id: w, epoch: generation, state: 'provisional', created_at: new Date().toISOString(), lineage: eligible.map(r => ({ id: r.id, revision: r.revision })) }; if (locked || generation !== epoch) fail('MODEL_INVALIDATED'); if (!model || model.state !== 'provisional') fail('MODEL_NOT_READY'); models.clear(); models.set(model.id, model); return model; } finally { training = false; }
    }
    case 'preview_model': { const request = await validateRank(fields.request); if (locked) fail('VAULT_LOCKED'); if (request.workspace_id !== w) fail('SCOPE_DENIED'); const model = models.get(uuid(fields.id)); if (!model || (model as Model & { workspace_id: string }).workspace_id !== w) fail('MODEL_INVALIDATED'); const generation = epoch; const result = await call('preview', { model: (model as Model & { model: unknown }).model, request }); if (locked || epoch !== generation) fail('MODEL_INVALIDATED'); if (!Array.isArray(result.ranking) || result.ranking.length !== request.candidates.length) fail('MODEL_NOT_READY');
      const seen = new Set<string>();
      const ranking = result.ranking.map((row: { candidate_id?: unknown; raw_score?: unknown }) => {
        if (typeof row.candidate_id !== 'string' || !request.candidates.some(c => c.id === row.candidate_id) || seen.has(row.candidate_id) || typeof row.raw_score !== 'number' || !Number.isFinite(row.raw_score)) fail('MODEL_NOT_READY');
        seen.add(row.candidate_id as string);
        return { candidate_id: row.candidate_id, raw_score: row.raw_score, probability: null, policy_status: 'unknown' };
      });
      return { model_id: model!.id, ranking, provisional: true, execution_authorization: 'none', probability: null, pairwise_probability: null, abstained: true, abstention_reasons: ['model_not_independently_validated'], mode: 'experimental_preview' }; }
    case 'activate_model': return fail('MODEL_NOT_READY');
    default: return fail('INPUT_INVALID');
  }
}
type Request = Awaited<ReturnType<typeof validateRank>>;
function rank(request: Request): Ranking {
  const signature = (cs: Proposal['candidates']) => stable(cs.map(c => stable({ text: c.text, attributes: c.attributes })).sort());
  const winners = new Set<string>(); const evidence: { record_id: string; revision: number }[] = []; let ambiguous = false;
  for (const r of records.values()) {
    const p = r.payload;
    if (r.workspace_id !== request.workspace_id || r.status !== 'confirmed' || p.domain !== request.domain || p.context.summary !== request.context.summary || stable(p.context.facts) !== stable(request.context.facts) || stable(p.context.unknown_fields) !== stable(request.context.unknown_fields) || signature(p.candidates) !== signature(request.candidates)) continue;
    if (p.response.response_type !== 'choose_one') continue;
    const chosen = p.candidates.find(c => c.id === p.response.candidate_id); if (!chosen) continue;
    const matches = request.candidates.filter(c => c.text === chosen.text && stable(c.attributes) === stable(chosen.attributes));
    if (matches.length !== 1) ambiguous = true;
    for (const c of matches) winners.add(c.id);
    if (evidence.length < 20) evidence.push({ record_id: r.id, revision: r.revision });
  }
  const reasons: string[] = [];
  if (ambiguous) reasons.push('ambiguous_candidates');
  if (request.context.unknown_fields.length || request.context.facts.some(f => f.evidence_status !== 'explicit')) reasons.push('missing_information');
  if (winners.size > 1) reasons.push('conflicting_evidence');
  if (!evidence.length) reasons.push('no_matching_confirmed_memory');
  if (request.mode === 'advisor') reasons.push('advisor_requires_separate_evidence');
  const selected = reasons.length ? null : [...winners][0] ?? null;
  return { ranking: request.candidates.map(c => ({ candidate_id: c.id, raw_score: null, probability: null, memory_match: c.id === selected })), selected_candidate_id: selected, evidence: request.include_evidence ? evidence : [], abstained: reasons.length > 0, abstention_reasons: reasons, execution_authorization: 'none', model_status: 'memory_only', probability: null };
}
export async function command<T>(operation: string, fields: Record<string, unknown> = {}): Promise<T> {
  const text = JSON.stringify(fields, (_key, value: unknown) => { if (typeof value === 'number' && !Number.isFinite(value)) fail('INPUT_INVALID'); return value; });
  if (new TextEncoder().encode(text).length > 256 * 1024) fail('INPUT_INVALID');
  return copy(await execute(operation, JSON.parse(text))) as T;
}
