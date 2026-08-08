//! Multi-account registry — the device-global list of users authenticated on
//! this device, plus which one is active.
//!
//! This is a thin **index**, not a store of user state: every user's full config
//! (server URL, token, prefs) lives at its own namespaced key
//! (`halogen.u{id}.client_config`, via [`halogen_ui_config::ClientConfigStore`]),
//! kept current by that user's own session saves. The registry only records the
//! *set of switchable users* and *who is active* — flipping `active_user_id` is
//! what hot-swaps the mounted namespace (see `providers::AccountsProvider`).
//!
//! Neither the API token nor the media cookie is stored here: the token rides in
//! the per-user `ClientConfig`, and the cookie is `HttpOnly` (unreadable) —
//! re-minted by POSTing `/auth/refresh` on a switch.

use serde::{Deserialize, Serialize};

use halogen_ui_config::{AccountKey, ServerKind};

/// One switchable account on this device — just the switcher-list metadata. The
/// token, admin flag, and prefs all live in this account's namespaced
/// `ClientConfig` (`halogen.u{id}.client_config` / `halogen.e{id}.…`), not here.
/// `needs_reauth` is set when a stored token is rejected on a switch, so the UI
/// can prefill the login with `username`.
///
/// Identity is the `(kind, id)` pair, not the bare id: user ids are per-server
/// (a remote server's admin and the embedded server's admin share the same
/// fixture-fixed id), so id-only keying would collapse them onto one registry
/// slot + storage namespace. `kind` defaults to `Remote` so pre-feature
/// registries deserialize unchanged.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StoredAccount {
    pub id: i32,
    pub username: String,
    #[serde(default)]
    pub needs_reauth: bool,
    #[serde(default)]
    pub kind: ServerKind,
    /// Hash of the remote server's URL (0 for embedded). Part of the identity so
    /// accounts on different remote servers with the same user id stay distinct.
    /// See [`AccountKey::server`].
    #[serde(default)]
    pub server: u64,
}

impl StoredAccount {
    /// This account's composite identity.
    pub fn key(&self) -> AccountKey {
        AccountKey {
            kind: self.kind,
            id: self.id,
            server: self.server,
        }
    }
}

/// Device-global account registry: the switchable-user list + active user. Server
/// URL / setup / token are deliberately absent (they stay in `ClientConfig`).
#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Accounts {
    /// The user whose namespace is currently mounted. `None` = signed out.
    /// Read through [`active_key`](Self::active_key) — the id alone is
    /// ambiguous across server kinds; `active_kind` rides beside it (defaults
    /// `Remote` for pre-feature registries).
    pub active_user_id: Option<i32>,
    /// Which kind of server the active user belongs to.
    #[serde(default)]
    pub active_kind: ServerKind,
    /// The active REMOTE account's server hash (0 for embedded / signed out).
    #[serde(default)]
    pub active_server: u64,
    /// Every authenticated account, in add order.
    #[serde(default)]
    pub users: Vec<StoredAccount>,
}

impl Accounts {
    /// The active account's composite identity, `None` when signed out.
    pub fn active_key(&self) -> Option<AccountKey> {
        self.active_user_id.map(|id| AccountKey {
            kind: self.active_kind,
            id,
            server: self.active_server,
        })
    }

    /// Set (or clear) the active account. The raw fields always move together —
    /// use this, not field writes.
    pub fn set_active(&mut self, key: Option<AccountKey>) {
        self.active_user_id = key.map(|k| k.id);
        self.active_kind = key.map(|k| k.kind).unwrap_or_default();
        self.active_server = key.map(|k| k.server).unwrap_or(0);
    }

    /// Insert or replace an account by `(kind, id)` (preserving add order on
    /// replace).
    pub fn upsert(&mut self, account: StoredAccount) {
        match self.users.iter_mut().find(|u| u.key() == account.key()) {
            Some(slot) => *slot = account,
            None => self.users.push(account),
        }
    }

    /// Remove an account. If it was active, clears the active key (the caller
    /// decides what to switch to, or triggers sign-out when none remain).
    pub fn remove(&mut self, key: AccountKey) {
        self.users.retain(|u| u.key() != key);
        if self.active_key() == Some(key) {
            self.set_active(None);
        }
    }
}

/// Decode the `sub` (user id) claim from a JWT **without** verifying the
/// signature. The client only needs the id to key local storage; the server
/// verifies every request. Returns `None` for a non-JWT / malformed token.
pub fn jwt_sub(token: &str) -> Option<i32> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    // `sub` may be encoded as a JSON string ("42") or a JSON number (42)
    // depending on the issuer — accept both so users don't silently collapse
    // onto the legacy fallback namespace.
    let sub = json.get("sub")?;
    // `try_from` (not `as i32`) so an out-of-range id is rejected rather than
    // silently wrapped onto another user's `u{id}` namespace.
    sub.as_str()
        .and_then(|s| s.parse::<i32>().ok())
        .or_else(|| sub.as_i64().and_then(|n| i32::try_from(n).ok()))
}

/// Persistent storage backend for [`Accounts`].
///
/// Web: IndexedDB `halogen.accounts`, store `registry`, record `"accounts"`.
/// Native: JSON file at `<config root>/accounts.json` (see
/// `halogen_ui_platform::paths::config_root`).
pub struct AccountsStore;

impl AccountsStore {
    /// Load the registry (a fresh default when nothing is stored — e.g. first run).
    pub async fn load() -> Accounts {
        Self::load_raw().await.unwrap_or_default()
    }

    /// Persist the registry.
    pub async fn save(accounts: &Accounts) {
        #[cfg(target_arch = "wasm32")]
        crate::web::save(accounts).await;
        #[cfg(not(target_arch = "wasm32"))]
        halogen_ui_platform::kv_save!("", accounts_path(), accounts);
    }

    /// Read the persisted registry, `None` if absent/unreadable.
    async fn load_raw() -> Option<Accounts> {
        #[cfg(target_arch = "wasm32")]
        {
            crate::web::load().await
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            halogen_ui_platform::kv_load_opt!(Accounts, "", accounts_path())
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn accounts_path() -> std::path::PathBuf {
    halogen_ui_platform::paths::config_root().join("accounts.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn make_token(payload: &str) -> String {
        let b = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload);
        format!("header.{b}.sig")
    }

    #[test]
    fn parses_integer_sub() {
        let t = make_token(r#"{"sub":"42","exp":123}"#);
        assert_eq!(jwt_sub(&t), Some(42));
    }

    #[test]
    fn rejects_malformed() {
        assert_eq!(jwt_sub("notajwt"), None);
        assert_eq!(jwt_sub("a.b.c"), None);
        assert_eq!(jwt_sub(&make_token(r#"{"exp":1}"#)), None);
        assert_eq!(jwt_sub(&make_token(r#"{"sub":"abc"}"#)), None);
    }

    #[test]
    fn upsert_replaces_and_remove_clears_active() {
        let mut accounts = Accounts::default();
        accounts.upsert(StoredAccount {
            id: 1,
            username: "a".into(),
            needs_reauth: false,
            kind: ServerKind::Remote,
            server: 0,
        });
        accounts.upsert(StoredAccount {
            id: 1,
            username: "a2".into(),
            needs_reauth: false,
            kind: ServerKind::Remote,
            server: 0,
        });
        assert_eq!(
            accounts.users.len(),
            1,
            "upsert replaces by (kind, id, server)"
        );
        assert_eq!(accounts.users[0].username, "a2");
        accounts.set_active(Some(AccountKey::remote(1, 0)));
        accounts.remove(AccountKey::remote(1, 0));
        assert!(accounts.users.is_empty());
        assert_eq!(accounts.active_key(), None, "removing active clears it");
    }

    /// A remote and an embedded account with the SAME numeric id coexist —
    /// the exact collision the composite key exists for (both servers seed
    /// their admin at one fixed id).
    #[test]
    fn same_id_different_kind_are_distinct_accounts() {
        let mut accounts = Accounts::default();
        accounts.upsert(StoredAccount {
            id: i32::MAX,
            username: "admin".into(),
            needs_reauth: false,
            kind: ServerKind::Remote,
            server: 0,
        });
        accounts.upsert(StoredAccount {
            id: i32::MAX,
            username: "local".into(),
            needs_reauth: false,
            kind: ServerKind::Embedded,
            server: 0,
        });
        assert_eq!(accounts.users.len(), 2, "no collapse across kinds");

        accounts.set_active(Some(AccountKey::embedded(i32::MAX)));
        accounts.remove(AccountKey::remote(i32::MAX, 0));
        assert_eq!(accounts.users.len(), 1, "only the remote entry removed");
        assert_eq!(
            accounts.active_key(),
            Some(AccountKey::embedded(i32::MAX)),
            "embedded stays active"
        );
        assert_ne!(
            AccountKey::remote(i32::MAX, 0).segment(),
            AccountKey::embedded(i32::MAX).segment(),
            "storage namespaces are partitioned"
        );
    }

    /// Two accounts with the SAME user id on DIFFERENT remote servers coexist as
    /// distinct entries with distinct storage namespaces — the collision the
    /// `server` component of the key exists to prevent (H5).
    #[test]
    fn same_id_different_remote_server_are_distinct() {
        let sx = halogen_ui_config::server_hash("https://server-x.example");
        let sy = halogen_ui_config::server_hash("https://server-y.example");
        assert_ne!(sx, sy, "distinct servers hash differently");

        let mut accounts = Accounts::default();
        accounts.upsert(StoredAccount {
            id: 1,
            username: "admin@x".into(),
            needs_reauth: false,
            kind: ServerKind::Remote,
            server: sx,
        });
        accounts.upsert(StoredAccount {
            id: 1,
            username: "admin@y".into(),
            needs_reauth: false,
            kind: ServerKind::Remote,
            server: sy,
        });
        assert_eq!(
            accounts.users.len(),
            2,
            "same id, different server → 2 slots"
        );
        assert_ne!(
            AccountKey::remote(1, sx).segment(),
            AccountKey::remote(1, sy).segment(),
            "storage namespaces are partitioned by server"
        );

        // Removing one leaves the other intact.
        accounts.remove(AccountKey::remote(1, sx));
        assert_eq!(accounts.users.len(), 1);
        assert_eq!(accounts.users[0].username, "admin@y");
    }

    /// A pre-feature registry (no `kind`/`active_kind` fields) loads with
    /// every account as Remote — the backward-compat contract.
    #[test]
    fn registry_without_kind_deserializes_as_remote() {
        let json = r#"{"active_user_id":3,"users":[{"id":3,"username":"a"}]}"#;
        let reg: Accounts = serde_json::from_str(json).unwrap();
        assert_eq!(reg.active_key(), Some(AccountKey::remote(3, 0)));
        assert_eq!(reg.users[0].kind, ServerKind::Remote);
        assert_eq!(reg.users[0].key(), AccountKey::remote(3, 0));
    }
}
