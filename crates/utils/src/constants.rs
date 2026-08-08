/// Length of generated admin passwords (used by `seed_admin_user`).
pub const WT_PASSWORD_LENGTH: usize = 20;

// ── Server runtime defaults ──────────────────────────────────────────────────
// The canonical home for the server's `Config` defaults (the server's
// `config::Config::default()` reads them here). Kept in `utils` so the UI can
// share the download/poll values (the podcast-config form prefills them).
pub const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1";
pub const DEFAULT_LISTEN_PORT: u16 = 8080;
/// URL schemes a client may use for its configured server base URL. Validated on
/// the connect/login form and whenever auth is (re)applied, so a stray scheme
/// (`file:`, `ftp:`, `javascript:`) can't be stored and later used to build API /
/// media URLs. Add a scheme here to allow it everywhere at once.
pub const ALLOWED_SERVER_URL_SCHEMES: &[&str] = &["http", "https"];
pub const DEFAULT_DB_PATH: &str = "halogen.db";
pub const DEFAULT_MEDIA_ROOT: &str = "./media";
/// Discover providers' upstream search endpoints. The server proxies these (the
/// client never calls them directly). Overridable on `Config` — primarily a test
/// seam so integration tests can point them at a wiremock server.
pub const DEFAULT_DISCOVER_ITUNES_SEARCH_URL: &str = "https://itunes.apple.com/search";
pub const DEFAULT_DISCOVER_GPODDER_SEARCH_URL: &str = "https://gpodder.net/search.json";
/// Filename of the runtime config-overrides file. Lives beside the main config
/// file (or in the config dir) and is layered LAST, on top of CLI/env/TOML.
pub const DEFAULT_CONFIG_OVERRIDES_FILENAME: &str = "config.overrides.toml";
/// How often the poller WAKES to scan for due podcasts (the loop cadence, not how
/// often any one feed is fetched). Not overridable per podcast.
pub const DEFAULT_POLL_WAKE_INTERVAL_SECONDS: u64 = 5 * 60;
pub const DEFAULT_CHUNK_SIZE: u64 = 1 << 20;

// ── Outbound-fetch resource caps (DoS guards) ────────────────────────────────
// Hard ceilings on bytes we read from / write for untrusted remotes (feed hosts,
// episode-media origins). All are deliberately generous — they exist to stop a
// hostile or misconfigured host from exhausting memory/disk, not to constrain
// legitimate content.

/// Largest RSS/Atom feed document we'll buffer in memory before parsing. The body
/// comes from a (user-supplied) feed URL and is read whole, so an unbounded read
/// is a memory-exhaustion vector. Real feeds are well under a megabyte; 128 MiB is
/// a generous ceiling that still refuses a multi-gigabyte response.
pub const MAX_FEED_BODY_BYTES: u64 = 128 << 20;

/// Refuse to write newly-downloaded media once the filesystem backing the media
/// root is at or above this percent full (used ÷ total capacity). Leaves headroom
/// so a runaway or oversized download can't fill the volume and wedge the app and
/// its SQLite DB. Checked before a download starts and periodically as it streams.
pub const MAX_DISK_USAGE_PERCENT: u64 = 90;

/// Default API token / session lifetime, in minutes. ~1 week.
pub const DEFAULT_TOKEN_EXPIRY_MINUTES: u64 = 60 * 24 * 7;
/// An episode is `Finished` once playback reaches the last N% of its duration.
pub const DEFAULT_EPISODE_PLAYBACK_COMPLETE_PERCENTAGE: u16 = 4;

// Podcast download/poll override defaults — shared with the UI's podcast-config
// form. Typed `u32` to match the `Option<u32>` fields on `PodcastConfigData`; the
// server casts to `u64`/`usize` where its `Config` needs them.
pub const DEFAULT_POLL_INTERVAL_SECONDS: u32 = 60 * 60;
pub const DEFAULT_MAX_EPISODES: u32 = 50;
pub const DEFAULT_MAX_CONCURRENT_DOWNLOADS: u32 = 3;
pub const DEFAULT_MAX_POLL_CONCURRENT: u32 = 5;
/// Default steady-state stuck-download cutoff: a row left `Downloading` longer
/// than this is treated as orphaned and reset by the watchdog.
pub const DEFAULT_DOWNLOAD_STUCK_AFTER: std::time::Duration =
    std::time::Duration::from_secs(6 * 60 * 60);
/// Default attempt budget before a repeatedly-failing download is marked
/// `DownloadBroken` (terminal, never auto-retried).
pub const DEFAULT_DOWNLOAD_MAX_ATTEMPTS: usize = 10;
/// Whether the poller auto-downloads new episodes server-side. Off by default;
/// overridable globally (`subscription_poll_auto_download_enabled`) or per podcast
/// (`podcast_config.auto_download_enabled`).
pub const DEFAULT_AUTO_DOWNLOAD_EPISODES_ENABLED: bool = false;
/// Whether the poller inserts auto-added episodes at the START of their target
/// playlists instead of appending at the end. Off by default; overridable
/// globally (`subscription_auto_playlist_add_to_start`) or per podcast
/// (`podcast_auto_playlist.add_to_start`).
pub const DEFAULT_AUTO_PLAYLIST_ADD_TO_START: bool = false;

// ── Validation error envelope vocabulary ─────────────────────────────────────
// Errors are `validator`'s shape: field -> [{ code, message }].
//   field = WHERE the error is (the location). Specific when we can pinpoint it,
//           generic fallbacks otherwise.
//   code  = WHY it happened (the reason). Specific when we know it, generic
//           otherwise; the HTTP status is DERIVED from `code` in
//           `extract_status_code` — codes are never status names.
// Do NOT add model/resource names (podcast/user/...) as fields, nor HTTP status
// names (not_found/bad_request/...) as codes. That pollution is exactly what this
// split exists to prevent — keep fields about *where* and codes about *why*.

// fields (the WHERE) — most specific to most generic
pub const VALIDATION_ID_FIELD: &str = "id";
pub const VALIDATION_AUTHTOKEN_FIELD: &str = "authtoken";
pub const VALIDATION_AUTHCOOKIE_FIELD: &str = "authcookie";
pub const VALIDATION_DATABASE_FIELD: &str = "database";
pub const VALIDATION_DATA_FIELD: &str = "data";
pub const VALIDATION_PARAMS_FIELD: &str = "params";
pub const VALIDATION_REQUEST_FIELD: &str = "request";

// codes (the WHY) — most specific to most generic
pub const VALIDATION_EXISTS_CODE: &str = "exists";
pub const VALIDATION_UNAUTHENTICATED_CODE: &str = "unauthenticated";
pub const VALIDATION_UNAUTHORIZED_CODE: &str = "unauthorized";
pub const VALIDATION_UNIQUE_CODE: &str = "unique";
pub const VALIDATION_CONFLICT_CODE: &str = "conflict";
pub const VALIDATION_UNIMPLEMENTED_CODE: &str = "unimplemented";
pub const VALIDATION_PANIC_CODE: &str = "panic";
pub const VALIDATION_PARSING_CODE: &str = "parsing";
pub const VALIDATION_INVALID_CODE: &str = "invalid";
