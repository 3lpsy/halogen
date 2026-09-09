use chrono::{DateTime, Utc};
use chrono_tz::Tz;

pub fn dt_human(timestamp: DateTime<Utc>, timezone: &Tz) -> String {
    let dt_local = timestamp.with_timezone(timezone);
    dt_local.format("%B %d, %Y at %I:%M %p").to_string()
}
