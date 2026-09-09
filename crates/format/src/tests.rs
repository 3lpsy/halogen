use super::*;
use chrono::{TimeZone, Utc};
// --- dt_human ------------------------------------------------------------

#[test]
fn dt_human_formats_fixed_timestamp_in_utc() {
    // Fixed instant: 2021-03-04 09:05:00 UTC.
    let ts = Utc.with_ymd_and_hms(2021, 3, 4, 9, 5, 0).unwrap();
    let out = dt_human(ts, &chrono_tz::UTC);
    assert_eq!(out, "March 04, 2021 at 09:05 AM");
}

#[test]
fn dt_human_respects_timezone_offset() {
    // 2021-03-04 09:05:00 UTC is 04:05 AM in New York (EST, -5).
    let ts = Utc.with_ymd_and_hms(2021, 3, 4, 9, 5, 0).unwrap();
    let out = dt_human(ts, &chrono_tz::America::New_York);
    assert_eq!(out, "March 04, 2021 at 04:05 AM");
}
