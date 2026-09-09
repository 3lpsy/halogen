WASM Web Worker entrypoint for the background sync service.

- Built as a wasm-bindgen library and started by the app's `worker.js` bootstrap.
- An initialization message supplies the account namespace before opening metadata and audio stores.
- Commands and domain events cross `postMessage` as JSON; worker logs forward to the main-thread logger.
- Native builds are empty because native scheduling runs the service directly.
