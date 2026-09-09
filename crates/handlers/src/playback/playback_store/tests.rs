use super::*;

#[test]
fn playback_status_for_covers_states_and_threshold() {
    use PlaybackStatus::*;
    let pct = 4; // Finished within the last 4%.

    // Explicit completion wins regardless of cursor/duration.
    assert_eq!(playback_status_for(0, true, Some(100), pct), Finished);
    assert_eq!(playback_status_for(0, true, None, pct), Finished);

    // Not started → Unplayed (even with a duration).
    assert_eq!(playback_status_for(0, false, Some(100), pct), Unplayed);

    // Started but no duration → can't compute remaining, so Played.
    assert_eq!(playback_status_for(50, false, None, pct), Played);
    assert_eq!(playback_status_for(50, false, Some(0), pct), Played);

    // Threshold boundary on a 100s episode: remaining ≤ 4% → Finished.
    assert_eq!(playback_status_for(96, false, Some(100), pct), Finished); // 4% left
    assert_eq!(playback_status_for(100, false, Some(100), pct), Finished); // 0% left
    assert_eq!(playback_status_for(95, false, Some(100), pct), Played); // 5% left
    assert_eq!(playback_status_for(1, false, Some(100), pct), Played); // just started
}
