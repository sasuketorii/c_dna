import { readFileSync, readdirSync, lstatSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { resolve, relative } from 'node:path';

// Read-only: never builds, uploads, loads credentials, or invokes a provider API.
const app = fileURLToPath(new URL('../', import.meta.url));
const root = resolve(app, 'dist-static');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const require = (condition, message) => { if (!condition) throw new Error(message); };
const configBytes = readFileSync(resolve(app, 'wrangler.jsonc'));
const config = JSON.parse(configBytes);
require(JSON.stringify(Object.keys(config).sort()) === JSON.stringify(['account_id', 'assets', 'compatibility_date', 'name', 'preview_urls', 'workers_dev'].sort()), 'Unexpected config key: review execution/bindings/routes before release');
require(config.name === 'cdna-playground' && config.account_id === '1b0d66d05b34ffe694010807c0c360c7', 'Deployment identity changed');
require(config.workers_dev === true && config.preview_urls === false, 'Publication configuration changed');
require(JSON.stringify(config.assets) === JSON.stringify({ directory: './dist-static', not_found_handling: 'none' }), 'Assets configuration changed');
const files = [];
function walk(directory) {
  for (const name of readdirSync(directory).sort()) {
    const path = resolve(directory, name), stat = lstatSync(path);
    require(!stat.isSymbolicLink(), 'Symlink in artifact');
    if (stat.isDirectory()) walk(path);
    else {
      const name = relative(root, path);
      require(stat.isFile() && stat.size < 25 * 1024 * 1024, `Invalid asset or size: ${name}`);
      require(!/(^|\/)(\.|node_modules|functions)|\.(map|pem|key|env)$|(^|\/)_worker\.js$/.test(name), `Unexpected publishable file: ${name}`);
      const bytes = readFileSync(path);
      if (name === '_redirects') require(bytes.toString().trim() === '', 'Unexpected redirect rules');
      if (/\.(js|mjs|html|json|py)$/.test(name)) {
        const text = bytes.toString();
        require(!/-----BEGIN .*PRIVATE KEY-----|\b(?:sk_live_|sk-proj-|ghp_|github_pat_)/.test(text), `Credential marker in ${name}`);
        require(!text.includes('/api/command'), `Desktop API route in ${name}`);
      }
      files.push({ name, bytes: stat.size, sha256: hash(bytes) });
    }
  }
}
walk(root);
require(files.length <= 20_000, 'Free-tier asset count exceeded');
const headers = readFileSync(resolve(root, '_headers'), 'utf8');
require(headers === readFileSync(resolve(app, 'public/_headers'), 'utf8'), 'Stale security headers');
for (const directive of ["default-src 'self'", "script-src 'self' 'wasm-unsafe-eval'", "connect-src 'self'", "worker-src 'self'", "object-src 'none'", "frame-ancestors 'none'"]) require(headers.includes(directive), `Missing CSP ${directive}`);
const manifest = JSON.parse(readFileSync(resolve(root, 'runtime/manifest.json')));
for (const [name, evidence] of Object.entries(manifest.files)) {
  const bytes = readFileSync(resolve(root, 'runtime', name === 'learner.py' ? name : `pyodide/${name}`));
  require(hash(bytes) === evidence.sha256 && bytes.length === evidence.bytes, `Runtime integrity mismatch: ${name}`);
}
for (const name of ['index.html', 'runtime/domain/cdna_browser.js', 'runtime/domain/cdna_browser_bg.wasm']) require(files.some(f => f.name === name), `Missing ${name}`);
for (const name of ['cdna_browser.js', 'cdna_browser_bg.wasm']) require(readFileSync(resolve(root, 'runtime/domain', name)).equals(readFileSync(resolve(app, '../../crates/cdna-browser/assets/domain', name))), `Stale domain runtime: ${name}`);
require(readFileSync(resolve(root, 'runtime/learner.py')).equals(readFileSync(resolve(app, '../../learner/src/cdna_learner/worker.py'))), 'Stale learner source');
const lock = JSON.parse(readFileSync(resolve(root, 'runtime/pyodide/pyodide-lock.json')));
const checked = new Set();
function checkPackage(name) {
  name = name.toLowerCase().replace(/[-_.]+/g, '-');
  if (checked.has(name)) return;
  checked.add(name);
  const entry = lock.packages[name];
  require(entry && /^[A-Za-z0-9_.-]+$/.test(entry.file_name), `Invalid package: ${name}`);
  require(hash(readFileSync(resolve(root, 'runtime/pyodide', entry.file_name))) === entry.sha256, `Missing or invalid self-hosted package: ${name}`);
  for (const dependency of entry.depends) checkPackage(dependency);
}
checkPackage('scikit-learn');
checkPackage('pydantic');
const receipt = { status: 'LOCAL_PREFLIGHT_PASS', configSha256: hash(configBytes), artifactSha256: hash(JSON.stringify(files)), fileCount: files.length, totalBytes: files.reduce((sum, f) => sum + f.bytes, 0), largest: files.reduce((a, b) => a.bytes > b.bytes ? a : b), files };
console.log(JSON.stringify(receipt, null, 2));
