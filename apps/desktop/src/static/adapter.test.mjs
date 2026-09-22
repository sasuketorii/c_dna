// State-machine regression tests use real Rust WASM validation. Deferred learner
// reports below exercise storage/races only, not learning correctness acceptance.
import { test, expect, mock } from 'bun:test';
import { readFile } from 'node:fs/promises';
const domain = await import('../../../../crates/cdna-browser/assets/domain/cdna_browser.js');
domain.initSync({ module: await readFile(new URL('../../../../crates/cdna-browser/assets/domain/cdna_browser_bg.wasm', import.meta.url)) });
let resolveTrain;
let rejectTrain;
let resetCount = 0;
let previewRows;
mock.module('../../../../crates/cdna-browser/runtime', () => ({
  call: async (operation, payload) => {
    if (operation.startsWith('validate_')) return JSON.parse(domain[operation](JSON.stringify(payload)));
    if (operation === 'train') return new Promise((resolve, reject) => { resolveTrain = resolve; rejectTrain = reject; });
    return { ranking: previewRows, probability: 0.99, pairwise_probability: 0.99, execution_authorization: 'execute', unexpected: 'must not escape' };
  },
  reset: () => { resetCount++; rejectTrain?.(new Error('cancelled')); },
}));
const { command } = await import('./adapter.ts');
const uuid = () => crypto.randomUUID();
const report = { ok: true, model: { feature_version: '1.0', weights: [1] }, counts: {}, evaluation: {} };
const tick = () => new Promise(resolve => setTimeout(resolve, 0));
test('ephemeral command state, stale snapshots, output allowlist and lock', async () => {
  const { workspaces: [workspace_id] } = await command('status');
  const p = { schema_version: '1.0', request_id: uuid(), workspace_id, family_id: uuid(), case_kind: 'hypothetical', domain: 'product_delivery', context: { as_of: '2026-09-22T00:00:00Z', summary: '架空の比較', facts: [], unknown_fields: [] }, candidates: [{ id: 'a', text: 'A', attributes: {} }, { id: 'b', text: 'B', attributes: {} }], response: { response_type: 'choose_one', candidate_id: 'a' }, source: { artifact_id: uuid(), source_kind: 'human_app', occurred_at: null, observed_at: '2026-09-22T00:00:00Z' }, rationale_explicit: null, reversal_conditions: [], model_exposure: false };
  const first = await command('propose', { proposal: p });
  expect(first.status).toBe('pending'); expect(first.revision).toBe(1);
  expect(await command('propose', { proposal: p })).toEqual(first);
  await expect(command('propose', { proposal: { ...p, response: { response_type: 'skip' } } })).rejects.toThrow('CONFLICT');
  const request_id = uuid();
  const confirmed = await command('confirm', { workspace_id, id: first.id, expected_revision: 1, request_id });
  expect(confirmed.status).toBe('confirmed'); expect(confirmed.revision).toBe(2);
  expect(await command('confirm', { workspace_id, id: first.id, expected_revision: 1, request_id })).toEqual(confirmed);
  await expect(command('confirm', { workspace_id, id: first.id, expected_revision: 1, request_id: uuid() })).rejects.toThrow('REVISION_CONFLICT');
  const training = command('train', { workspace_id }); await tick();
  const revised = await command('revise', { workspace_id, id: first.id, expected_revision: 2, request_id: uuid(), proposal: { ...p, response: { response_type: 'choose_one', candidate_id: 'b' } } });
  expect(revised.status).toBe('pending'); expect(revised.revision).toBe(3);
  resolveTrain(report); await expect(training).rejects.toThrow('MODEL_INVALIDATED');
  expect((await command('models', { workspace_id })).models).toEqual([]);
  await command('confirm', { workspace_id, id: first.id, expected_revision: 3, request_id: uuid() });
  const fresh = command('train', { workspace_id }); await tick(); resolveTrain(report); const model = await fresh;
  const request = { schema_version: '1.0', request_id: uuid(), workspace_id, domain: p.domain, context: p.context, candidates: p.candidates, mode: 'imitate', include_evidence: true, allow_cloud: false };
  previewRows = [{ candidate_id: 'a', raw_score: 1, probability: 0.98 }, { candidate_id: 'b', raw_score: 0, probability: 0.02 }];
  const preview = await command('preview_model', { workspace_id, id: model.id, request });
  expect(preview.probability).toBeNull(); expect(preview.pairwise_probability).toBeNull(); expect(preview.ranking.every(r => r.probability === null)).toBe(true); expect(preview.execution_authorization).toBe('none'); expect(preview.unexpected).toBeUndefined();
  previewRows = [{ candidate_id: 'a', raw_score: 1 }, { candidate_id: 'a', raw_score: 0 }];
  await expect(command('preview_model', { workspace_id, id: model.id, request })).rejects.toThrow('MODEL_NOT_READY');
  await command('delete', { workspace_id, id: first.id, expected_revision: 4 });
  expect((await command('list', { workspace_id })).records).toEqual([]); expect((await command('models', { workspace_id })).models).toEqual([]);
  await expect(command('propose', { proposal: p })).rejects.toThrow('INPUT_INVALID');
  const next = await command('propose', { proposal: { ...p, request_id: uuid() } });
  await command('confirm', { workspace_id, id: next.id, expected_revision: 1, request_id: uuid() });
  const pending = command('train', { workspace_id }); const cancelled = pending.then(() => 'unexpected-success', e => e.message); await tick();
  expect(await command('lock')).toEqual({ locked: true }); expect(await cancelled).toBe('cancelled');
  expect(resetCount).toBe(1); const status = await command('status'); expect(status.locked).toBe(true); expect(status.workspaces).toEqual([]);
  await expect(command('list', { workspace_id })).rejects.toThrow('VAULT_LOCKED');
});
