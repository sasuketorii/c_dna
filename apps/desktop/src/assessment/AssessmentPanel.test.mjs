import { test, expect, mock } from 'bun:test';
mock.module('../api', () => ({ command: async () => { throw new Error('No transport in import validation tests'); } }));
const { parseAssessmentImport } = await import('./AssessmentPanel');
const fixture = () => ({ schema_version: '1.0', instrument: 'bfi_2_j', instrument_version: 'report edition', locale: 'ja-JP', confirmed_by_user: true, assessed_on: '2026-09-22', scores: [{ dimension: 'Reported dimension', value: 3, minimum: 1, maximum: 5, unit: 'original points', missing_reason: null }], source: { kind: 'manual_result', reference: 'My report' }, consent: { store_locally: true, mcp_read: false, external_send: false }, validity: 'user_reported_unverified' });
test('import preserves original result but requires fresh consent and confirmation', () => {
  const input = fixture();
  const result = parseAssessmentImport(JSON.stringify(input));
  expect(result.scores).toEqual(input.scores);
  expect(result.source).toEqual({ kind: 'imported_result', reference: 'My report' });
  expect(result.consent).toEqual({ store_locally: false, mcp_read: false, external_send: false });
  expect(result.confirmed_by_user).toBe(false);
});
test('unsupported instrument, inferred source, validity claim and sharing reject', () => {
  for (const patch of [{ instrument: 'unknown' }, { validity: 'validated' }, { source: { kind: 'conversation_inference', reference: 'chat' } }, { consent: { store_locally: true, mcp_read: true, external_send: false } }]) {
    expect(() => parseAssessmentImport(JSON.stringify({ ...fixture(), ...patch }))).toThrow();
  }
});
test('malformed, oversized, missing scores or object-valued score fail closed', () => {
  for (const text of ['null', '{', ' '.repeat(32769), JSON.stringify({ ...fixture(), scores: [] }), JSON.stringify({ ...fixture(), scores: [{}] })]) {
    expect(() => parseAssessmentImport(text)).toThrow();
  }
});
