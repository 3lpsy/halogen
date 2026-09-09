# halogen-download

Episode media downloads and retention.

- Stream to a temporary file and atomically publish the completed download.
- Retry transient failures, classify terminal failures and publish byte progress.
- Recovery revisits interrupted attempts; retention removes older stored media.
