Browser metadata adapter over the shared `sync-store` contract.

- Re-exports common store types and the native SQLite adapter.
- IndexedDB stores records separately and indexes episode publication time and podcast ownership.
- Cache changes, journal appends, and delta cursors commit together; cursor comparison rejects stale concurrent pulls.
- Schema upgrades rebuild disposable cache stores while preserving queued operations.

## Glossary

- Cursor: the last committed server change-feed position.
- Journal: queued and quarantined mutations retained on this device.
- Namespace: storage isolation for one account and server.
