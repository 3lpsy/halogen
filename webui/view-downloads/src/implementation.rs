use dioxus::prelude::*;

use super::ListPage;
use crate::components::{
    EpisodeFilter, FilterSpec, ListSource, OrderDirection, SortField, SortSpec,
};
use halogen_webui_hooks::use_config;

/// Show device downloads newest first, or the local runtime's download set in local-only mode. Swipe preferences remap
/// device actions to local-runtime equivalents. [`ListPage`] also removes obsolete `OnDevice` filters on mount.
#[component]
pub fn Downloads() -> Element {
    let config = use_config();

    // Remove the obsolete `OnDevice` token on mount to avoid a phantom filter badge. Return the sanitized value to
    // `ListPage`; passing its signals into this parent callback violates Dioxus signal ownership.
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
