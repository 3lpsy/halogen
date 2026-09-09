//! Playback controller and queue navigation. Audio backends, sleep policy, and
//! OS media controls live in dedicated player crates.

mod controller;
mod navigation;
mod playback;
mod scope_bound;
pub use halogen_webui_player_backend::*;
use halogen_webui_player_sleep as sleep;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;

pub use controller::PlayerController;

pub use scope_bound::ScopeBound;
pub use sleep::{SleepState, TICK_INTERVAL_MS};
