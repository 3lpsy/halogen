Routed playlists UI.

- Routes: `/playlists`, `/playlists/create`, `/playlists/:id`, `/playlists/:id/edit`, `/playlists/:id/reorder-by`.
- Hosted by `routes`; navigation uses URL paths to avoid a dependency on the route enum.
- Reads shared state through hooks and sends mutations through `commands::actions`.
- Modules: `filter`, `playlist_detail`, `playlist_reorder_by`, `page`, `controls`, `playlist_form`.
