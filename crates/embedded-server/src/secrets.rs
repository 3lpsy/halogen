//! Generated auth material, persisted once at first boot: the JWT signing
//! secret (must stay stable across restarts or every session dies) and the
//! local admin's credentials (the user never sees a password — silent login
//! uses these).
//!
//! Threat model: anything that can read this file can already read the SQLite
//! DB next to it and the per-user `client.json` holding a live bearer token —
//! parity with existing client-side storage, not a regression. Written 0600
//! on unix, atomically (temp sibling + rename) like the config overrides file.

use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};
use rand::Rng;
use rand::distributions::Alphanumeric;
use serde::{Deserialize, Serialize};

use crate::dirs::EmbeddedDirs;

/// The admin credentials for silent login, surfaced to the UI host.
#[derive(Clone, Debug)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// Username of the provisioned admin. Deliberately not "admin" so embedded
/// accounts read distinctly in the account switcher next to a remote server's
/// conventional `admin`.
pub(crate) const LOCAL_ADMIN_USERNAME: &str = "local";

/// One additional (non-seed) embedded user's silent-login credential.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct StoredUserSecret {
    pub username: String,
    pub password: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Secrets {
    pub version: u32,
    pub auth_token_secret: String,
    pub admin_username: String,
    pub admin_password: String,
    pub created_at: DateTime<Utc>,
    /// Users created through the embedded add-account flow, each with their
    /// generated password (same threat model as the admin fields above).
    /// `serde(default)` keeps first-boot v1 files loading unchanged.
    #[serde(default)]
    pub users: Vec<StoredUserSecret>,
}

impl Secrets {
    pub(crate) fn generate() -> Self {
        Self {
            version: 1,
            // ~380 bits — the HS256 signing key.
            auth_token_secret: random_alnum(64),
            admin_username: LOCAL_ADMIN_USERNAME.to_string(),
            // The server's own password generator — ONE policy source, so a
            // future server-side password rule can't strand machine-managed
            // embedded credentials.
            admin_password: halogen_orm::user::generate_password(),
            created_at: Utc::now(),
            users: Vec::new(),
        }
    }

    /// The stored password for `username` (the seeded admin or an added user).
    pub(crate) fn password_for(&self, username: &str) -> Option<&str> {
        if username == self.admin_username {
            return Some(&self.admin_password);
        }
        self.users
            .iter()
            .find(|u| u.username == username)
            .map(|u| u.password.as_str())
    }

    /// Upsert `username`'s stored password.
    pub(crate) fn set_password(&mut self, username: &str, password: String) {
        if username == self.admin_username {
            self.admin_password = password;
            return;
        }
        match self.users.iter_mut().find(|u| u.username == username) {
            Some(slot) => slot.password = password,
            None => self.users.push(StoredUserSecret {
                username: username.to_string(),
                password,
            }),
        }
    }

    /// Read the persisted secrets; `None` when never provisioned.
    pub(crate) fn load(dirs: &EmbeddedDirs) -> Result<Option<Self>, String> {
        let path = dirs.secrets_path();
        if !path.exists() {
            return Ok(None);
        }
        let body = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
        let secrets: Secrets = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse {}: {e}", path.display()))?;
        Ok(Some(secrets))
    }

    /// Load, or generate + persist on first boot.
    pub(crate) fn load_or_create(dirs: &EmbeddedDirs) -> Result<Self, String> {
        if let Some(existing) = Self::load(dirs)? {
            return Ok(existing);
        }
        let fresh = Self::generate();
        fresh.save(dirs)?;
        Ok(fresh)
    }

    pub(crate) fn save(&self, dirs: &EmbeddedDirs) -> Result<(), String> {
        fs::create_dir_all(dirs.root())
            .map_err(|e| format!("Failed to create {}: {e}", dirs.root().display()))?;
        let path = dirs.secrets_path();
        let body = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize secrets: {e}"))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, body.as_bytes())
            .map_err(|e| format!("Failed to write {}: {e}", tmp.display()))?;
        restrict_permissions(&tmp);
        fs::rename(&tmp, &path)
            .map_err(|e| format!("Failed to persist {}: {e}", path.display()))?;
        Ok(())
    }

    /// New random admin password (recovery path) — the signing secret is
    /// deliberately untouched so existing sessions survive.
    pub(crate) fn rotate_password(&mut self) {
        self.admin_password = Self::fresh_password();
    }

    /// A fresh generated password (the server's generator — one policy source).
    pub(crate) fn fresh_password() -> String {
        halogen_orm::user::generate_password()
    }

    pub(crate) fn credentials(&self) -> Credentials {
        Credentials {
            username: self.admin_username.clone(),
            password: self.admin_password.clone(),
        }
    }
}

fn random_alnum(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

/// 0600 on unix; elsewhere the app-private data dir is the boundary.
fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}
