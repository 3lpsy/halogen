# ORM

SeaORM entities and conversions for the podcast database.

- Models cover users, podcasts, episodes, chapters, playlists, subscriptions,
  playback/status, polling jobs and download/sync errors.
- Wire conversions live here because entities depend on wire contracts.
- `common` provides simple entity lookup, sorting and pagination helpers.
- Complex application selection and raw snapshot SQL live in `queries`.
- Schema changes belong to new forward migrations in `migrations`.
