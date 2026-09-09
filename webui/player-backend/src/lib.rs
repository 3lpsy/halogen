pub use halogen_webui_player_types::*;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::NoopPlayerBackend;
#[cfg(target_arch = "wasm32")]
pub mod web;
#[cfg(all(not(target_arch = "wasm32"), feature = "desktop"))]
pub mod webview;
#[cfg(all(not(target_arch = "wasm32"), feature = "desktop"))]
pub use webview::WebviewPlayerBackend;
