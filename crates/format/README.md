# Format

Pure date and playback-position formatting shared by application clients.

- `dt_human` formats an instant in the requested timezone.
- `format_time` preserves the player's minutes-and-seconds display.
- WASM-safe; no database, HTTP, filesystem or UI dependencies.
