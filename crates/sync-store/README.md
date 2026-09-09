# halogen-sync-store

Shared metadata and mutation persistence contract.

- LocalStore abstracts native SQLite and browser IndexedDB; NativeLocalStore delegates SQL to queries::cache.
- StoreChanges commits metadata, journal entries and sync cursors together.
- Audio bytes live in a separate media store.
- StoreHandle exposes an optional shared store to platform adapters.

## Glossary

- Journal: Durable mutation records awaiting delivery or projection confirmation.
- Cursor: Server change position committed with corresponding metadata.
