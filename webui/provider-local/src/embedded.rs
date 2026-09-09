//! Native library lifecycle. API requests use an in-process profile session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbeddedState {
    Unavailable,
    Stopped,
    Starting,
    Running,
    Failed { error: String },
}

#[derive(Clone, Debug)]
pub struct LocalProfile {
    pub id: i32,
    pub username: String,
    pub is_admin: bool,
    pub base: String,
}

#[cfg(all(not(target_arch = "wasm32"), feature = "local-runtime"))]
mod native;
#[cfg(all(not(target_arch = "wasm32"), feature = "local-runtime"))]
pub use native::*;

#[cfg(not(all(not(target_arch = "wasm32"), feature = "local-runtime")))]
mod stub {
    use super::{EmbeddedState, LocalProfile};
    pub fn new_password() -> String {
        String::new()
    }
    pub fn available() -> bool {
        false
    }
    pub fn library_exists() -> bool {
        false
    }
    pub fn install() {}
    pub fn ensure_started() -> Result<String, String> {
        Err("Local library is unavailable".into())
    }
    pub async fn wait_ready() -> Result<(), String> {
        Err("Local library is unavailable".into())
    }
    pub fn state() -> EmbeddedState {
        EmbeddedState::Unavailable
    }
    pub async fn stop() {}
    pub async fn destroy() -> Result<(), String> {
        Err("Local library is unavailable".into())
    }
    pub async fn profile(_: Option<&str>) -> Result<LocalProfile, String> {
        Err("Local library is unavailable".into())
    }
    pub fn data_dir_display() -> Option<String> {
        None
    }
}
#[cfg(not(all(not(target_arch = "wasm32"), feature = "local-runtime")))]
pub use stub::*;
