import { cpSync, existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
const source = fileURLToPath(new URL('../../../crates/cdna-browser/assets/', import.meta.url));
const target = fileURLToPath(new URL('../public/runtime/', import.meta.url));
for (const file of ['domain/cdna_browser.js', 'domain/cdna_browser_bg.wasm', 'pyodide/pyodide.mjs', 'manifest.json', 'learner.py']) {
  if (!existsSync(join(source, file))) throw new Error(`Missing browser runtime ${file}; prepare crates/cdna-browser assets first.`);
}
const manifest = JSON.parse(readFileSync(join(source, 'manifest.json'), 'utf8'));
for (const [name, evidence] of Object.entries(manifest.files)) {
  const path = join(source, name === 'learner.py' ? name : `pyodide/${name}`);
  const bytes = readFileSync(path);
  if (createHash('sha256').update(bytes).digest('hex') !== evidence.sha256) throw new Error(`Runtime integrity mismatch: ${name}`);
}
const learner = readFileSync(fileURLToPath(new URL('../../../learner/src/cdna_learner/worker.py', import.meta.url)));
if (!learner.equals(readFileSync(join(source, 'learner.py')))) throw new Error('Browser learner is stale; run runtime preparation again.');
function inspect(directory) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) inspect(path);
    else if (!entry.isFile() || statSync(path).size > 25 * 1024 * 1024) throw new Error(`Invalid static asset: ${entry.name}`);
  }
}
inspect(source);
cpSync(source, target, { recursive: true, force: true });
console.log('Verified and staged the shared browser runtime.');
