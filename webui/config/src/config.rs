//! The `ClientConfig` blob + its per-user `ClientConfigStore`, the
//! `DeviceLogConfig`, and `api_client_from` + the namespaced key/path helpers.
//! The preference value types it holds live in the sibling `prefs`/`nav`/
//! `swipe`/`list_view` modules.

use halogen_apiclient::ApiClient;
use serde::{Deserialize, Serialize};

use crate::store::NamespacedStore;
use crate::{
    AccountKey, DownloadPrefs, FontSize, ListViewStore, NavConfig, PlaybackPrefs, ServerKind,
    SwipePrefs,
};
use halogen_webui_logging::Level;
/// Local-only client configuration. Never synced to any server.
///
/// Holds the server URL, auth token(s), setup state, nav configuration, and playback prefs.
/// Stored in browser `localStorage` on web or a JSON file on native.
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct ClientConfig {
    /// e.g. "https://podcasts.example.com". For an `Embedded` config this is a
    /// runtime value: the store overlays the live loopback URL at every load
    /// (the persisted copy is whatever port was current at the last save — the
    /// port is never identity).
    pub server_url: Option<String>,
    /// Which kind of server this config is bound to. `Remote` for every
    /// pre-feature persisted config (serde default).
    #[serde(default)]
    pub server_kind: ServerKind,
    /// JWT from /auth/login
    pub access_token: Option<String>,
    /// Reserved for future refresh-token support; unused in single-token MVP
    pub refresh_jwt: Option<String>,
    /// True once server setup + login completed successfully
    pub server_setup: bool,
    /// Whether the signed-in user is a server admin (fetched after login).
    #[serde(default)]
    pub is_admin: bool,
    /// Client-side navigation ordering and visibility.
    #[serde(default)]
    pub nav: NavConfig,
    /// Client-side playback preferences.
    #[serde(default)]
    pub playback_prefs: PlaybackPrefs,
    /// Client-side UI font size scaling.
    #[serde(default)]
    pub font_size: FontSize,
    /// Device-log capture settings (enable flag + level threshold).
    #[serde(default)]
    pub device_logs: DeviceLogConfig,
    /// User-toggled "Go Offline" mode. When `true` the app behaves as offline
    /// regardless of real connectivity: the worker skips all network sync and
    /// reports `Offline`, and paged lists serve from cache only. Persisted so the
    /// choice survives a reload; the navbar status button toggles it.
    #[serde(default)]
    pub manual_offline: bool,
    /// Discover search providers the user has toggled OFF. Empty (the default)
    /// means every available provider is enabled. Stored as a *disabled* set so a
    /// newly shipped provider is enabled by default without a config migration;
    /// reconciled against the server's available list on the Discover page.
    #[serde(default)]
    pub disabled_discover_providers: Vec<halogen_wire::DiscoverProvider>,
    /// Per-page episode swipe-action configuration. Defaults reproduce the
    /// historical hardcoded swipes, so existing users see no change.
    #[serde(default)]
    pub swipe_prefs: SwipePrefs,
    /// Device-download chunk size + parallelism. Defaults (4 MB / 1) reproduce
    /// the historical behavior, so existing persisted configs load unchanged.
    #[serde(default)]
    pub download_prefs: DownloadPrefs,
}

/// Device-log capture configuration. Held in [`ClientConfig`] so it persists, and mirrored into the logging runtime
/// globals so the `/logs/device` capture gate is live (see `halogen_webui_logging`). `enabled` defaults to on in dev +
/// tests (`debug_assertions || cfg!(test)`) and off in release/prod.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceLogConfig {
    /// Whether log lines passing through the `logging::` facade are captured to
    /// the device log.
    #[serde(default = "default_device_logs_enabled")]
    pub enabled: bool,
    /// Capture threshold: lines at this level or more severe are kept.
    #[serde(default)]
    pub level: Level,
}

impl Default for DeviceLogConfig {
    fn default() -> Self {
        Self {
            enabled: default_device_logs_enabled(),
            level: Level::default(),
        }
    }
}

/// On by default in dev + tests, off in release/prod.
fn default_device_logs_enabled() -> bool {
    cfg!(debug_assertions) || cfg!(test)
}

impl ClientConfig {
    /// True when the user has completed setup and has a valid token.
    pub fn is_authenticated(&self) -> bool {
        self.server_setup && self.access_token.is_some()
    }

    /// Build the read-side client with the configured access token. Return None for missing/invalid/disallowed URLs or
    /// manual offline mode, preventing page/hook requests. Artwork loads bypass this builder.
    pub fn api_client(&self) -> Option<ApiClient> {
        if self.manual_offline {
            return None;
        }
        api_client_from(self.server_url.as_deref(), self.access_token.as_deref())
    }

    /// [`api_client`](Self::api_client) for write paths that surface a string
    /// error to the user instead of silently no-opping — one canonical message so
    /// "no server" reads the same everywhere.
    pub fn api_client_or_err(&self) -> Result<ApiClient, String> {
        self.api_client()
            .ok_or_else(|| "No server configured.".to_string())
    }
}

/// Build a read-only [`ApiClient`] from a server URL + token, validating the scheme against
/// [`ALLOWED_SERVER_URL_SCHEMES`]. `None` when the URL is missing, unparseable, or uses a disallowed scheme. Prefer
/// [`ClientConfig::api_client`]; this free form exists for callers that hold the URL/token apart from a full
/// `ClientConfig` (e.g. the paged list's `PagedDeps`).
pub fn api_client_from(server_url: Option<&str>, token: Option<&str>) -> Option<ApiClient> {
    let url = server_url?.parse::<url::Url>().ok()?;
    let native_local = cfg!(not(target_arch = "wasm32")) && url.scheme() == "halogen-local";
    if !native_local
        && !halogen_utils::constants::ALLOWED_SERVER_URL_SCHEMES.contains(&url.scheme())
    {
        return None;
    }
    let client = ApiClient::new(url);
    client.set_token(token.map(|t| t.to_string()));
    Some(client)
}

/// Store full config per user. Ambient load/save use AccountsProvider's namespace; explicit load_for/save_for support
/// login and switching before it changes. Pre-login uses anon; target-specific storage is delegated to NamespacedStore.
pub struct ClientConfigStore;

/// The shared namespaced-store backend for [`ClientConfig`] (key suffix
/// `client_config` / file `client.json`).
const CONFIG_STORE: NamespacedStore<ClientConfig> =
    NamespacedStore::new("client_config", "client.json");

/// Latch real load/parse failures and reject saves until boot successfully reloads. Otherwise fallback defaults could
/// overwrite the unreadable stored config and auth token; absence alone does not latch degradation.
static STORAGE_DEGRADED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether config storage is in the degraded (loads failed → saves refused)
/// state — see [`STORAGE_DEGRADED`].
pub fn config_storage_degraded() -> bool {
    STORAGE_DEGRADED.load(std::sync::atomic::Ordering::Relaxed)
}

impl ClientConfigStore {
    /// Load the active user's config (ambient namespace), defaults if none.
    /// A backend FAILURE also defaults (callers need a value), but latches
    /// [`STORAGE_DEGRADED`] so the faked default can never be saved over the
    /// real record. Boot code should prefer [`Self::try_load`] and retry.
    pub async fn load() -> ClientConfig {
        Self::try_load().await.unwrap_or_default()
    }

    /// Error-aware load of the active user's config: absent defaults, backend
    /// failure is `Err` (+ latches [`STORAGE_DEGRADED`]); success clears the
    /// latch — the active namespace is readable again, so saves are safe.
    pub async fn try_load() -> Result<ClientConfig, String> {
        match CONFIG_STORE.try_load().await {
            Ok(mut config) => {
                STORAGE_DEGRADED.store(false, std::sync::atomic::Ordering::Relaxed);
                config.nav.normalize();
                apply_runtime_overlay(&mut config);
                Ok(config)
            }
            Err(e) => {
                STORAGE_DEGRADED.store(true, std::sync::atomic::Ordering::Relaxed);
                halogen_webui_logging::error!(
                    error = %e,
                    "Client config failed to LOAD — refusing config saves until a load succeeds \
                     (saving now would overwrite the stored config with defaults)"
                );
                Err(e)
            }
        }
    }

    /// Persist the active user's config (ambient namespace). Refused while
    /// [`STORAGE_DEGRADED`] is latched — see its docs.
    pub async fn save(config: &ClientConfig) {
        if config_storage_degraded() {
            halogen_webui_logging::warn!(
                "Skipping config save: storage is degraded (an earlier load failed)"
            );
            return;
        }
        CONFIG_STORE.save(config).await;
    }

    /// Load a specific account's config (defaults if none). Used by login/
    /// switch, which address an account before the ambient namespace flips.
    /// A backend failure defaults but latches [`STORAGE_DEGRADED`] (it does
    /// NOT clear it on success — only the boot-path ambient load does).
    pub async fn load_for(key: AccountKey) -> ClientConfig {
        match CONFIG_STORE.try_load_for(key).await {
            Ok(mut config) => {
                config.nav.normalize();
                apply_runtime_overlay_for(&mut config, Some(key.id));
                config
            }
            Err(e) => {
                STORAGE_DEGRADED.store(true, std::sync::atomic::Ordering::Relaxed);
                halogen_webui_logging::error!(
                    error = %e,
                    "Account config failed to LOAD — refusing config saves until a load succeeds"
                );
                ClientConfig::default()
            }
        }
    }

    /// Persist a specific account's config. Refused while
    /// [`STORAGE_DEGRADED`] is latched — see its docs.
    pub async fn save_for(key: AccountKey, config: &ClientConfig) {
        if config_storage_degraded() {
            halogen_webui_logging::warn!(
                "Skipping config save: storage is degraded (an earlier load failed)"
            );
            return;
        }
        CONFIG_STORE.save_for(key, config).await;
    }

    /// Remove an account's config (on sign-out of that account). Also drops
    /// that account's list-view state — held in the sibling [`ListViewStore`] —
    /// which the ambient `ListViewStore::clear` can't reach for a non-active
    /// account, leaving it orphaned on the device forever otherwise.
    pub async fn clear_for(key: AccountKey) {
        CONFIG_STORE.clear_for(key).await;
        ListViewStore::clear_for(key).await;
    }
}

/// Normalize loaded local-runtime config using the host resolver and disable manual offline mode. All load paths share
/// this boundary so callers receive the live runtime address without depending on the runtime crate. Without a
/// resolver, retain persisted config for degraded handling.
fn apply_runtime_overlay(config: &mut ClientConfig) {
    apply_runtime_overlay_for(config, halogen_webui_platform::namespace::active());
}
fn apply_runtime_overlay_for(config: &mut ClientConfig, _user_id: Option<i32>) {
    if config.server_kind != ServerKind::Embedded {
        return;
    }
    config.manual_offline = false;
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(resolver) = EMBEDDED_URL_RESOLVER.get()
        && let Some(id) = _user_id
        && let Some(url) = resolver(id)
    {
        config.server_url = Some(url);
    }
}

/// The host-installed "current embedded base URL" source (`ui-state` installs
/// it at boot when built with the `local-runtime` feature). A plain fn
/// pointer: the resolver reads process-global supervisor state, no capture
/// needed.
#[cfg(not(target_arch = "wasm32"))]
static EMBEDDED_URL_RESOLVER: std::sync::OnceLock<fn(i32) -> Option<String>> =
    std::sync::OnceLock::new();

/// Install the local-runtime URL resolver (first install wins; later calls
/// are no-ops — there is only one supervisor per process).
#[cfg(not(target_arch = "wasm32"))]
pub fn set_embedded_url_resolver(resolver: fn(i32) -> Option<String>) {
    let _ = EMBEDDED_URL_RESOLVER.set(resolver);
}

/// Native path for a per-user store: `<config root>/{segment}/{file}`, where the root comes from
/// `halogen_webui_platform::paths::config_root()` (XDG on Linux, `Library/Application Support` on macOS/iOS, the app
/// files dir on Android, never a panic, never the CWD). (Web uses IndexedDB, see `crate::web`, and needs no path
/// helper.)
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn ns_path(segment: &str, file: &str) -> std::path::PathBuf {
    halogen_webui_platform::paths::config_root()
        .join(segment)
        .join(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DownloadChunkSize;

    #[test]
    fn config_default_is_unauthenticated() {
        let config = ClientConfig::default();
        assert!(!config.is_authenticated());
        assert!(config.server_url.is_none());
        assert!(config.access_token.is_none());
        assert!(!config.server_setup);
    }

    #[test]
    fn config_is_authenticated_when_setup_and_token() {
        let config = ClientConfig {
            server_url: Some("https://example.com".into()),
            server_kind: ServerKind::Remote,
            access_token: Some("jwt".into()),
            refresh_jwt: None,
            server_setup: true,
            is_admin: false,
            nav: NavConfig::default(),
            playback_prefs: PlaybackPrefs::default(),
            font_size: FontSize::default(),
            device_logs: DeviceLogConfig::default(),
            manual_offline: false,
            disabled_discover_providers: Vec::new(),
            swipe_prefs: SwipePrefs::default(),
            download_prefs: DownloadPrefs::default(),
        };
        assert!(config.is_authenticated());
    }

    #[test]
    fn config_roundtrip() {
        let config = ClientConfig {
            server_url: Some("https://example.com".into()),
            server_kind: ServerKind::Embedded,
            access_token: Some("mytoken".into()),
            refresh_jwt: None,
            server_setup: true,
            is_admin: false,
            nav: NavConfig::default(),
            playback_prefs: PlaybackPrefs::default(),
            font_size: FontSize::Medium,
            device_logs: DeviceLogConfig::default(),
            manual_offline: false,
            disabled_discover_providers: vec![halogen_wire::DiscoverProvider::Gpodder],
            swipe_prefs: SwipePrefs::default(),
            download_prefs: DownloadPrefs {
                chunk_size: DownloadChunkSize::EightMB,
                parallelism: 4,
            },
        };
        let json = serde_json::to_string(&config).unwrap();
        let loaded: ClientConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, loaded);
    }

    #[test]
    fn config_roundtrip_defaults() {
        let config = ClientConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let loaded: ClientConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, loaded);
    }

    /// A pre-feature persisted config (no `server_kind` field) loads as
    /// `Remote` — the backward-compat contract for every existing install.
    #[test]
    fn config_without_server_kind_deserializes_as_remote() {
        let json = r#"{"server_url":"https://example.com","access_token":"t","refresh_jwt":null,"server_setup":true}"#;
        let loaded: ClientConfig = serde_json::from_str(json).unwrap();
        assert_eq!(loaded.server_kind, ServerKind::Remote);
        assert!(loaded.is_authenticated());
    }

    /// The load-time overlay rewrites an Embedded config's runtime fields —
    /// live loopback URL in, `manual_offline` forced off — and leaves Remote
    /// configs alone.
    #[test]
    fn runtime_overlay_rewrites_embedded_only() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            fn test_resolver(id: i32) -> Option<String> {
                Some(format!("halogen-local://{id}"))
            }
            set_embedded_url_resolver(test_resolver);
        }

        let mut embedded = ClientConfig {
            server_url: Some("http://127.0.0.1:1".into()),
            server_kind: ServerKind::Embedded,
            manual_offline: true,
            ..Default::default()
        };
        apply_runtime_overlay_for(&mut embedded, Some(42));
        assert!(!embedded.manual_offline, "Go Offline forced off");
        #[cfg(not(target_arch = "wasm32"))]
        assert_eq!(
            embedded.server_url.as_deref(),
            Some("halogen-local://42"),
            "local profile identity overlaid"
        );

        let mut remote = ClientConfig {
            server_url: Some("https://example.com".into()),
            manual_offline: true,
            ..Default::default()
        };
        apply_runtime_overlay(&mut remote);
        assert!(remote.manual_offline, "Remote configs untouched");
        assert_eq!(remote.server_url.as_deref(), Some("https://example.com"));
    }
}

// Native fs behavior of the degraded-storage latch, against a real temp config
// root via the `HALOGEN_CONFIG_DIR` override (safe under nextest's
// process-per-test model; `paths` memoizes per process).
#[cfg(all(test, not(target_arch = "wasm32")))]
mod degraded_storage_tests {
    use futures::executor::block_on;

    use super::*;

    #[test]
    fn load_failure_latches_and_refuses_saves_until_a_load_succeeds() {
        let root = std::env::temp_dir().join(format!(
            "halogen-config-degraded-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("config root");
        unsafe {
            std::env::set_var("HALOGEN_CONFIG_DIR", &root);
            std::env::set_var("HALOGEN_DATA_DIR", &root);
        }
        // Ambient namespace defaults to `anon` (no active user in this process).
        let path = ns_path("anon", "client.json");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("segment dir");

        // A CORRUPT stored config (real record, unreadable) must not silently
        // become Default — and must latch the degraded state.
        std::fs::write(&path, b"{ not json").expect("seed corrupt config");
        assert!(block_on(ClientConfigStore::try_load()).is_err());
        assert!(config_storage_degraded());

        // While latched, saves are refused: the corrupt-but-real record
        // survives (recoverable by hand / a later release) instead of being
        // overwritten by the faked default.
        block_on(ClientConfigStore::save(&ClientConfig::default()));
        assert_eq!(
            std::fs::read(&path).expect("stored config"),
            b"{ not json",
            "a save while degraded must not touch the stored record"
        );

        // Once a load succeeds (record repaired), the latch clears and saves
        // work again.
        std::fs::write(
            &path,
            serde_json::to_vec(&ClientConfig::default()).expect("serialize default"),
        )
        .expect("repair config");
        assert!(block_on(ClientConfigStore::try_load()).is_ok());
        assert!(!config_storage_degraded());
        let cfg = ClientConfig {
            server_url: Some("https://example.com".into()),
            ..Default::default()
        };
        block_on(ClientConfigStore::save(&cfg));
        let saved: ClientConfig =
            serde_json::from_slice(&std::fs::read(&path).expect("stored config"))
                .expect("saved config parses");
        assert_eq!(saved.server_url.as_deref(), Some("https://example.com"));

        // An absent record is NOT an error — first boot must stay clean.
        std::fs::remove_file(&path).expect("remove");
        assert!(block_on(ClientConfigStore::try_load()).is_ok());
        assert!(!config_storage_degraded());
    }
}
