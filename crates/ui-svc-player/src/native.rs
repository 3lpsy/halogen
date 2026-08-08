//! No-op audio backend for RENDERLESS native builds — the
//! `--no-default-features` unit-test graph, which links no webview renderer
//! and so has nothing to play into.
//!
//! Real native playback lives in `webview.rs` (behind the `desktop`/`mobile`
//! renderer features): an `<audio>` element in the app webview. Wasm uses
//! `WebPlayerBackend`.

use super::{MediaSource, PlayerBackend, PlayerEvent};

/// No-op backend for renderless native builds (tests).
pub struct NoopPlayerBackend;

impl PlayerBackend for NoopPlayerBackend {
    fn load(&self, _src: MediaSource, _start_at_secs: f64) {}
    fn play(&self) {}
    fn pause(&self) {}
    fn seek(&self, _secs: f64) {}
    fn set_rate(&self, _rate: f32) {}
    fn stop(&self) {}
    fn poll_events(&mut self) -> Vec<PlayerEvent> {
        Vec::new()
    }
}
