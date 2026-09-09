Halogen's Dioxus browser and desktop UI.

- `app` launches the renderer. The package is `halogen-webui`; the desktop binary remains `halogen-ui` for existing bundles.
- `routes` owns route matching, authentication redirects, navigation history, and shell composition. `page-login` and `view-*` own page content and navigate using URL paths.
- `provider-app` composes `provider-*` lifecycles. `hook-*` expose typed reads and derived state without depending on provider implementations; `hooks` collects their exports.
- `commands` defines the worker protocol and `commands::actions` dispatch helpers. Views send mutations through these helpers; the sync worker owns durable state changes.
- `component-*` contain reusable presentation. Podcast, playlist, episode, and player packages contain domain behavior.
- Playback is split between `player-types`, `player`, `player-sleep`, `player-backend`, `player-media-session`, and `player-controls`.
- `provider-local` connects desktop local profiles to the Rust runtime. `provider-webview-media` provides the webview's audio transport.

Use `just ui-build` for browser builds so Tailwind and distribution assets are prepared. Select `desktop` with default features disabled for native desktop builds. Renderer features propagate through dependent UI packages.
