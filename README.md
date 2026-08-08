# halogen

Halogen is a self-hostable, local-first podcast application written in Rust:
one server, three native-feeling clients, and full offline playback.

- **Server** (`crates/server`): an Axum REST API that subscribes to RSS
  feeds, polls them in the background, downloads episodes, and caches
  artwork — backed by SQLite. Feeds, audio, and artwork are fetched
  server-side; clients only ever talk to their own server.
- **Web / desktop app** (`crates/ui*`): a Dioxus frontend that stores
  episodes on-device and plays them offline; embedded into the server
  binary for single-binary deploys.
- **iOS app** (`ios/`): native SwiftUI. **Android app** (`android/`): native
  Kotlin/Compose. Both talk the same REST API and can run fully standalone
  by hosting the server in-process ("embedded server" mode, over UniFFI).

Shared wire types and validation live in `crates/wire*`; Swift and Kotlin
DTOs are generated from the same Rust definitions.

## Quick start

```bash
# Minimal: binds 127.0.0.1:8080, SQLite at ./halogen.db
halogen-server --auth-token-secret "$(openssl rand -hex 32)"

# Seed an admin on first boot + import a feed list
halogen-server --auth-token-secret ... \
  --admin-username admin --admin-password 'SecurePass123!' \
  --opml-file feeds.opml
```

A release binary serves the web app at `/`. Every knob (flags, `HALOGEN_*`
env vars, TOML file, runtime overrides) is documented in
[`docs/SERVER_CONFIG.md`](docs/SERVER_CONFIG.md).

## Building

A `justfile` drives everything — `just` (no args) lists all recipes.

```bash
just build-release    # server binary with the web frontend embedded
just ui-build         # web frontend -> dist/
just dev-server       # dev API server on :8080 serving dist/
just ios-build        # iOS app (needs macOS + Xcode)
just android-build    # Android APK (needs the Android SDK/NDK)
just test-all         # every test tier: unit -> integ -> ui -> e2e
```

Web/server builds need Rust (stable), the `dx` CLI, the `tailwindcss` CLI,
and `npm`. Always build the frontend through `just` — the recipes compile
Tailwind and pass the required feature flags.
