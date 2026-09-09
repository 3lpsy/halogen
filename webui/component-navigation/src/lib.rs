use dioxus::prelude::*;

use halogen_webui_component_icons::{
    Bolt, ClockRotateLeft, DocumentText, Download, Gear, ListUl, MagnifyingGlass, Music, Podcast,
    ShieldHalved,
};
use halogen_webui_config::{BuiltinNav, NavKey};

/// Render the Font Awesome icon for a nav key, sized via `class`. Used by the
/// sidebar, dock, and the mobile menu so all nav surfaces share one icon set.
pub fn nav_icon(key: &NavKey, class: &'static str) -> Element {
    match key {
        NavKey::Builtin(BuiltinNav::Queue) => rsx! { ListUl { class } },
        NavKey::Builtin(BuiltinNav::Latest) => rsx! { Bolt { class } },
        NavKey::Builtin(BuiltinNav::Podcasts) => rsx! { Podcast { class } },
        NavKey::Builtin(BuiltinNav::Playlists) => rsx! { Music { class } },
        NavKey::Builtin(BuiltinNav::Downloads) => rsx! { Download { class } },
        NavKey::Builtin(BuiltinNav::Discover) => rsx! { MagnifyingGlass { class } },
        NavKey::Builtin(BuiltinNav::History) => rsx! { ClockRotateLeft { class } },
        NavKey::Builtin(BuiltinNav::Settings) => rsx! { Gear { class } },
        NavKey::Builtin(BuiltinNav::Polling) => rsx! { ShieldHalved { class } },
        NavKey::Builtin(BuiltinNav::DeviceLogs) => rsx! { DocumentText { class } },
        NavKey::Builtin(BuiltinNav::ServerLogs) => rsx! { DocumentText { class } },
        NavKey::Pin(_) => rsx! { Music { class } },
    }
}

/// A single navigation item used by the sidebar, dock, and mobile menu.
#[derive(Clone, Debug, PartialEq)]
pub struct NavItem {
    pub key: NavKey,
    pub label: String,
    pub route: String,
    pub admin_only: bool,
}

/// Resolve a builtin nav key to its label and route. Icons are rendered by
/// [`nav_icon`] keyed on the [`NavKey`], not stored on the item.
fn builtin_to_item(key: &NavKey, is_admin: bool) -> Option<NavItem> {
    let (label, route, admin_only) = match key {
        NavKey::Builtin(BuiltinNav::Queue) => ("Queue".into(), "/queue".to_owned(), false),
        NavKey::Builtin(BuiltinNav::Latest) => ("Latest".into(), "/latest".to_owned(), false),
        NavKey::Builtin(BuiltinNav::Podcasts) => ("Podcasts".into(), "/podcasts".to_owned(), false),
        NavKey::Builtin(BuiltinNav::Playlists) => {
            ("Playlists".into(), "/playlists".to_owned(), false)
        }
        NavKey::Builtin(BuiltinNav::Downloads) => {
            ("Downloads".into(), "/downloads".to_owned(), false)
        }
        NavKey::Builtin(BuiltinNav::Discover) => ("Discover".into(), "/discover".to_owned(), false),
        NavKey::Builtin(BuiltinNav::History) => ("History".into(), "/history".to_owned(), false),
        NavKey::Builtin(BuiltinNav::Settings) => ("Settings".into(), "/settings".to_owned(), false),
        NavKey::Builtin(BuiltinNav::Polling) => ("Polling".into(), "/polling".to_owned(), true),
        NavKey::Builtin(BuiltinNav::DeviceLogs) => {
            ("Device Logs".into(), "/logs/device".to_owned(), false)
        }
        NavKey::Builtin(BuiltinNav::ServerLogs) => {
            ("Server Logs".into(), "/admin/logs".to_owned(), true)
        }
        NavKey::Pin(_) => return None,
    };
    if admin_only && !is_admin {
        return None;
    }
    Some(NavItem {
        key: key.clone(),
        label,
        route,
        admin_only,
    })
}

/// The display label for `key` if it's a builtin this user may configure on the
/// configure-dock screen (admin-only items resolve to `None` for non-admins, and
/// pins are managed elsewhere so also `None`). Pairs with [`nav_icon`].
pub fn nav_label(key: &NavKey, is_admin: bool) -> Option<String> {
    builtin_to_item(key, is_admin).map(|i| i.label)
}

/// Produce the ordered list of nav items from config. Applies `order`, drops `hidden`, drops admin-only items for
/// non-admins, and appends pins (resolved from `pinned_playlists` with a placeholder label since we don't have playlist
/// data at this layer).
pub fn nav_items(config: &halogen_webui_config::ClientConfig, is_admin: bool) -> Vec<NavItem> {
    let mut items = Vec::new();
    for key in &config.nav.order {
        // Skip hidden
        if config.nav.hidden.contains(key) {
            continue;
        }
        // `builtin_to_item` already drops admin-only items (Polling, ServerLogs)
        // for non-admins, so no per-item admin check is needed here.
        if let Some(item) = builtin_to_item(key, is_admin) {
            items.push(item);
        }
    }
    // Append pins (label is a placeholder; real data comes from EpisodeState).
    // Each pin links to its own playlist, not the generic Playlists list.
    for pin_id in &config.nav.pinned_playlists {
        items.push(NavItem {
            key: NavKey::Pin(*pin_id),
            label: format!("Playlist #{pin_id}"),
            route: format!("/playlists/{pin_id}"),
            admin_only: false,
        });
    }
    items
}
