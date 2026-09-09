Provider-layer local profile lifecycle and account activation for desktop.

- `desktop` enables `local-runtime`, which selects `halogen-local-runtime`, which binds no API port.
- The config overlay retains profile IDs in `halogen-local` URLs; API clients resolve them to in-process sessions.
- Existing account cache namespaces and database paths are preserved.
- The browser build exposes an unavailable stub. Desktop media uses the separate webview media bridge.
