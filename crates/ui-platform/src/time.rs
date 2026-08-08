//! Cross-target async timers (native `tokio` vs wasm `gloo`), shared by the UI
//! code that needs a delay regardless of platform.

/// Sleep for `ms` milliseconds on either target.
pub async fn sleep_ms(ms: u32) {
    #[cfg(not(target_arch = "wasm32"))]
    tokio::time::sleep(std::time::Duration::from_millis(ms as u64)).await;
    #[cfg(target_arch = "wasm32")]
    gloo_timers::future::TimeoutFuture::new(ms).await;
}

/// Wall-clock milliseconds since the Unix epoch, on either target.
///
/// Used for coarse recency / staleness checks (e.g. "did we already pull within
/// the last couple of seconds?") — NOT for precise interval measurement. The wasm
/// path uses `Date.now()`; the native path uses `SystemTime`. A clock that jumps
/// backwards can at worst make a recency window expire early (a redundant refresh),
/// never a correctness bug, so wall-clock is fine here.
pub fn now_ms() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as u64
    }
}
