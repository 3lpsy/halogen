//! Build media URLs from the configured server without coupling config to wire types. Request server artwork even
//! without a row art_url because lazy fallbacks may succeed; Artwork handles real failures. No configured server
//! returns None, and feed-origin requests remain forbidden.

use halogen_wire::{EpisodeData, PodcastData};

/// Publish the native media bridge's boot-time loopback port and nonce for shared URL builders. Webview platform media
/// stacks require HTTP(S), so custom asset schemes cannot serve audio reliably.
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

/// Web media uses configured-server URLs with auth_media cookies; native uses the bearer-authenticated loopback bridge
/// because webviews lack those cookies. Before native binding, retain relative URL shapes for tests. Both require a
/// configured server and return None before setup.
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

    // These unit tests run natively (renderless `test-ui`), so they assert the native form: relative `halogen-media`
    // proxy URLs. The wasm arm of `media_base` (absolute `{server}/api/v1`, slash-trimmed) is the same format the
    // pre-split builders had; it's compile-checked by the wasm build (`check-all`) and exercised end-to-end by the
    // browser tier.

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
