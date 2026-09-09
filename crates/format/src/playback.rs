/// Format a playback position as minutes and seconds.
pub fn format_time(secs: f64) -> String {
    let mins = (secs / 60.0) as i32;
    let secs = (secs % 60.0) as i32;
    format!("{mins}:{secs:02}")
}
