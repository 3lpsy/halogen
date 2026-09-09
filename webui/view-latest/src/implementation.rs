use dioxus::prelude::*;

use crate::components::{ListSource, OrderDirection, SortField, SortSpec};
use halogen_webui_hooks::use_config;

use super::ListPage;

/// Latest page — newest episodes across all podcasts. Offline-first, server-paged:
/// the whole library no longer has to live in memory to browse the latest feed.
#[component]
pub fn Latest() -> Element {
    let config = use_config();
    rsx! {
        ListPage {
            view_key: "latest",
            source: ListSource::AllEpisodes,
            default_sort: SortSpec {
                field: SortField::PublishedAt,
                direction: OrderDirection::Desc,
            },
            swipe: config().swipe_prefs.latest,
        }
    }
}
