use crate::Route;
use crate::components::{Dock, Navbar, Sidebar};
use dioxus::prelude::*;

#[component]
pub fn AppLayout() -> Element {
    halogen_webui_page_app::use_app_layout::<Route>(
        rsx! { Navbar {} },
        rsx! { Sidebar {} },
        rsx! { Dock {} },
    )
}
