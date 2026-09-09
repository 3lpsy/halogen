# halogen-sync

Shared durable push and incremental pull execution.

- Push applies canonical journal operations through ApiClient and shared retry policy.
- Pull refreshes changed resources and applies tombstones; cache updates and cursors commit together.
- Initial snapshots retry when actor changes make offset paging inconsistent.
- Cursor comparison rejects stale concurrent pulls; queued mutations prevent a pull from overwriting optimistic edits.
- Platform adapters own scheduling and project cached snapshots into their UI stores.

## Glossary

- Cursor: Opaque position in an actor-scoped server change stream.
- Tombstone: A deletion event applied to the cache.
- Snapshot: Complete canonical metadata collected before incremental updates.
