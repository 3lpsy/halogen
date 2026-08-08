//! `From` conversions bridging the wire DTOs (`halogen-wire`) and the SeaORM
//! entity models defined in this crate.
//!
//! They live here rather than in `halogen-wire` because they reference both
//! layers, and `halogen-orm` is the crate that depends on `halogen-wire` — the
//! reverse direction would be a dependency cycle.

use chrono::Utc;
use halogen_wire::{
    EpisodeChapterData, EpisodeData, EpisodePlaylistData, EpisodeStoreData, PlaybackData,
    PlaybackStatus, PlaylistData, PodcastAutoPlaylistData, PodcastConfigData,
    PodcastConfigStoreData, PodcastData, PodcastStoreData, UserData,
};
use sea_orm::ActiveValue;

impl From<PodcastStoreData> for crate::podcast::ActiveModel {
    fn from(podcast_data: PodcastStoreData) -> Self {
        let now = Utc::now();
        crate::podcast::ActiveModel {
            id: ActiveValue::NotSet,
            title: ActiveValue::Set(podcast_data.title),
            description: ActiveValue::Set(podcast_data.description.unwrap_or_default()),
            feed_url: ActiveValue::Set(podcast_data.feed_url),
            art_url: ActiveValue::Set(podcast_data.art_url),
            author: ActiveValue::Set(podcast_data.author),
            polled_at: ActiveValue::NotSet,
            podcast_config_id: ActiveValue::Set(podcast_data.podcast_config_id),
            // Owner is server-derived from the auth token; the handler Sets it
            // before insert (the NOT NULL column has no DB default).
            owner_id: ActiveValue::NotSet,
            art_file_path: ActiveValue::NotSet,
            etag: ActiveValue::NotSet,
            last_modified: ActiveValue::NotSet,
            feed_url_redirects: ActiveValue::NotSet,
            created_at: ActiveValue::Set(now),
            updated_at: ActiveValue::Set(now),
        }
    }
}

impl From<crate::podcast::Model> for PodcastData {
    fn from(model: crate::podcast::Model) -> Self {
        Self {
            id: model.id,
            title: model.title,
            description: model.description,
            feed_url: model.feed_url,
            art_url: model.art_url,
            author: model.author,
            polled_at: model.polled_at,
            podcast_config_id: model.podcast_config_id,
            art_file_path: model.art_file_path,
            etag: model.etag,
            last_modified: model.last_modified,
            podcast_config: None,
            created_at: model.created_at,
            updated_at: model.updated_at,
            episode_count: None,
            feed_url_redirects: model.feed_url_redirects,
        }
    }
}

impl From<EpisodeStoreData> for crate::episode::ActiveModel {
    fn from(episode_data: EpisodeStoreData) -> Self {
        let now = Utc::now();
        crate::episode::ActiveModel {
            id: ActiveValue::NotSet,
            podcast_id: ActiveValue::Set(episode_data.podcast_id),
            title: ActiveValue::Set(episode_data.title),
            description: ActiveValue::Set(episode_data.description),
            content_url: ActiveValue::Set(episode_data.content_url),
            guid: ActiveValue::Set(episode_data.guid),
            art_url: ActiveValue::Set(episode_data.art_url),
            published_at: ActiveValue::Set(episode_data.published_at),
            // Server-managed download bookkeeping — the API DTO no longer carries
            // these; only the download + art-cache pipelines set them (always
            // under `media_root`). A freshly stored episode has no download yet:
            // NotSet → NULL paths/timestamp + the DB-default status (NotDownloaded).
            downloaded_at: ActiveValue::NotSet,
            content_file_path: ActiveValue::NotSet,
            download_size: ActiveValue::NotSet,
            art_file_path: ActiveValue::NotSet,
            download_status: ActiveValue::NotSet,
            // Server-managed download bookkeeping; a freshly stored episode has
            // no attempt history (NotSet → NULL start + DB-default 0 attempts).
            download_started_at: ActiveValue::NotSet,
            download_attempts: ActiveValue::NotSet,
            duration_secs: ActiveValue::Set(episode_data.duration_secs),
            created_at: ActiveValue::Set(now),
            updated_at: ActiveValue::Set(now),
        }
    }
}

impl From<crate::episode::Model> for EpisodeData {
    fn from(model: crate::episode::Model) -> Self {
        Self {
            id: model.id,
            podcast_id: model.podcast_id,
            title: model.title,
            // The column is NOT NULL (always a String); the response DTO keeps
            // `Option` for wire/cache back-compat, so it's always `Some`.
            description: Some(model.description),
            content_url: model.content_url,
            guid: model.guid,
            art_url: model.art_url,
            published_at: model.published_at,
            downloaded_at: model.downloaded_at,
            content_file_path: model.content_file_path,
            // DB column is a signed SQLite INTEGER; the wire type is u64 (a
            // size can't be negative) — clamp defensively rather than wrap.
            download_size: model.download_size.map(|v| u64::try_from(v).unwrap_or(0)),
            art_file_path: model.art_file_path,
            download_status: model.download_status,
            download_started_at: model.download_started_at,
            download_attempts: model.download_attempts,
            // `playback_status` is per-user now (`user_episode_status`); the handler
            // overwrites this from the caller's row before serializing. Defaults to
            // Unplayed here for any path that doesn't (e.g. a freshly stored row).
            playback_status: PlaybackStatus::default(),
            duration_secs: model.duration_secs,
            created_at: model.created_at,
            updated_at: model.updated_at,
            podcast: None,
            playback: None,
            chapters: None,
        }
    }
}

impl From<PodcastConfigStoreData> for crate::podcast_config::ActiveModel {
    fn from(config_data: PodcastConfigStoreData) -> Self {
        crate::podcast_config::ActiveModel {
            id: ActiveValue::NotSet,
            poll_interval_seconds: ActiveValue::Set(config_data.poll_interval_seconds),
            max_episodes: ActiveValue::Set(config_data.max_episodes),
            max_concurrent_downloads: ActiveValue::Set(config_data.max_concurrent_downloads),
            auto_download_enabled: ActiveValue::Set(config_data.auto_download_enabled),
            created_at: ActiveValue::NotSet,
            updated_at: ActiveValue::NotSet,
        }
    }
}

impl From<crate::podcast_config::Model> for PodcastConfigData {
    fn from(model: crate::podcast_config::Model) -> Self {
        Self {
            id: model.id,
            poll_interval_seconds: model.poll_interval_seconds,
            max_episodes: model.max_episodes,
            max_concurrent_downloads: model.max_concurrent_downloads,
            auto_download_enabled: model.auto_download_enabled,
            created_at: model.created_at,
            updated_at: model.updated_at,
        }
    }
}

impl From<crate::episode_playlist::Model> for EpisodePlaylistData {
    fn from(model: crate::episode_playlist::Model) -> Self {
        Self {
            episode_id: model.episode_id,
            playlist_id: model.playlist_id,
            position: model.position,
        }
    }
}

impl From<crate::playback::Model> for PlaybackData {
    fn from(model: crate::playback::Model) -> Self {
        Self {
            id: model.id,
            user_id: model.user_id,
            episode_id: model.episode_id,
            // Signed in the DB, unsigned on the wire (a position can't be negative).
            cursor: u64::try_from(model.cursor).unwrap_or(0),
            completed: model.completed,
            created_at: model.created_at,
            updated_at: model.updated_at,
        }
    }
}

impl From<crate::playlist::Model> for PlaylistData {
    fn from(model: crate::playlist::Model) -> Self {
        Self {
            id: model.id,
            name: model.name,
            description: model.description,
            is_default: model.is_default,
            position: model.position,
            on_remove_delete_file_server: model.on_remove_delete_file_server,
            on_remove_delete_file_client: model.on_remove_delete_file_client,
            created_at: model.created_at,
            updated_at: model.updated_at,
            episode_ids: None,
            episode_playlist: None,
        }
    }
}

impl From<crate::episode_chapter::Model> for EpisodeChapterData {
    fn from(model: crate::episode_chapter::Model) -> Self {
        Self {
            id: model.id,
            episode_id: model.episode_id,
            title: model.title,
            starts_at_secs: model.starts_at_secs,
            created_at: model.created_at,
            updated_at: model.updated_at,
        }
    }
}

impl From<crate::podcast_auto_playlist::Model> for PodcastAutoPlaylistData {
    fn from(model: crate::podcast_auto_playlist::Model) -> Self {
        Self {
            podcast_id: model.podcast_id,
            playlist_id: model.playlist_id,
            add_to_start: model.add_to_start,
        }
    }
}

impl From<crate::user::Model> for UserData {
    fn from(model: crate::user::Model) -> Self {
        Self {
            id: model.id,
            username: model.username,
            is_admin: model.is_admin,
            created_at: model.created_at,
            updated_at: model.updated_at,
        }
    }
}
