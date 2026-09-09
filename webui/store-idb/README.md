IndexedDB transactions, records, and schema management for UI services.

- Versioned schemas distinguish disposable cache stores from durable account, configuration, and journal stores.
- Provides JSON records, episode indexes, and transaction completion helpers.
- Guarded awaits keep callbacks alive when a request future is cancelled.
- Durable journal reads fail on malformed entries; disposable cache readers may skip corrupt records.
- Exports browser functionality only; native builds contain no IndexedDB implementation.

## Glossary

- Cache store: records that can be rebuilt after a schema change.
- Durable store: records preserved during schema upgrades.
- Guarded request: an IndexedDB request whose callback lifetime survives cancellation.
