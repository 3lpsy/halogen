//! Client-side navigation ordering / visibility persisted in
//! [`ClientConfig`](super::ClientConfig).

use serde::{Deserialize, Serialize};

/// Client-side navigation configuration.
/// Controls the order, visibility, and pinning of nav items in the
/// sidebar and dock. Persisted to `ClientConfig` and saved on change.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NavConfig {
    /// User-defined ordering of builtin nav keys and pin ids.
    #[serde(default = "default_order")]
    pub order: Vec<NavKey>,
    /// Builtin nav keys the user chose to hide.
    #[serde(default = "default_hidden")]
    pub hidden: Vec<NavKey>,
    /// Playlist ids pinned as nav links.
    #[serde(default)]
    pub pinned_playlists: Vec<i32>,
}

impl Default for NavConfig {
    fn default() -> Self {
        Self {
            order: default_order(),
            hidden: default_hidden(),
            pinned_playlists: Vec::new(),
        }
    }
}

impl NavConfig {
    /// Self-heal a persisted order that lost builtin keys (a historical configure-dock bug could drop admin-only keys
    /// on Apply): append any missing [`default_order`] builtins at the end. A key absent from `order` is unreachable
    /// everywhere, the dock, sidebar, `/menu`, and the configure-dock screen all render only keys present in `order`.
    pub fn normalize(&mut self) {
        for key in default_order() {
            if !self.order.contains(&key) {
                self.order.push(key);
            }
        }
    }
}

/// The canonical builtin nav order. Includes the secondary destinations
/// (Polling, Device/Server logs) at the end — they're [`default_hidden`] so they
/// don't clutter the dock/sidebar until the user opts them in via the
/// configure-dock screen.
fn default_order() -> Vec<NavKey> {
    vec![
        NavKey::Builtin(BuiltinNav::Queue),
        NavKey::Builtin(BuiltinNav::Latest),
        NavKey::Builtin(BuiltinNav::Podcasts),
        NavKey::Builtin(BuiltinNav::Playlists),
        NavKey::Builtin(BuiltinNav::Downloads),
        NavKey::Builtin(BuiltinNav::Discover),
        NavKey::Builtin(BuiltinNav::History),
        NavKey::Builtin(BuiltinNav::Settings),
        NavKey::Builtin(BuiltinNav::Polling),
        NavKey::Builtin(BuiltinNav::DeviceLogs),
        NavKey::Builtin(BuiltinNav::ServerLogs),
    ]
}

/// Secondary destinations hidden out of the dock/sidebar by default; a user opts
/// them in from the configure-dock screen. (Admin-only ones are also gated by
/// `is_admin` regardless.)
fn default_hidden() -> Vec<NavKey> {
    vec![
        NavKey::Builtin(BuiltinNav::Polling),
        NavKey::Builtin(BuiltinNav::DeviceLogs),
        NavKey::Builtin(BuiltinNav::ServerLogs),
    ]
}

/// Stable identifier for a nav item (builtin or pinned playlist).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum NavKey {
    Builtin(BuiltinNav),
    Pin(i32),
}

/// Built-in navigation entries.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinNav {
    Queue,
    Latest,
    Podcasts,
    Playlists,
    Downloads,
    // Online-only podcast search/discovery.
    Discover,
    History,
    Settings,
    // Server polling history. Admin-only; hidden by default.
    Polling,
    // This device's captured logs. Hidden by default.
    DeviceLogs,
    // The server's application logs. Admin-only; hidden by default.
    ServerLogs,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_restores_dropped_builtins() {
        let mut nav = NavConfig::default();
        // Simulate the historical configure-dock corruption: admin-only keys
        // dropped from the persisted order.
        nav.order.retain(|k| {
            !matches!(
                k,
                NavKey::Builtin(BuiltinNav::Polling | BuiltinNav::ServerLogs)
            )
        });
        nav.normalize();
        assert!(nav.order.contains(&NavKey::Builtin(BuiltinNav::Polling)));
        assert!(nav.order.contains(&NavKey::Builtin(BuiltinNav::ServerLogs)));
        assert_eq!(nav.order.len(), default_order().len());
    }

    #[test]
    fn nav_config_default_order() {
        let nav = NavConfig::default();
        // 8 primary + 3 secondary (Polling, Device/Server logs) appended.
        assert_eq!(nav.order.len(), 11);
        assert_eq!(nav.order[0], NavKey::Builtin(BuiltinNav::Queue));
        assert_eq!(nav.order[1], NavKey::Builtin(BuiltinNav::Latest));
        // The 3 secondary destinations are hidden out of the dock/sidebar by default.
        assert_eq!(nav.hidden.len(), 3);
        assert!(nav.hidden.contains(&NavKey::Builtin(BuiltinNav::Polling)));
        assert!(nav.pinned_playlists.is_empty());
    }
}
