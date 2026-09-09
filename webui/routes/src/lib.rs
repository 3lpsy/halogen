//! `halogen-webui-routes`, the Route-coupled top of the view layer: the `Route` enum and router glue (`app`,
//! `root_guard`), every `pages` view, the app `layouts`, and the "smart" `components` that navigate via the `Route`
//! enum (nav/navbar/ sidebar/dock, the now-playing player UI, and the podcast/playlist context menus). Depends on every
//! lower UI crate; consumed only by the `halogen-webui` root.
#![allow(clippy::module_inception, clippy::too_many_arguments)]

pub mod app;
pub mod components;
pub mod layouts;
pub mod pages;
pub mod root_guard;
pub mod routes;

pub use app::App;
// Re-exported at the crate root so the moved view files keep using `crate::Route`,
// and the `halogen-webui` binary can name `halogen_webui_routes::Route`.
pub use routes::Route;
