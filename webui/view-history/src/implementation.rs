use dioxus::prelude::*;

use crate::components::{ListSource, OrderDirection, SortField, SortSpec};
use halogen_webui_hooks::use_config;

use super::ListPage;

/// History page — recently played episodes with progress.
#[component]
pub fn History() -> Element {
    let config = use_config();
    rsx! {
        ListPage {
            view_key: "history",
            source: ListSource::History,
            default_sort: SortSpec {
                field: SortField::UpdatedAt,
                direction: OrderDirection::Desc,
            },
            swipe: config().swipe_prefs.history,
        }
    }
}
