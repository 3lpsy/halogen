pub mod playback_delete;
pub mod playback_get;
pub mod playback_list;
pub mod playback_store;

pub use playback_delete::handle as delete;
pub use playback_get::handle as get;
pub use playback_list::handle as list;
pub use playback_store::handle as store;

/// The playback-complete threshold (an episode is `Finished` once playback
/// reaches the last N% of its duration), layered as an `Extension` so the
/// playback upsert handler can maintain the caller's `user_episode_status` row.
#[derive(Clone, Copy)]
pub struct PlaybackCompleteConfig(pub u16);
