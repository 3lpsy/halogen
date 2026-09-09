# halogen-sync-enrich

Portable mutation, journal and cache change types.

- OutboxOp is the canonical mutation representation shared by browser and native hosts.
- StoreChanges groups metadata updates, removals and cursor changes.
- JournalEntry and StoredEntry carry durable delivery state.
- Episode query helpers apply the same filter and ordering semantics across stores.

## Glossary

- Outbox operation: A typed user mutation to apply and eventually deliver.
- Journal entry: An operation plus persistent identity and delivery state.
