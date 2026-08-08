# Server configuration

`halogen-server` resolves its configuration from five layers, highest priority
first:

1. **Config-overrides file** — writable runtime layer (allowlisted keys only,
   see below)
2. **CLI flags**
3. **Environment variables** — `HALOGEN_*`
4. **TOML config file** — `--config <path>`, else the XDG default
   (`~/.config/halogen-server/halogen.toml` on Linux,
   `org.fgsec/halogen-server/halogen.toml` on macOS)
5. **Built-in defaults** (below)

`--auth-token-secret` is **required** — the server refuses to start without it.
It signs the JWTs, so keep it stable across restarts; changing it invalidates
every issued session.

## Knobs

Env var = `HALOGEN_` + the flag in SCREAMING_SNAKE (e.g. `--listen-port` →
`HALOGEN_LISTEN_PORT`). TOML uses the section shown.

### Server (`[server]`)

| Flag | Default | What it does |
|---|---|---|
| `--listen-address` | `127.0.0.1` | Bind address |
| `--listen-port` | `8080` | Bind port |
| `--server-disable-polling-service` | `false` | Don't start the background feed poller |
| `--allow-private-network` | `false` | SSRF guard: allow outbound fetches (feeds/art/media) to private or loopback hosts — needed for dev or a LAN feed host |
| `--cors-allowed-origin` (repeatable) | empty | Restrict CORS to these origins; empty = permissive (dev) |

### Database (`[db]`)

| Flag | Default | What it does |
|---|---|---|
| `--db-path` | `halogen.db` | SQLite file path |
| `--db-no-migrate` | `false` | Skip running migrations on startup |
| `--db-no-wal` | `false` | Disable WAL journaling (use on network filesystems; converts an existing WAL DB back) |
| `--db-skip-default-playlist` | `false` | Don't seed the default "Queue" playlist |

### Media & frontend (`[media]`, `[public]`)

| Flag | Default | What it does |
|---|---|---|
| `--media-root` | `./media` | Where downloaded episode audio + cached artwork live |
| `--enable-public-server` | `false` | Serve a static directory (the built frontend) from disk |
| `--public-root` | none | The directory to serve (point at `dist/`) |
| `--public-url-path` | `/` | Mount path; at `/` it becomes the SPA fallback |

### Subscriptions & downloads (`[subscription]`)

| Flag | Default | What it does |
|---|---|---|
| `--subscription-poll-wake-interval` | `300` s | Poller loop cadence (each wake fetches only feeds that are due) |
| `--subscription-fallback-poll-interval` | `3600` s | Per-feed poll interval when the podcast has no per-podcast config |
| `--subscription-fallback-max-episodes` | `50` | Retention cap per podcast (oldest downloads purged past it) |
| `--subscription-max-concurrent-downloads` | `3` | Parallel episode downloads |
| `--subscription-max-poll-concurrent` | `5` | Parallel feed fetches per poll |
| `--subscription-poll-auto-download-enabled` | `false` | Auto-download newly ingested episodes (per-podcast config wins) |
| `--subscription-download-stuck-after` | `21600` s | Watchdog: a download stuck this long resets to `DownloadError` for retry |
| `--subscription-download-max-attempts` | `10` | Attempts before a failing download is retired as `DownloadBroken` |
| `--subscription-sync-on-start` | `false` | Force-poll every feed once at startup |
| `--subscription-no-sync-before` | `2026-01-01` | Ignore episodes published before this date when syncing |
| `--dev-use-mock-download` | `false` | Fake downloads (dev/test) |
| `--dev-seed-data` | `false` | Seed sample data on startup (debug builds only) |

### Auth & playback (`[auth]`, `[episode]`)

| Flag | Default | What it does |
|---|---|---|
| `--auth-token-secret` | **required** | JWT signing secret |
| `--auth-token-expiry-minutes` | `43200` (30 d) | Token lifetime |
| `--episode-playback-complete-percentage` | `4` | % remaining at which playback counts as complete |

### Logging (`[log]`)

| Flag | Default | What it does |
|---|---|---|
| `--log-level` | `info` | `error`/`warn`/`info`/`debug`/`trace` |
| `--log-file` | none | Also write logs to this file |
| `--log-target` | `false` | Include the log target in output |
| `--log-file-name` | `true` | Include source file names |
| `--log-line-number` | `true` | Include source line numbers |

### Admin seeding & OPML (`[admin]`, `[opml]`)

| Flag | Default | What it does |
|---|---|---|
| `--admin-username` | none | Seed an admin user on first boot (only when no users exist) |
| `--admin-password` | random | Admin password; if omitted, a random one is generated and logged |
| `--admin-disable-seed` | `false` | Never seed an admin |
| `--opml-file` | none | Import this OPML feed list on startup |

### Config overrides (`[config_overrides]`)

| Flag | Default | What it does |
|---|---|---|
| `--config-overrides-path` | `config.overrides.toml` beside `--config` (else XDG) | Where the overrides file lives |
| `--config-overrides-disable` | `false` | Don't load the overrides file |

## Runtime config overrides

A small writable layer for changing tuning knobs without touching CLI/env/TOML:
admins edit the overrides file through the API (`GET/POST/DELETE
/api/v1/config-overrides`), then `POST /api/v1/server/restart` gracefully
re-execs the server to apply it. Only tuning keys are allowlisted (poll/download
intervals and limits, auto-download, token expiry, playback-complete %,
`opml_file`); binding, identity, and secret keys are never overridable. The file
uses the same sectioned TOML schema, listing only the overridden keys.
`GET /api/v1/config` reports the reconciled config and which fields are
overridden.
