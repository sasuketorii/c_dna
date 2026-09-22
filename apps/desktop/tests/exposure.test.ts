import assert from 'node:assert/strict';
import test from 'node:test';
import { ExposureHistory } from '../src/exposure.ts';
import type { Proposal } from '../src/api.ts';
const saved = { workspace_id: 'workspace-a', family_id: 'family-a', model_exposure: false } as Proposal;

test('preview then reload old memory for revision cannot restore an unexposed label', () => {
  const history = new ExposureHistory();
  history.mark(saved, 'record-a');
  const reopened = history.apply(structuredClone(saved), 'record-a');
  assert.equal(reopened.model_exposure, true);
  assert.equal(history.apply({ ...reopened, model_exposure: false }, 'record-a').model_exposure, true);
  assert.equal(saved.model_exposure, false, 'existing pre-preview observation stays unchanged');
});

test('family and record identity survive revision, but workspace and unrelated records stay separate', () => {
  const history = new ExposureHistory();
  history.mark(saved, 'record-a');
  assert.equal(history.apply(saved, 'record-b').model_exposure, true);
  assert.equal(history.apply({ ...saved, family_id: 'changed-family' }, 'record-a').model_exposure, true);
  assert.equal(history.apply({ ...saved, family_id: 'unrelated' }, 'record-b').model_exposure, false);
  assert.equal(history.apply({ ...saved, workspace_id: 'workspace-b' }, 'record-a').model_exposure, false);
});

test('existing exposure is never downgraded and lock clears only session history', () => {
  const history = new ExposureHistory();
  history.mark(saved, 'record-a');
  history.clear();
  assert.equal(history.apply(saved, 'record-a').model_exposure, false);
  assert.equal(history.apply({ ...saved, model_exposure: true }).model_exposure, true);
});
