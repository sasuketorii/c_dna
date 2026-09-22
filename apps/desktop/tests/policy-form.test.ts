import assert from 'node:assert/strict';
import test from 'node:test';
import { buildPolicy } from '../src/policy-form.ts';
import type { PolicyForm } from '../src/policy-form.ts';
const form:PolicyForm={statement:'納期が厳しいときは速度を重視する',whenField:'deadline_pressure',whenComparison:'gte',whenValue:.7,requiresField:'speed',requiresComparison:'gte',requiresValue:.8,expires:''};
const now=new Date('2026-09-22T00:00:00Z');
test('minimal form encodes background and candidate comparisons with required ratio units',()=>{
 const policy=buildPolicy(form,now);
 assert.deepEqual(policy.when,{op:'compare',field:'deadline_pressure',comparison:'gte',value:.7,unit:'ratio'});
 assert.deepEqual(policy.requires,{op:'compare',field:'speed',comparison:'gte',value:.8,unit:'ratio'});
 assert.equal(policy.effective_from,now.toISOString());assert.equal(policy.expires_at,null);
});
test('future expiry is UTC and invalid periods or values preserve a rejected draft',()=>{
 assert.equal(buildPolicy({...form,expires:'2026-09-23T00:00:00Z'},now).expires_at,'2026-09-23T00:00:00.000Z');
 for(const patch of [{statement:''},{whenValue:NaN},{requiresValue:2},{whenField:'speed'},{requiresField:'deadline_pressure'},{expires:'2026-09-21T00:00:00Z'},{expires:'invalid'}]) assert.throws(()=>buildPolicy({...form,...patch},now));
 assert.equal(form.statement,'納期が厳しいときは速度を重視する');
});
