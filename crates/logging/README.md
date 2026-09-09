# halogen-logging

Process-wide tracing and recent log capture.

- Initialize stdout, an in-memory ring and optional append-only file output.
- Repeated initialization is harmless.
- Recent lines support administrator log viewing when no file is configured.
