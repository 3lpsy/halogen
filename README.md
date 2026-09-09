# halogen

Halogen is a self-hostable podcast application with a shared Rust core,
local libraries and offline playback.

- `crates/server` hosts the Axum API and browser app for remote clients.
  SQLite stores the library; background jobs poll feeds and cache media.
- `webui/` contains the Dioxus browser and desktop clients, organized into
  focused view, provider, hook and player packages.
- `ios/` is SwiftUI and `android/` is Kotlin/Compose. Local application calls
  enter the Rust runtime through UniFFI without starting an HTTP server.
  Desktop calls the same core in process and keeps a small media bridge
  for webview audio.

Remote clients fetch media through their selected server. Local Only needs no
Halogen account or synchronization server, while explicit feed and media
requests may still use the network. Linux releases include Flatpak, AppImage
and standalone desktop packages; Windows and Android remain supported.

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

The justfile lists build, test, device and CI commands. Run `just` to see them.

```bash
just build-release    # server binary with the web frontend embedded
just ui-build         # web frontend -> dist/
just dev-server       # dev API server on :8080 serving dist/
just ios-check        # Linux Swift typecheck with the Darwin SDK
just ios-device-test  # Linux app/test build and physical-device execution
just ios-build        # Simulator app with macOS + Xcode
just android-build    # Android APK (needs the Android SDK/NDK)
just check-all        # complete Linux gate, including browser journeys
```

Web/server builds need Rust (stable), the `dx` CLI, the `tailwindcss` CLI,
and `npm`. Always build the frontend through `just` — the recipes compile
Tailwind and pass the required feature flags.
