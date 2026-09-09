# halogen-webui-logging

Logging facade and platform adapters for the web UI.

- Reexports shared capture, types and tracing macros from halogen-logging-client.
- Persists logs to IndexedDB on web and a file on native targets.
- Exports browser downloads and native file writes for logs and other application exports.
