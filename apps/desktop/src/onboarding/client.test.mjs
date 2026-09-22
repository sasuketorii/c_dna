import { test, expect } from 'bun:test';
import { createOnboardingClient, importFields, assertSourceOnly, PreviewGeneration, MAX_WEB_BYTES } from './client.ts';

const batch = {
  sources: ['human', 'assistant', 'system', 'tool', 'unknown'].map(speaker => ({
    source_id: speaker, speaker, speaker_raw: speaker, text: 'untrusted',
    review_pending: true, training_eligible: false,
  })),
  issues: [], duplicates: 0, skipped_conversations: 0, complete: true,
  artifact_hash: 'fixture', extractor_version: 'fixture',
};

test('malformed, empty, ZIP, oversized, escaped and multibyte exports fail before dispatch', () => {
  for (const json of ['', 'PK zip', '{', '{}', '[]', ' '.repeat(MAX_WEB_BYTES + 1), JSON.stringify(['あ'.repeat(MAX_WEB_BYTES / 3)])]) {
    expect(() => importFields('w', 'claude', json, true)).toThrow();
  }
  const escaped = JSON.stringify(['"'.repeat(80000)]);
  expect(new TextEncoder().encode(escaped).length).toBeLessThan(MAX_WEB_BYTES);
  expect(() => importFields('w', 'claude', escaped, true)).toThrow();
});

test('preview and save use only import_sources; all roles remain unconfirmed', async () => {
  const calls = [];
  const client = createOnboardingClient(async (operation, fields) => {
    calls.push({ operation, ...fields }); return batch;
  }, false);
  const json = '[{"uuid":"c","chat_messages":[]}]';
  expect(await client.importSources('w', 'claude', json, true)).toBe(batch);
  expect(await client.importSources('w', 'claude', json, false)).toBe(batch);
  expect(calls.map(call => [call.operation, call.preview])).toEqual([['import_sources', true], ['import_sources', false]]);
  expect(calls.every(call => call.export_json === json && call.workspace_id === 'w')).toBe(true);
  for (const speaker of ['human', 'assistant']) {
    expect(() => assertSourceOnly({ ...batch, sources: [{ speaker, review_pending: false, training_eligible: true }] })).toThrow();
  }
  expect(() => assertSourceOnly({ ...batch, sources: [{ speaker: 'administrator', review_pending: true, training_eligible: false }] })).toThrow();
});

test('static mode blocks import without invoking transport', async () => {
  let calls = 0;
  const client = createOnboardingClient(async () => { calls++; return batch; }, true);
  await expect(client.importSources('w', 'claude', '[{}]', true)).rejects.toThrow('静的モード');
  await expect(client.importSources('w', 'claude', '[{}]', false)).rejects.toThrow('静的モード');
  expect(calls).toBe(0);
});

test('cancel and replacement ignore delayed preview results', async () => {
  const generation = new PreviewGeneration();
  const first = generation.next();
  let resolve;
  const delayed = new Promise(done => { resolve = done; });
  let displayed = false;
  const read = delayed.then(() => { if (generation.isCurrent(first)) displayed = true; });
  generation.cancel(); resolve(); await read;
  expect(displayed).toBe(false);
  const second = generation.next();
  const third = generation.next();
  expect(generation.isCurrent(second)).toBe(false);
  expect(generation.isCurrent(third)).toBe(true);
});

test('save failure is surfaced without automatic retries', async () => {
  let calls = 0;
  const client = createOnboardingClient(async () => { calls++; throw new Error('connection lost'); }, false);
  await expect(client.importSources('w', 'claude', '[{}]', false)).rejects.toThrow('connection lost');
  expect(calls).toBe(1);
});

test('source review helpers preserve workspace scope and deletion revision', async () => {
  const calls = [];
  const client = createOnboardingClient(async (operation, fields) => { calls.push({ operation, ...fields }); return {}; }, false);
  await client.sources('w', 100);
  await client.sourceGet('w', 's');
  await client.sourceDelete('w', 's', 3);
  expect(calls).toEqual([
    { operation: 'sources', workspace_id: 'w', offset: 100 },
    { operation: 'source_get', workspace_id: 'w', id: 's' },
    { operation: 'source_delete', workspace_id: 'w', id: 's', expected_revision: 3 },
  ]);
});
