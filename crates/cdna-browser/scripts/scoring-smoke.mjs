import {readFile} from 'node:fs/promises';
import assert from 'node:assert/strict';
import {performance} from 'node:perf_hooks';
import init,{score_candidates} from '../assets/domain/cdna_browser.js';
await init({module_or_path:await readFile(new URL('../assets/domain/cdna_browser_bg.wasm',import.meta.url))});
const request=JSON.parse(await readFile(new URL('../../../contracts/fixtures/rank-valid-ratio.json',import.meta.url),'utf8'));
const weights=Array.from({length:17},(_,i)=>(i-8)/9);
const input=JSON.stringify({feature_version:'1.0',weights,request});
const output=JSON.parse(score_candidates(input));
assert.equal(output.engine,'rust_wasm');
assert.equal(output.scores.length,request.candidates.length);
for(let i=0;i<output.scores.length;i++){
 const expected=output.features[i].reduce((s,x,j)=>s+x*weights[j],0);
 assert.ok(Math.abs(output.scores[i]-expected)<1e-12);
 assert.ok(Math.abs(output.contributions[i].reduce((a,b)=>a+b,0)-expected)<1e-12);
}
assert.throws(()=>score_candidates(input.replace('"feature_version":"1.0"','"feature_version":"1.0","feature_version":"1.0"')));
assert.throws(()=>score_candidates(JSON.stringify({feature_version:'1.0',weights:[1],request})));
const timings=[];
for(let i=0;i<200;i++){const t=performance.now();score_candidates(input);timings.push(performance.now()-t);}
timings.sort((a,b)=>a-b);
console.log(JSON.stringify({engine:'actual_shared_rust_wasm',samples:200,candidates:request.candidates.length,p50_ms:timings[100],p95_ms:timings[190],scope:'Node warmed wasm scoring including strict JSON parsing, not browser paint or personal accuracy'}));
