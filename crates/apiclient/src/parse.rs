//! `Content-Range` / `Content-Length` header parsing helpers.

/// Parse the `<total>` out of a `Content-Range: bytes <start>-<end>/<total>`
/// header value. `None` for an unknown total (`*`) or a malformed header.
pub(crate) fn parse_content_range_total(value: &str) -> Option<u64> {
    value.rsplit('/').next()?.trim().parse::<u64>().ok()
}

/// Parse the `<start>` out of a `Content-Range: bytes <start>-<end>/<total>` header
/// value. `None` for an unsatisfied (`*`) or malformed header.
pub(crate) fn parse_content_range_start(value: &str) -> Option<u64> {
    let v = value.trim();
    v.strip_prefix("bytes ")
        .unwrap_or(v)
        .split('-')
        .next()?
        .trim()
        .parse::<u64>()
        .ok()
}
