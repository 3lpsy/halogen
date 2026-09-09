Browser and desktop sync worker.

- Owns metadata state and emits per-domain events to UI providers.
- Drains queued operations through shared sync policy and transport.
- Pulls actor-scoped changes through the shared delta engine; an initial snapshot and later tombstones update the cache before publication.
- Journaled commands stage cache writes, operations, and cursor coalescing in `sync-journal::BufferedStore`.
- State publishes, success toasts, audio deletion, and progress polling wait for atomic commit. Storage failure restores the previous working state.
- Rejected changes retain their journal rows; affected metadata is revalidated once pending changes finish, with failed repairs retried at a bounded cadence.
- Device audio downloads run separately and report progress through worker commands.

## Glossary

- Journal: durable pending or rejected server operations.
- Cache: metadata retained for offline reads.
- Cursor: the committed server change-feed position, guarded against concurrent stale pulls.
- Tombstone: a server deletion that removes a cached resource.
- Tracked: a working state value that records whether it needs publication.
