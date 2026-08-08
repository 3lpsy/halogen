#![allow(clippy::module_inception, clippy::too_many_arguments)]

//! `halogen-ui-state` — the reactive coordination layer of the UI: app-wide
//! context `providers`, the typed `hooks` that read them, and the worker-`commands`
//! façade. Wires the sync worker (`ui-svc-sync`) and depends on the data/service
//! crates; consumed by the view crates (`ui-widgets`, `ui-episode-list`,
//! `ui-views`).

pub mod commands;
pub mod embedded;
pub mod embedded_session;
pub mod hooks;
pub mod providers;
