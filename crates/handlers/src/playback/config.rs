/// The playback-complete threshold (an episode is `Finished` once playback
/// reaches the last N% of its duration), layered as an `Extension` so the
/// playback upsert handler can maintain the caller's `user_episode_status` row.
#[derive(Clone, Copy)]
pub struct PlaybackCompleteConfig(pub u16);
