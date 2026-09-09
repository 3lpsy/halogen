#[cfg(target_arch = "wasm32")]
pub mod media_session;
#[cfg(all(not(target_arch = "wasm32"), feature = "desktop"))]
pub mod media_session_webview;
