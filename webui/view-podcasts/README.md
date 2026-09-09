Routed podcasts UI.

- Routes: `/podcasts`, `/podcasts/create`, `/podcasts/:id`, `/podcasts/:id/metadata`, `/podcasts/:id/edit`, `/podcasts/:id/config/create`, `/podcasts/:id/config/:config_id/edit`, `/podcasts/:id/auto-playlists`.
- Hosted by `routes`; navigation uses URL paths to avoid a dependency on the route enum.
- Reads shared state through hooks and sends mutations through `commands::actions`.
- Modules: `podcast_config_form`, `filter`, `podcast_auto_playlists`, `page`, `podcast_edit`, `podcast_metadata`, `podcast_detail`, `controls`, `podcast_create`.
