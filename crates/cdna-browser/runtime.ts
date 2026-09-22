/** Lazy browser-only learner. No data leaves this worker; reset destroys its memory. */
type Operation = 'validate_proposal' | 'validate_rank' | 'train' | 'preview';
type Pending = { resolve: (value: unknown) => void; reject: (error: Error) => void; timer: ReturnType<typeof setTimeout> };
let worker: Worker | undefined;
let pending: Pending | undefined;
let nextId = 0;
let activeId = 0;
export function reset(): void {
  worker?.terminate();
  worker = undefined;
  if (pending) {
    clearTimeout(pending.timer);
    pending.reject(new Error('処理を中断しました'));
    pending = undefined;
  }
}
export function call(operation: Operation, payload: unknown): Promise<any> {
  if (pending) return Promise.reject(new Error('処理中です。完了または中断してから再実行してください'));
  const json = JSON.stringify(payload);
  if (new TextEncoder().encode(json).length > 8 * 1024 * 1024) return Promise.reject(new Error('入力が大きすぎます'));
  if (!worker) {
    worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module', name: 'cdna-local-learner' });
    worker.onmessage = ({ data }) => {
      if (!pending || data.id !== activeId) return;
      if (data.progress) { globalThis.dispatchEvent(new CustomEvent('cdna-runtime-progress', { detail: data.progress })); return; }
      const current = pending;
      pending = undefined;
      clearTimeout(current.timer);
      if (data.error) current.reject(new Error(data.error));
      else current.resolve(data.result);
    };
    worker.onerror = () => reset();
  }
  activeId = ++nextId;
  return new Promise((resolve, reject) => {
    pending = { resolve, reject, timer: setTimeout(() => reset(), 180_000) };
    worker!.postMessage({ id: activeId, operation, json });
  });
}
