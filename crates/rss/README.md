# halogen-rss

Feed parsing, episode ingestion and download orchestration.

- RssManager owns the database, HTTP client and resolved SyncContext.
- Sync entry points support scheduled, forced and reported polling.
- Feed parsing stays separate from storage and retention decisions.
