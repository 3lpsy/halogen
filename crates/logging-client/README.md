# halogen-logging-client

Device log capture shared by native shells and Dioxus clients.

- Defines severity and log records, a bounded in-memory ring and a pending persistence queue.
- Owns the tracing subscriber, runtime capture filters and worker forwarding layer.
- Contains no webui crate dependencies; platform persistence and file downloads remain in webui/logging.
