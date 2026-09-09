Routed episode UI.

- Routes: `/episodes/:id`, `/episodes/:episode_id/playlists`, `/episodes/:id/metadata`, `/episodes/bulk/playlists/:ids`.
- Hosted by `routes`; navigation uses URL paths to avoid a dependency on the route enum.
- Reads shared state through hooks and sends mutations through `commands::actions`.
- Modules: `episode_metadata`, `episode_playlists`, `bulk_episode_playlists`, `episode_detail`.
