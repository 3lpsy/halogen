// Bootstrap the separate sync worker built by wasm-bindgen.
// Buffer messages until Rust installs its handler.

// CRITICAL — early message buffer: the main thread posts `Init` the instant the
// Worker is constructed, but our wasm loads ASYNCHRONOUSLY and messages sent
// before a listener exists are LOST. Buffer synchronously now; `worker_main`
// replays the buffer into the real (Rust) handler, in order.
const __buffered = [];
self.onmessage = (e) => __buffered.push(e);

// CRITICAL — surface worker-scope crashes to the parent's `Worker.onerror`
// (watched by `WebWorkerRuntime` to respawn / fall back): a Rust panic in a
// spawned sync future is an UNHANDLED PROMISE REJECTION, which per spec does
// NOT fire parent onerror — re-raise via a timer throw so an UNCAUGHT error does.
self.addEventListener('unhandledrejection', (e) => {
  console.error('halogen worker: unhandled rejection', e.reason);
  e.preventDefault();
  const err = e.reason instanceof Error ? e.reason : new Error(String(e.reason));
  setTimeout(() => { throw err; });
});
self.addEventListener('error', (e) => {
  // Logged for the worker console; the `error` event already propagates to the
  // parent `Worker.onerror` on its own (we don't `preventDefault`).
  console.error('halogen worker: uncaught error', e.message || e.error);
});

importScripts('/worker/halogen_worker.js');
wasm_bindgen('/worker/halogen_worker_bg.wasm').then(() => {
  // `worker_main` installs the real Rust `onmessage` handler (replacing the buffer).
  wasm_bindgen.worker_main();
  const handler = self.onmessage;
  // Replay everything received during wasm load, in arrival order.
  for (const e of __buffered) handler(e);
  __buffered.length = 0;
}).catch((err) => {
  // The wasm failed to load/instantiate: without this the worker would die silently
  // (never installing the real handler, never replying `Ready`) and sync would just
  // appear stuck. Surface a diagnostic on the worker console; the main thread's
  // `onerror` handler also logs uncaught worker errors.
  console.error('halogen worker: wasm load failed', err);
});
