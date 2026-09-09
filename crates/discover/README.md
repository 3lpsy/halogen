Bounded podcast directory search and read-only RSS previews.

- iTunes searches podcasts and episodes; gpodder searches podcasts and explicitly reports unsupported episode searches.
- Provider requests run concurrently; failures accompany successful results. Artwork is not fetched.
- Paged endpoints return 25 rows from immutable snapshots: 200 iTunes results plus up to 20 gpodder podcasts. Providers do not offer supported offset traversal.
- Cursors bind query, provider selection, and mode. Retrying is stable until ten-minute expiry or eviction; restart without a cursor afterward.
- Cache limits: 32 snapshots and 16 MiB. Remote requests have time, body, field, and SSRF bounds; previews create no library records.
- Tests use local provider/feed mocks only.
