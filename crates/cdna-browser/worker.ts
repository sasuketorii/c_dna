/// <reference lib="webworker" />
export {};
const scope = globalThis as unknown as DedicatedWorkerGlobalScope;
let python: any;
let domain: any;
let busy = false;
const base = new URL('/runtime/', scope.location.origin);
async function validate(operation: string, json: string) {
  if (!domain) {
    const url = new URL('domain/cdna_browser.js', base).href;
    const module = await import(/* @vite-ignore */ url);
    await module.default({ module_or_path: new URL('domain/cdna_browser_bg.wasm', base) });
    domain = module;
  }
  return JSON.parse(domain[operation](json));
}
async function loadPython(id: number) {
  if (python) return python;
  scope.postMessage({ id, progress: 'ブラウザ内の学習環境を読み込んでいます（初回のみ）' });
  const url = new URL('pyodide/pyodide.mjs', base).href;
  const { loadPyodide } = await import(/* @vite-ignore */ url);
  const runtime = await loadPyodide({ indexURL: new URL('pyodide/', base).href });
  await runtime.loadPackage(['scikit-learn', 'pydantic'], { messageCallback: () => {}, errorCallback: () => {} });
  const response = await fetch(new URL('learner.py', base), { credentials: 'omit' });
  if (!response.ok) throw new Error('学習プログラムを読み込めませんでした');
  const source = await response.text();
  runtime.globals.set('__name__', 'cdna_browser_learner');
  await runtime.runPythonAsync(source);
  await runtime.runPythonAsync(`
def browser_request(raw):
    obj = json.loads(raw, object_pairs_hook=unique_object, parse_constant=lambda _: (_ for _ in ()).throw(ValueError("nonfinite")))
    if obj.get("operation") == "train" and (len(obj.get("records", [])) > 500 or obj.get("compare_lightgbm", False)):
        raise ValueError("browser training limit")
    output = json.dumps(run(Request.model_validate(obj)), allow_nan=False, separators=(",", ":"))
    if len(output.encode()) > 1024 * 1024:
        raise ValueError("output limit")
    return output
`);
  python = runtime;
  return python;
}
scope.onmessage = async ({ data }) => {
  const { id, operation, json } = data;
  if (busy) { scope.postMessage({ id, error: '処理中です' }); return; }
  busy = true;
  try {
    if (typeof json !== 'string' || new TextEncoder().encode(json).length > 8 * 1024 * 1024) throw new Error('入力上限');
    let result: unknown;
    if (operation === 'validate_proposal' || operation === 'validate_rank') {
      result = await validate(operation, json);
    } else if (operation === 'train' || operation === 'preview') {
      let payload = JSON.parse(json);
      if (operation === 'preview') {
        await validate('validate_rank', JSON.stringify(payload.request));
        payload = { operation: 'rank', feature_version: '1.0', model: payload.model, context: payload.request.context, candidates: payload.request.candidates, domain: payload.request.domain };
      } else if (payload.operation !== 'train' || payload.records?.length > 500 || payload.compare_lightgbm) {
        throw new Error('ブラウザ学習は500件までです');
      }
      const py = await loadPython(id);
      scope.postMessage({ id, progress: operation === 'train' ? 'ブラウザ内で学習しています' : '候補を比較しています' });
      py.globals.set('_browser_input', JSON.stringify(payload));
      try { result = JSON.parse(await py.runPythonAsync('browser_request(_browser_input)')); }
      finally { py.globals.delete('_browser_input'); }
    } else throw new Error('未対応の操作');
    scope.postMessage({ id, result });
  } catch {
    scope.postMessage({ id, error: '入力内容または学習環境を確認してください。処理を完了できませんでした。' });
  } finally { busy = false; }
};
