//! Playback service. Re-export shell; the shared value types live in [`types`],
//! the state machine in `controller`, and the per-target audio backends in
//! `web` (wasm `HtmlAudioElement`) / `webview` (native desktop: an
//! `<audio>` element in the app webview, driven over `document::eval`) /
//! `native` (renderless no-op for native builds without a webview renderer —
//! i.e. the `--no-default-features` test graph). OS lock-screen controls live
//! in `media_session` (wasm) / `media_session_webview` (native), with matching
//! public surfaces so the provider wires both targets identically.

mod controller;
mod navigation;
mod playback;
mod scope_bound;
mod sleep;
mod types;

#[cfg(target_arch = "wasm32")]
pub mod media_session;
#[cfg(target_arch = "wasm32")]
pub mod web;

// Native desktop: the app is a wry webview, so audio + media-session
// ride the webview's own media stack (hardware decode, pitch-preserving rate,
// `navigator.mediaSession`). Gated on the renderer features — the renderless
// native build (unit tests) has no webview to eval into.
#[cfg(all(not(target_arch = "wasm32"), feature = "desktop"))]
pub mod media_session_webview;
#[cfg(all(not(target_arch = "wasm32"), feature = "desktop"))]
pub mod webview;

// Renderless native (the `--no-default-features` test graph) keeps the no-op
// backend: nothing to play into without a webview.
#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(test)]
mod tests;

pub use controller::PlayerController;
#[cfg(not(target_arch = "wasm32"))]
pub use native::NoopPlayerBackend;
pub use scope_bound::ScopeBound;
pub use sleep::{SleepState, TICK_INTERVAL_MS};
pub use types::*;
#[cfg(all(not(target_arch = "wasm32"), feature = "desktop"))]
pub use webview::WebviewPlayerBackend;
