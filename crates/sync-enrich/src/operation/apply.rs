use super::*;

/// Clamp to the DTO's char-count validation bound (validator counts chars).
fn clamp_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

impl OutboxOp {
    pub fn is_absence_request(&self) -> bool {
        matches!(
            self,
            Self::Unsubscribe { .. }
                | Self::RemoveFromPlaylist { .. }
                | Self::RemovePodcastConfig { .. }
                | Self::RemoveServerDownload { .. }
        )
    }

    /// Send this queued op to the server. This is the single place that maps an
    /// outbox op to an API call — the worker drains ops and acks on `Ok`.
    /// `Subscribe` returns the created/reused podcast so the worker can merge it
    /// into the pool immediately (the post-drain pull doesn't fetch podcasts).
    pub async fn apply(&self, api: &ApiClient) -> Result<Option<PodcastData>, ApiError> {
        match self {
            OutboxOp::Subscribe {
                feed_url,
                title,
                description,
                author,
            } => {
                // The DTO requires a 1-256 char title; fall back to the feed URL —
                // the RSS ingest heals it to the channel title on first sync.
                let title = title
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .unwrap_or(feed_url);
                api.create_podcast(PodcastStoreData {
                    title: clamp_chars(title, 256),
                    description: description.as_deref().map(|d| clamp_chars(d, 65536)),
                    feed_url: feed_url.clone(),
                    art_url: None,
                    author: author.as_deref().map(|a| clamp_chars(a, 256)),
                    podcast_config_id: None,
                })
                .await
                .map(Some)
            }
            OutboxOp::Unsubscribe { podcast_id } => {
                api.delete_podcast(*podcast_id).await.map(|_| None)
            }
            // Marking played/unplayed resets the position to 0 — deliberately, and
            // in lock-step with the optimistic local change (`set_playback_locally`
            // sets `pb.cursor = 0` for the same command), so a finished/reset
            // episode shows no resume position on either side.
            OutboxOp::MarkPlayed { episode_id, played } => api
                .upsert_playback(PlaybackStoreData {
                    episode_id: *episode_id,
                    cursor: 0,
                    completed: *played,
                })
                .await
                .map(|_| None),
            OutboxOp::SetCursor { episode_id, cursor } => api
                .upsert_playback(PlaybackStoreData {
                    episode_id: *episode_id,
                    // Client-side cursors stay i64 (seek math may transiently go
                    // negative); the wire type is u64 — clamp at the boundary.
                    cursor: u64::try_from(*cursor).unwrap_or(0),
                    completed: false,
                })
                .await
                .map(|_| None),
            OutboxOp::AddToPlaylist {
                playlist_id,
                episode_ids,
                position,
            } => {
                if episode_ids.is_empty() {
                    return Ok(None);
                }
                match position {
                    // Front-of-queue: a positioned insert is only ever a single id,
                    // and the bulk endpoint appends only — so use the positioned
                    // single-add endpoint.
                    Some(pos) => {
                        for episode_id in episode_ids {
                            api.add_episode(*playlist_id, *episode_id, Some(*pos))
                                .await?;
                        }
                        Ok(None)
                    }
                    None => api
                        .add_episodes_bulk(*playlist_id, episode_ids.clone())
                        .await
                        .map(|_| None),
                }
            }
            OutboxOp::RemoveFromPlaylist {
                playlist_id,
                episode_ids,
            } => {
                if episode_ids.is_empty() {
                    return Ok(None);
                }
                api.remove_episodes_bulk(*playlist_id, episode_ids.clone())
                    .await
                    .map(|_| None)
            }
            OutboxOp::MoveInPlaylist {
                playlist_id,
                episode_id,
                to,
            } => api
                .move_episode(*playlist_id, *episode_id, *to)
                .await
                .map(|_| None),
            OutboxOp::MovePlaylist { playlist_id, to } => {
                api.move_playlist(*playlist_id, *to).await.map(|_| None)
            }
            OutboxOp::ReorderPlaylist {
                playlist_id,
                field,
                direction,
            } => api
                .reorder_playlist(*playlist_id, *field, direction.clone())
                .await
                .map(|_| None),
            OutboxOp::UpdatePlaylist { playlist_id, data } => api
                .update_playlist(*playlist_id, data.clone())
                .await
                .map(|_| None),
            OutboxOp::UpdatePodcastConfig { config_id, data } => api
                .update_podcast_config(*config_id, data.clone())
                .await
                .map(|_| None),
            OutboxOp::RemovePodcastConfig { podcast_id } => api
                .remove_podcast_config_for(*podcast_id)
                .await
                .map(|_| None),
            OutboxOp::SetPodcastAutoPlaylists {
                podcast_id,
                playlist_ids,
                add_to_start,
            } => api
                .set_podcast_auto_playlists(*podcast_id, playlist_ids.clone(), *add_to_start)
                .await
                .map(|_| None),
            OutboxOp::TriggerDownload { episode_ids } => {
                if episode_ids.is_empty() {
                    return Ok(None);
                }
                api.trigger_download_bulk(episode_ids.clone())
                    .await
                    .map(|_| None)
            }
            OutboxOp::RemoveServerDownload { episode_ids } => {
                if episode_ids.is_empty() {
                    return Ok(None);
                }
                api.remove_server_download_bulk(episode_ids.clone())
                    .await
                    .map(|_| None)
            }
        }
    }

    /// True for ops that change server-side structure the client can't fully
    /// reproduce locally (new podcast + its episodes), so a pull should follow.
    pub fn needs_pull_after(&self) -> bool {
        matches!(
            self,
            OutboxOp::Subscribe { .. } | OutboxOp::Unsubscribe { .. }
        )
    }
}
