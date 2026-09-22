/** Run from repository root: node --experimental-vm-modules apps/desktop/src/demo-data/engine.test.mjs */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import { SourceTextModule, SyntheticModule } from 'node:vm';

const json = async name => JSON.parse(await readFile(new URL(name, import.meta.url), 'utf8'));
const fixtures = await json('./scenarios.json');
const artifact = await json('./model.json');
const parity = await json('./parity.json');
const domainUrl = new URL('../../../../crates/cdna-browser/assets/domain/cdna_browser.js', import.meta.url);
const domain = await import(domainUrl);
await domain.default({ module_or_path: await readFile(new URL('cdna_browser_bg.wasm', domainUrl)) });
const close = (a, b) => assert.ok(Math.abs(a - b) < 1e-10, `${a} != ${b}`);
assert.equal(artifact.model.weights.length, 17);
assert.equal(artifact.provenance.humanLabel, false);
assert.equal(artifact.provenance.verificationState, 'unconfirmed');
assert.equal(fixtures.length, 12);
for (const s of fixtures) {
  const result = JSON.parse(domain.score_candidates(JSON.stringify({feature_version:'1.0',weights:artifact.model.weights,request:{schema_version:'1.0',request_id:'11111111-1111-4111-8111-111111111111',workspace_id:'22222222-2222-4222-8222-222222222222',mode:'imitate',domain:'product_delivery',context:s.context,candidates:s.candidates,include_evidence:true,allow_cloud:false}})));
  const expected = parity.cases.find(c => c.scenarioId === s.id);
  for (const [i, c] of s.candidates.entries()) {
    close(result.scores[i], expected.ranking.find(r => r.candidate_id === c.id).raw_score);
    result.features[i].forEach((x, j) => close(x, expected.features[c.id][j]));
    close(result.contributions[i].reduce((a,b) => a+b,0), result.scores[i]);
  }
}
// Exercise the real TypeScript adapter, injecting only the file-loaded WASM module
// because Node cannot fetch a browser-relative runtime URL. No inference is mocked.
const code = stripTypeScriptTypes(await readFile(new URL('../demo-engine.ts', import.meta.url), 'utf8'));
const wasm = new SyntheticModule(['default','score_candidates'], function() {
  this.setExport('default', async () => {});
  this.setExport('score_candidates', domain.score_candidates);
});
await wasm.link(() => {}); await wasm.evaluate();
const engineModule = new SourceTextModule(code, { initializeImportMeta(meta) { meta.env = {BASE_URL:'/'}; }, importModuleDynamically: async () => wasm });
await engineModule.link(async spec => new SyntheticModule(['default'], function() { this.setExport('default', spec.endsWith('scenarios.json') ? fixtures : artifact); }));
await engineModule.evaluate();
const engine = engineModule.namespace;
await engine.warmup();
const outcomes = [];
for (const scenario of engine.scenarios) {
  const out = await engine.decide(scenario.id);
  const expected = parity.cases.find(c => c.scenarioId === scenario.id);
  assert.equal(out.abstained, expected.abstained);
  assert.equal(out.selectedCandidate?.id ?? null, expected.abstained ? null : expected.ranking[0].candidate_id);
  assert.ok(Number.isFinite(out.inferenceMs) && out.inferenceMs >= 0);
  assert.equal(out.engine, 'rust_wasm');
  assert.ok(out.rationale.length && out.reversalConditions.length);
  if (out.abstained) assert.equal(out.contributions.length, 0);
  else close(out.contributions.reduce((a,b)=>a+b.contribution,0),out.ranking[0].score);
  outcomes.push({id:scenario.id,selected:out.selectedCandidate?.id ?? null,abstained:out.abstained,inferenceMs:out.inferenceMs});
}
const low = await engine.decide('delivery',{deadline_pressure:-1});
const high = await engine.decide('delivery',{deadline_pressure:1});
assert.notEqual(low.selectedCandidate.id, high.selectedCandidate.id, 'Changing context must actually reverse a tradeoff');
assert.ok(high.reversalConditions.some(x=>x.includes('首位になります')));
assert.equal(engine.matchScenarios('納期が心配')[0].id, 'delivery');
assert.equal(engine.matchScenarios('宇宙へ行きたい').length, 0);
await assert.rejects(engine.decide('not-a-scenario'));
await assert.rejects(engine.decide('delivery',{budget_pressure:Infinity}));
await assert.rejects(engine.decide('delivery',{budget_pressure:2}));
await assert.rejects(engine.decide('delivery',{arbitrary:0}));
console.log(JSON.stringify({success:true,actualSharedRustWasm:true,pythonParity:12,contextChangesWinner:true,outcomes},null,2));
