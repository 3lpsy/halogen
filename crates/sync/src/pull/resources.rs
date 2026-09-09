use anyhow::Result;
use halogen_apiclient::{ApiClient, ApiError};
use halogen_sync_store::{LocalStore, StoreChanges};
use halogen_wire::{EpisodeInclude, PlaylistInclude, SyncChangeData, SyncResource};

fn is_missing(error: &ApiError) -> bool {
    matches!(halogen_sync_policy::status_of_error(error), Some(403 | 404))
}

pub(super) async fn fetch_changes(
    store: &dyn LocalStore,
    api: &ApiClient,
    events: &[SyncChangeData],
) -> Result<StoreChanges> {
    let mut changes = StoreChanges::default();
    // Only the last event for a row matters; all fetches resolve its current authorized state.
    for (index, event) in events.iter().enumerate() {
        if events[index + 1..]
            .iter()
            .any(|later| later.resource == event.resource && later.resource_id == event.resource_id)
        {
            continue;
        }
        let id = event.resource_id;
        anyhow::ensure!(id > 0, "invalid sync resource id");
        match event.resource {
            SyncResource::Podcasts => {
                if !event.deleted {
                    match api.get_podcast(id).await {
                        Ok(row) => {
                            // A new subscription can expose links created before this cursor.
                            match api.get_podcast_auto_playlists(id).await {
                                Ok(rows) => {
                                    changes.auto_playlists.insert(id, rows);
                                }
                                Err(error) if is_missing(&error) => {
                                    changes.deleted_auto_playlists.push(id);
                                }
                                Err(error) => return Err(error.into()),
                            }
                            changes.podcasts.push(row);
                            continue;
                        }
                        Err(error) if is_missing(&error) => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                changes.deleted_podcasts.push(id);
                changes.deleted_auto_playlists.push(id);
                let episodes = store.list_episode_ids_for_podcast(id).await?;
                changes.deleted_playbacks.extend(&episodes);
                changes.deleted_episodes.extend(episodes);
            }
            SyncResource::Episodes | SyncResource::Playbacks => {
                if !event.deleted || event.resource == SyncResource::Playbacks {
                    match api
                        .get_episode(id, &[EpisodeInclude::Playback, EpisodeInclude::Chapters])
                        .await
                    {
                        Ok(row) => {
                            if let Some(playback) = row.playback.clone() {
                                changes.playbacks.push(playback);
                            } else {
                                changes.deleted_playbacks.push(id);
                            }
                            changes.episodes.push(row);
                            continue;
                        }
                        Err(error) if is_missing(&error) => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                changes.deleted_episodes.push(id);
                changes.deleted_playbacks.push(id);
            }
            SyncResource::Playlists => {
                if !event.deleted {
                    match api
                        .get_playlist_including(id, &[PlaylistInclude::EpisodeIds])
                        .await
                    {
                        Ok(row) => {
                            changes.playlists.push(row);
                            continue;
                        }
                        Err(error) if is_missing(&error) => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                changes.deleted_playlists.push(id);
            }
            SyncResource::PodcastAutoPlaylists => {
                if !event.deleted {
                    match api.get_podcast_auto_playlists(id).await {
                        Ok(rows) => {
                            changes.auto_playlists.insert(id, rows);
                            continue;
                        }
                        Err(error) if is_missing(&error) => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                changes.deleted_auto_playlists.push(id);
            }
        }
    }
    if !changes.deleted_episodes.is_empty() {
        let refreshed = changes
            .playlists
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>();
        for mut row in store.list_playlists().await? {
            if refreshed.contains(&row.id) || changes.deleted_playlists.contains(&row.id) {
                continue;
            }
            if let Some(ids) = row.episode_ids.as_mut() {
                let before = ids.len();
                ids.retain(|id| !changes.deleted_episodes.contains(id));
                if before != ids.len() {
                    changes.playlists.push(row);
                }
            }
        }
    }
    Ok(changes)
}
