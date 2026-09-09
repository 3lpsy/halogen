//! Share IndexedDB database prefixes, store names, and record keys between producers and cache recovery without crate
//! coupling. Per-user databases append namespace segments; the account registry is device-global.

// ── Metadata store database (per-user): `halogen.store.{segment}` ──────────────

/// Database-name prefix; append the namespace segment.
pub const STORE_DB_PREFIX: &str = "halogen.store.";

/// Object stores within the metadata database.
pub const STORE_PODCASTS: &str = "podcasts";
pub const STORE_EPISODES: &str = "episodes";
pub const STORE_PLAYLISTS: &str = "playlists";
pub const STORE_PLAYBACKS: &str = "playbacks";
pub const STORE_SYNC_META: &str = "sync_metadata";
pub const STORE_AUTO_PLAYLISTS: &str = "podcast_auto_playlists";
pub const STORE_OUTBOX: &str = "outbox";

/// The re-fetchable cached-content stores (everything but the outbox). The
/// failsafe's "clear content" wipes these while preserving the `outbox` (which
/// holds unsynced offline actions). Kept disjoint from `STORE_OUTBOX` on purpose.
pub const CONTENT_STORES: [&str; 6] = [
    STORE_SYNC_META,
    STORE_AUTO_PLAYLISTS,
    STORE_PODCASTS,
    STORE_EPISODES,
    STORE_PLAYLISTS,
    STORE_PLAYBACKS,
];

// ── Config store database (per-user): `halogen.config.{segment}` ──────────────

/// Database-name prefix; append the namespace segment.
pub const CONFIG_DB_PREFIX: &str = "halogen.config.";

/// The single key/value object store holding the per-user config records.
pub const CONFIG_STORE: &str = "kv";

/// The list-view-state record's key within [`CONFIG_STORE`] (also the localStorage
/// suffix / native filename stem). The failsafe's "clear view settings" deletes
/// just this record, leaving the rest of the config intact.
pub const CONFIG_LIST_VIEWS_KEY: &str = "list_views";

// ── Device media store database (per-user): `halogen.media.{segment}` ────────── Device-downloaded audio bytes.
// Per-user like the metadata/config stores, so a remote and an embedded account (whose per-server episode ids collide)
// never serve each other's audio. The failsafe names these to purge every segment.

/// Database-name prefix; append the namespace segment.
pub const MEDIA_DB_PREFIX: &str = "halogen.media.";

/// The pre-namespacing device-global media database. No longer opened (the
/// per-segment stores replace it); cleaned up on upgrade and by the failsafe.
pub const MEDIA_DB_LEGACY: &str = "halogen.media";

/// Object stores within the media database. `audio` = committed downloads;
/// `audio_partial` + `audio_partial_meta` = in-progress chunk staging.
pub const MEDIA_AUDIO_STORE: &str = "audio";
pub const MEDIA_PARTIAL_STORE: &str = "audio_partial";
pub const MEDIA_META_STORE: &str = "audio_partial_meta";
pub const MEDIA_STORES: [&str; 3] = [MEDIA_AUDIO_STORE, MEDIA_PARTIAL_STORE, MEDIA_META_STORE];

// ── Account registry database (device-global) ─────────────────────────────────

/// The device-global account-registry database name.
pub const ACCOUNTS_DB: &str = "halogen.accounts";

/// The registry's object store.
pub const ACCOUNTS_STORE: &str = "registry";

/// The single registry record's key within [`ACCOUNTS_STORE`].
pub const ACCOUNTS_KEY: &str = "accounts";
