# Queries

Reusable database queries and snapshot operations, separate from HTTP handlers.

- `episodes` applies validated list filters within the actor's subscriptions,
  including per-user playback state and search relevance.
- `playlists` filters authorized playlist episodes using per-user playback state.
- `snapshot` uses a bound path for consistent SQLite export, scrubs the copy
  through ORM operations and reads migration identities for import checks.
- `cache`, behind `sqlite-cache`, owns native cache and sync-journal SQL.
- Whole-database snapshot callers must authorize administrator access first.
- No HTTP hosting or platform UI dependencies. Model definitions stay in `orm`.
