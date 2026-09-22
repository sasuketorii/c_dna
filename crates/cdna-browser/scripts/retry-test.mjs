/** Executes the actual worker module with injected initialization failures. */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { stripTypeScriptTypes } from 'node:module';
import vm from 'node:vm';
const source = stripTypeScriptTypes(await readFile(new URL('../worker.ts', import.meta.url), 'utf8'));

async function fixture(failure) {
  const messages = [];
  let remainingFailures = 1;
  let pythonStarts = 0;
  let domainStarts = 0;
  const fail = (stage) => {
    if (failure === stage && remainingFailures > 0) { remainingFailures--; throw new Error(`injected ${stage}`); }
  };
  const domain = {
    default: async () => { domainStarts++; fail('domain_init'); },
    validate_rank: (value) => { assert.equal(remainingFailures, 0); return value; },
  };
  const python = {
    loadPyodide: async () => {
      pythonStarts++;
      fail('python_init');
      let packages = false;
      let learner = false;
      let bridge = false;
      return {
        globals: new Map(),
        loadPackage: async () => { fail('loadPackage'); packages = true; },
        runPythonAsync: async (code) => {
          assert.ok(packages, 'packages must load before execution');
          if (code === 'LEARNER_SOURCE') { fail('learner_exec'); learner = true; return; }
          if (code.includes('def browser_request')) { assert.ok(learner); fail('bridge_exec'); bridge = true; return; }
          assert.ok(bridge, 'no cached partially initialized runtime');
          return '{"ok":true,"model":{"weights":[1]}}';
        },
      };
    },
  };
  const context = vm.createContext({ URL, TextEncoder, location:{origin:'https://example.test'}, postMessage:(value)=>messages.push(value), fetch: async () => {
    if (failure === 'source_fetch' && remainingFailures > 0) { remainingFailures--; return {ok:false}; }
    return {ok:true,text:async()=> 'LEARNER_SOURCE'};
  }});
  const module = new vm.SourceTextModule(source, {context,importModuleDynamically: async (url) => {
    const value = url.includes('cdna_browser.js') ? domain : python;
    const dependency = new vm.SyntheticModule(Object.keys(value), function() { for (const [key,item] of Object.entries(value)) this.setExport(key,item); }, {context});
    await dependency.link(()=>{});
    await dependency.evaluate();
    return dependency;
  }});
  await module.link(()=>{});
  await module.evaluate();
  const operation = failure === 'domain_init' ? 'validate_rank' : 'train';
  const json = operation === 'train' ? '{"operation":"train","feature_version":"1.0","records":[]}' : '{"valid":true}';
  await context.onmessage({data:{id:1,operation,json}});
  assert.ok(messages.find(message=>message.id===1 && message.error), `${failure}: first request must fail`);
  await context.onmessage({data:{id:2,operation,json}});
  assert.ok(messages.find(message=>message.id===2 && message.result), `${failure}: second request must recover`);
  assert.equal(failure === 'domain_init' ? domainStarts : pythonStarts, 2, `${failure}: fresh initialization required`);
  await context.onmessage({data:{id:3,operation,json}});
  assert.ok(messages.find(message=>message.id===3 && message.result));
  assert.equal(failure === 'domain_init' ? domainStarts : pythonStarts, 2, 'successful runtime must be reused');
}
for (const stage of ['domain_init','python_init','loadPackage','source_fetch','learner_exec','bridge_exec']) await fixture(stage);
console.log('PASS: 6 initialization failure stages recover on retry, then reuse successful state');
