Routed settings UI.

- Routes: `/settings`, `/settings/playback`, `/settings/downloads`, `/settings/ui`, `/settings/accounts`, `/settings/accounts/add-embedded`, `/settings/server`, `/settings/podcasts`.
- Hosted by `routes`; navigation uses URL paths to avoid a dependency on the route enum.
- Reads shared state through hooks and sends mutations through `commands::actions`.
- Modules: `accounts`, `podcasts`, `playback`, `server`, `add_embedded_user`, `ui`, `downloads`.
