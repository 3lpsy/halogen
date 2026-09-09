# Routes

HTTP-shaped application actions shared by server and local router dispatch.

- Resource adapters extract validated input, authorize references and call handlers.
- `routers` includes podcasts, episodes, playlists, playback, users, configuration,
  discovery, polling, media, logs and database transfer.
- `router` registers these actions; `server` owns listening and frontend hosting.
- Route tests use the shared real-router fixture through a development dependency.
- Folder facades only export actions; implementation and tests live separately.
