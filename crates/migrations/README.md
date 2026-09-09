# halogen-migrations

SeaORM schema history and database connection setup.

- Schemas cover users, podcasts, episodes, playback, playlists, configs, subscriptions, statuses, chapters, poll jobs and sync metadata.
- Connection helpers select journal mode and run the registered migrations.
- Shipped migrations are immutable; add forward migrations for schema changes.
