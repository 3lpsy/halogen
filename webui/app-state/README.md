Domain snapshots shared by UI hooks, providers, and the sync worker.

- Separates episodes, podcasts, playlists, playbacks, history, downloads, sessions, and connection state.
- Contains data and URL construction; persistence and mutation scheduling live in other crates.
- `QueueState` distinguishes an unresolved default playlist from a confirmed absent queue.
