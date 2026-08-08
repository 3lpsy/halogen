//! The UI-facing facade over `halogen-embedded-server` — the ONLY place the
//! `embedded-server` cargo feature is checked. Views and session logic call
//! these functions unconditionally; on web, renderless test builds, or a
//! native build without the feature, the stub implementation compiles in
//! (`available() == false`, operations error) so no caller needs a cfg.
//!
//! `install()` registers the loopback-URL resolver with `halogen-ui-config`,
//! which applies it at config-load time (the "overlay"): loading an
//! `Embedded` config idempotently boots the in-process server (the port binds
//! synchronously, so the URL is live before the load returns) and substitutes
//! the current base URL for whatever port was persisted last session.

/// Where the embedded server is in its lifecycle, mirrored into a UI-owned
/// type so the stub build needs no `halogen-embedded-server` types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbeddedState {
    /// This build has no embedded server (web / feature off).
    Unavailable,
    Stopped,
    Starting {
        port: u16,
    },
    Running {
        port: u16,
    },
    Failed {
        error: String,
    },
}

/// The provisioned local admin's credentials (silent login only — never shown).
#[derive(Clone, Debug)]
pub struct EmbeddedCredentials {
    pub username: String,
    pub password: String,
}

#[cfg(all(not(target_arch = "wasm32"), feature = "embedded-server"))]
mod imp {
    use halogen_embedded_server as srv;
    use halogen_ui_platform::paths;

    use super::{EmbeddedCredentials, EmbeddedState};

    fn dirs() -> srv::EmbeddedDirs {
        srv::EmbeddedDirs::new(paths::embedded_server_root())
    }

    impl From<srv::Credentials> for EmbeddedCredentials {
        fn from(c: srv::Credentials) -> Self {
            EmbeddedCredentials {
                username: c.username,
                password: c.password,
            }
        }
    }

    pub fn available() -> bool {
        true
    }

    /// Whether an embedded library already exists on this device (drives the
    /// "reconnect vs create" confirmation copy).
    pub fn library_exists() -> bool {
        dirs().library_exists()
    }

    /// Register the config-load overlay resolver. Idempotent; called once at
    /// app boot, before the first config load.
    pub fn install() {
        halogen_ui_logging::info!("Embedded server support installed (resolver registered)");
        halogen_ui_config::set_embedded_url_resolver(resolver);
    }

    /// The overlay hook: loading an Embedded config *is* the demand signal for
    /// the server, so boot it (idempotent, port bound synchronously) and
    /// return the live base URL. On a start error fall back to the last-known
    /// URL — downstream treats the unreachable server as offline, and the
    /// Failed state is surfaced in Settings.
    fn resolver() -> Option<String> {
        match srv::ensure_started(dirs()) {
            Ok(url) => Some(url),
            Err(e) => {
                halogen_ui_logging::error!("Embedded server failed to start: {e}");
                srv::base_url()
            }
        }
    }

    pub fn ensure_started() -> Result<String, String> {
        srv::ensure_started(dirs())
    }

    pub async fn wait_ready() -> Result<(), String> {
        srv::wait_ready().await.map(|_port| ())
    }

    pub fn state() -> EmbeddedState {
        match srv::status() {
            srv::EmbeddedStatus::Stopped => EmbeddedState::Stopped,
            srv::EmbeddedStatus::Starting { port } => EmbeddedState::Starting { port },
            srv::EmbeddedStatus::Running { port } => EmbeddedState::Running { port },
            srv::EmbeddedStatus::Failed { error } => EmbeddedState::Failed { error },
        }
    }

    pub async fn stop() {
        srv::stop().await;
    }

    pub async fn destroy() -> Result<(), String> {
        srv::destroy(&dirs()).await
    }

    pub fn credentials() -> Result<EmbeddedCredentials, String> {
        srv::credentials(&dirs()).map(Into::into)
    }

    pub async fn recover_admin() -> Result<EmbeddedCredentials, String> {
        srv::recover_admin(&dirs()).await.map(Into::into)
    }

    /// Stored silent-login credentials for a specific embedded user.
    pub fn credentials_for(username: &str) -> Result<EmbeddedCredentials, String> {
        srv::credentials_for(&dirs(), username).map(Into::into)
    }

    /// Fresh credentials for a to-be-created embedded user — NOT yet
    /// persisted (persist with [`remember_user`] after the server-side create
    /// succeeds, so a failed create can't clobber an existing user's secret).
    pub fn generate_credentials(username: &str) -> EmbeddedCredentials {
        srv::generate_credentials(username).into()
    }

    /// Persist a user's silent-login credentials (after a successful create).
    pub fn remember_user(creds: &EmbeddedCredentials) -> Result<(), String> {
        srv::remember_user(
            &dirs(),
            &srv::Credentials {
                username: creds.username.clone(),
                password: creds.password.clone(),
            },
        )
    }

    /// Whether stored credentials exist for `username`.
    pub fn has_credentials(username: &str) -> bool {
        srv::has_credentials(&dirs(), username).unwrap_or(false)
    }

    /// Rotate a specific user's password in the DB + secrets (silent-login
    /// self-heal; also re-keys users a DB import created).
    pub async fn recover_user(username: &str) -> Result<EmbeddedCredentials, String> {
        srv::recover_user(&dirs(), username).await.map(Into::into)
    }

    /// The embedded data directory, for the Settings server page.
    pub fn data_dir_display() -> Option<String> {
        Some(paths::embedded_server_root().display().to_string())
    }
}

#[cfg(not(all(not(target_arch = "wasm32"), feature = "embedded-server")))]
mod imp {
    use super::{EmbeddedCredentials, EmbeddedState};

    const UNAVAILABLE: &str = "Embedded server is not available in this build";

    pub fn available() -> bool {
        false
    }

    pub fn library_exists() -> bool {
        false
    }

    pub fn install() {}

    pub fn ensure_started() -> Result<String, String> {
        Err(UNAVAILABLE.to_string())
    }

    pub async fn wait_ready() -> Result<(), String> {
        Err(UNAVAILABLE.to_string())
    }

    pub fn state() -> EmbeddedState {
        EmbeddedState::Unavailable
    }

    pub async fn stop() {}

    pub async fn destroy() -> Result<(), String> {
        Err(UNAVAILABLE.to_string())
    }

    pub fn credentials() -> Result<EmbeddedCredentials, String> {
        Err(UNAVAILABLE.to_string())
    }

    pub async fn recover_admin() -> Result<EmbeddedCredentials, String> {
        Err(UNAVAILABLE.to_string())
    }

    pub fn credentials_for(_username: &str) -> Result<EmbeddedCredentials, String> {
        Err(UNAVAILABLE.to_string())
    }

    pub fn generate_credentials(username: &str) -> EmbeddedCredentials {
        EmbeddedCredentials {
            username: username.to_lowercase(),
            password: String::new(),
        }
    }

    pub fn remember_user(_creds: &EmbeddedCredentials) -> Result<(), String> {
        Err(UNAVAILABLE.to_string())
    }

    pub fn has_credentials(_username: &str) -> bool {
        false
    }

    pub async fn recover_user(_username: &str) -> Result<EmbeddedCredentials, String> {
        Err(UNAVAILABLE.to_string())
    }

    pub fn data_dir_display() -> Option<String> {
        None
    }
}

pub use imp::{
    available, credentials, credentials_for, data_dir_display, destroy, ensure_started,
    generate_credentials, has_credentials, install, library_exists, recover_admin, recover_user,
    remember_user, state, stop, wait_ready,
};
