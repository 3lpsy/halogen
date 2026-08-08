use dioxus::prelude::*;

use super::ListPage;
use crate::components::{
    EpisodeFilter, FilterSpec, ListSource, OrderDirection, SortField, SortSpec,
};
use halogen_ui_state::hooks::use_config;

/// Downloads page — episodes downloaded to this device (the client-download set),
/// newest first. In embedded-server mode the source is the SERVER's download set
/// instead: the built-in server's library IS this device's library, and the
/// device-download concept doesn't exist there. Default swipes: left removes the
/// download, right adds to the queue (user-configurable via
/// `swipe_prefs.downloads`; device swipe actions remap to their server
/// counterparts in embedded mode).
///
/// Reuses the shared [`ListPage`] scaffold; the only extra behavior is a one-shot
/// `sanitize_filter` cleanup (below) that drops a stale `OnDevice` filter token.
#[component]
pub fn Downloads() -> Element {
    let config = use_config();

    // Self-heal stale state: earlier builds seeded an OnDevice token here and may
    // have persisted it. The chip is no longer offered, so drop the token to avoid
    // a phantom filter badge (and the now-invisible Downloading-strip it triggers).
    // Runs once on mount via `ListPage`'s `sanitize_filter` hook — value-in/
    // value-out (ListPage owns its signals and commits the result itself; passing
    // the child's signals into this parent-owned callback tripped dioxus'
    // copy-value-hoist warning).
    let cleanup = use_callback(|mut filter: FilterSpec| {
        filter.filters.retain(|f| *f != EpisodeFilter::OnDevice);
        filter
    });

    // The list source itself IS the download set, so there's no OnDevice filter
    // (it would be a no-op). Newest first; sort/filter remembered across visits.
    let source = if config().server_kind.is_embedded() {
        ListSource::ServerDownloads
    } else {
        ListSource::ClientDownloads
    };

    rsx! {
        ListPage {
            view_key: "downloads",
            source,
            default_sort: SortSpec {
                field: SortField::PublishedAt,
                direction: OrderDirection::Desc,
            },
            swipe: config().swipe_prefs.downloads,
            sanitize_filter: Some(cleanup),
        }
    }
}
