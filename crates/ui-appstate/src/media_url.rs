//! Server media/artwork URL builders.
//!
//! Free functions that take the server base explicitly — callers pass
//! `config.read().server_url.as_deref()`. Auth lives solely in `ClientConfig` (the
//! persisted source of truth), which is where that server base comes from. Kept here
//! (next to the domain types they format) rather than on `ClientConfig` to avoid a
//! `ui-config` → `wire` coupling.
//!
//! A hard product rule: the client makes NO requests outside the configured server
//! (enforced by the frontend CSP). Feed artwork is therefore served by
//! `GET /{episodes,podcasts}/{id}/art`, which resolves art lazily on first request.
//! The UI requests art OPTIMISTICALLY (no gating on the row's `art_url`, since the
//! server's fallback can produce art the row doesn't know about); the `Artwork`
//! component swaps in a placeholder when a request truly 404s. `None` here only
//! means "no session/server yet".

use halogen_wire::{EpisodeData, PodcastData};

/// The native loopback media base — `http://127.0.0.1:{port}/{nonce}` — set
/// once at boot by `ui-state`'s `WebviewMediaBridge` when it binds the
/// in-process media server. Ambient (like `ui-platform::namespace`) because
/// URL building happens all over the view layer with no channel to thread a
/// port through. Media URLs MUST be plain-http loopback on native: WebKitGTK's
/// GStreamer media path (and Android WebView's) cannot fetch `<audio>` from a
/// custom webview URI scheme — only http(s) works for media on every wry
/// platform.
#[cfg(not(target_arch = "wasm32"))]
static LOCAL_MEDIA_BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Publish the loopback media server's base URL (called once at bridge boot;
/// later calls are ignored).
#[cfg(not(target_arch = "wasm32"))]
pub fn set_local_media_base(base: String) {
    let _ = LOCAL_MEDIA_BASE.set(base);
}

/// The loopback media base, once the bridge has bound it. `None` in renderless
/// builds/tests and before bridge boot.
#[cfg(not(target_arch = "wasm32"))]
pub fn local_media_base() -> Option<&'static str> {
    LOCAL_MEDIA_BASE.get().map(String::as_str)
}

/// Base for the media asset URLs (artwork + the streaming audio endpoint).
///
/// - **Web (wasm)**: absolute on the configured server — the browser attaches
///   the `auth_media` cookie set at login, so `<img>`/`<audio>` requests
///   authenticate on their own.
/// - **Native**: the app runs in a webview that never saw the login (auth is a
///   bearer token inside reqwest), so absolute server URLs would 401. Media
///   rides the authenticated in-process loopback server instead
///   (`{local_media_base}/halogen-media/...`, served by `ui-state`'s
///   `WebviewMediaBridge`, which forwards to `{server}/api/v1/{path}` with the
///   bearer token). Loopback http, NOT a custom-scheme asset handler: webview
///   media stacks (WebKitGTK/GStreamer, Android WebView) can only stream
///   `<audio>` over http(s). Before the bridge binds (renderless tests), the
///   relative form keeps URL shapes stable.
///
/// Both arms gate on a configured server (`None` = no session/server yet), so
/// pre-login views skip media requests identically on every target.
pub(crate) fn media_base(server_url: Option<&str>) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        Some(format!("{}/api/v1", server_url?.trim_end_matches('/')))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = server_url?;
        match local_media_base() {
            Some(base) => Some(format!("{base}/halogen-media")),
            None => Some("/halogen-media".to_string()),
        }
    }
}

/// Server artwork URL for an episode row (`None` = no server configured yet).
/// Full resolution — for the big player + episode detail. List/mini-player call
/// sites want [`art_url_for_episode_small`] instead.
pub fn art_url_for_episode(server_url: Option<&str>, ep: &EpisodeData) -> Option<String> {
    let base = media_base(server_url)?;
    Some(format!("{base}/episodes/{}/art", ep.id))
}

/// Downscaled (~256px) thumbnail URL for an episode row. Used by the episode
/// list, mini player, and up-next, where the multi-MB original is wasted on a
/// ~70px tile. The server generates + caches the variant on first request.
pub fn art_url_for_episode_small(server_url: Option<&str>, ep: &EpisodeData) -> Option<String> {
    let base = media_base(server_url)?;
    Some(format!("{base}/episodes/{}/art/small", ep.id))
}

/// Server artwork URL for a podcast row (`None` = no server configured yet).
/// Full resolution — for the podcast detail view. The podcast list wants
/// [`art_url_for_podcast_small`] instead.
pub fn art_url_for_podcast(server_url: Option<&str>, p: &PodcastData) -> Option<String> {
    let base = media_base(server_url)?;
    Some(format!("{base}/podcasts/{}/art", p.id))
}

/// Downscaled (~256px) thumbnail URL for a podcast row. Used by the podcast list.
pub fn art_url_for_podcast_small(server_url: Option<&str>, p: &PodcastData) -> Option<String> {
    let base = media_base(server_url)?;
    Some(format!("{base}/podcasts/{}/art/small", p.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use halogen_wire::{DownloadStatus, PlaybackStatus};

    fn ep(id: i32) -> EpisodeData {
        let ts = Utc::now();
        EpisodeData {
            id,
            podcast_id: 1,
            title: String::new(),
            description: None,
            content_url: String::new(),
            guid: None,
            art_url: None,
            published_at: Some(ts),
            downloaded_at: None,
            content_file_path: None,
            download_size: None,
            art_file_path: None,
            download_status: DownloadStatus::NotDownloaded,
            download_started_at: None,
            download_attempts: 0,
            playback_status: PlaybackStatus::Unplayed,
            duration_secs: None,
            created_at: ts,
            updated_at: ts,
            podcast: None,
            playback: None,
            chapters: None,
        }
    }

    fn pod(id: i32) -> PodcastData {
        let ts = Utc::now();
        PodcastData {
            id,
            title: "P".into(),
            description: String::new(),
            feed_url: String::new(),
            art_url: None,
            author: None,
            polled_at: None,
            podcast_config_id: None,
            art_file_path: None,
            etag: None,
            last_modified: None,
            podcast_config: None,
            created_at: ts,
            updated_at: ts,
            episode_count: None,
            feed_url_redirects: None,
        }
    }

    #[test]
    fn none_without_server() {
        assert_eq!(art_url_for_episode(None, &ep(3)), None);
        assert_eq!(art_url_for_podcast(None, &pod(5)), None);
    }

    // These unit tests run natively (renderless `just test-ui`), so they assert
    // the native form: relative `halogen-media` proxy URLs. The wasm arm of
    // `media_base` (absolute `{server}/api/v1`, slash-trimmed) is the same
    // format the pre-split builders had; it's compile-checked by the wasm
    // build (`just check-all`) and exercised end-to-end by the browser tier.

    #[test]
    fn built_relative_to_media_proxy() {
        assert_eq!(
            art_url_for_episode(Some("https://srv.example///"), &ep(3)),
            Some("/halogen-media/episodes/3/art".to_string())
        );
        assert_eq!(
            art_url_for_podcast(Some("https://srv.example/"), &pod(5)),
            Some("/halogen-media/podcasts/5/art".to_string())
        );
    }

    #[test]
    fn small_variants_append_small_segment() {
        assert_eq!(art_url_for_episode_small(None, &ep(3)), None);
        assert_eq!(art_url_for_podcast_small(None, &pod(5)), None);
        assert_eq!(
            art_url_for_episode_small(Some("https://srv.example/"), &ep(3)),
            Some("/halogen-media/episodes/3/art/small".to_string())
        );
        assert_eq!(
            art_url_for_podcast_small(Some("https://srv.example"), &pod(5)),
            Some("/halogen-media/podcasts/5/art/small".to_string())
        );
    }
}
