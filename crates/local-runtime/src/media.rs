use halogen_routes::routers::{guards, media_path};
use sea_orm::EntityTrait;
use std::path::Path;

impl crate::LocalSession {
    /// Resolve only the supported media routes through subscription-checked accessors.
    pub async fn media_path(&self, path: &str) -> Result<Option<String>, String> {
        let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
        match parts.as_slice() {
            ["episodes", id, "audio"] => {
                self.audio_path(id.parse().map_err(|_| "invalid episode ID")?)
                    .await
            }
            [kind @ ("episodes" | "podcasts"), id, "art"] => {
                self.art_path(
                    id.parse().map_err(|_| "invalid artwork ID")?,
                    *kind == "episodes",
                    false,
                )
                .await
            }
            [kind @ ("episodes" | "podcasts"), id, "art", "small"] => {
                self.art_path(
                    id.parse().map_err(|_| "invalid artwork ID")?,
                    *kind == "episodes",
                    true,
                )
                .await
            }
            _ => Err("invalid media route".into()),
        }
    }

    /// Return a confined local file for the platform player, avoiding audio copies over FFI.
    pub async fn audio_path(&self, id: i32) -> Result<Option<String>, String> {
        let _guard = self.library.activity.read().await;
        self.library.ensure_active()?;
        let actor = self.current_actor().await?;
        if id <= 0 {
            return Err("invalid episode ID".into());
        }
        guards::require_episode_subscribed(&self.library.db, actor, id)
            .await
            .map_err(|e| format!("media access denied: {}", e.status()))?;
        let row = halogen_orm::episode::Entity::find_by_id(id)
            .one(&self.library.db)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("episode not found")?;
        if row.download_status != halogen_wire::DownloadStatus::Downloaded {
            return Ok(None);
        }
        Ok(row
            .content_file_path
            .and_then(|path| self.confined_path(&path)))
    }

    pub async fn art_path(
        &self,
        id: i32,
        is_episode: bool,
        small: bool,
    ) -> Result<Option<String>, String> {
        let _guard = self.library.activity.read().await;
        self.library.ensure_active()?;
        let actor = self.current_actor().await?;
        if id <= 0 {
            return Err("invalid artwork ID".into());
        }
        let db = &self.library.db;
        let root = &self.library.config.media_root;
        let path = if is_episode {
            guards::require_episode_subscribed(db, actor, id)
                .await
                .map_err(|e| e.status().to_string())?;
            if small {
                halogen_art::ensure_episode_art_small(db, id, root, true).await
            } else {
                halogen_art::ensure_episode_art(db, id, root, true).await
            }
        } else {
            guards::require_subscribed(db, actor, id)
                .await
                .map_err(|e| e.status().to_string())?;
            if small {
                halogen_art::ensure_podcast_art_small(db, id, root, true).await
            } else {
                halogen_art::ensure_podcast_art(db, id, root, true).await
            }
        }
        .map_err(|e| e.to_string())?;
        Ok(path.and_then(|path| self.confined_path(&path.to_string_lossy())))
    }

    fn confined_path(&self, path: &str) -> Option<String> {
        media_path::confined_or_rebased(&self.library.config.media_root, Path::new(path))
            .map(|path| path.to_string_lossy().into_owned())
    }
}
