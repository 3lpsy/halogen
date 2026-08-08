use std::collections::HashSet;

use halogen_wire::{PodcastAutoPlaylistData, PodcastAutoPlaylistSetData, ValidationErrors};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait};
use tracing::info;

use crate::handlers::{db_error, not_found};
use halogen_orm::playlist::{Column as PlaylistCol, Entity as PlaylistEntity};
use halogen_orm::podcast::{Column as PodCol, Entity as PodEntity};
use halogen_orm::podcast_auto_playlist::{self, Column as PapCol, Entity as PapEntity};

/// Replace a podcast's full set of auto-add playlists with `playlist_ids`,
/// transactionally. Idempotent — covers both first-time create and later edits
/// (the UI always sends the whole set). Unknown / since-deleted ids, and any
/// playlist not owned by the podcast's owner, are silently dropped so a stale
/// client can't hard-fail. 404 if the podcast is gone. Returns the resulting
/// (filtered, deduped) set.
pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    podcast_id: i32,
    data: PodcastAutoPlaylistSetData,
) -> Result<Vec<PodcastAutoPlaylistData>, ValidationErrors> {
    // The DTO was already validated by the `Body<PodcastAutoPlaylistSetData>` extractor.
    let txn = dbc
        .begin()
        .await
        .map_err(db_error("beginning transaction"))?;

    // The podcast must exist; its owner scopes which playlists may be linked.
    let owner_id = match PodEntity::find()
        .filter(PodCol::Id.eq(podcast_id))
        .one(&txn)
        .await
        .map_err(db_error("fetching podcast"))?
    {
        Some(p) => p.owner_id,
        None => return Err(not_found("Podcast not found")),
    };

    // Keep only ids that map to a live playlist OWNED BY THE PODCAST OWNER — linking
    // another user's playlist would let the poller inject episodes into it (IDOR).
    // Dedupe, preserving order.
    let live: HashSet<i32> = PlaylistEntity::find()
        .filter(PlaylistCol::UserId.eq(owner_id))
        .all(&txn)
        .await
        .map_err(db_error("listing playlists"))?
        .into_iter()
        .map(|p| p.id)
        .collect();
    let mut seen: HashSet<i32> = HashSet::new();
    let target: Vec<i32> = data
        .playlist_ids
        .into_iter()
        .filter(|id| live.contains(id) && seen.insert(*id))
        .collect();

    // Replace semantics: wipe the existing set, insert the new one.
    PapEntity::delete_many()
        .filter(PapCol::PodcastId.eq(podcast_id))
        .exec(&txn)
        .await
        .map_err(db_error("deleting auto-playlist links"))?;

    let now = chrono::Utc::now();
    for &playlist_id in &target {
        let model = podcast_auto_playlist::ActiveModel {
            podcast_id: Set(podcast_id),
            playlist_id: Set(playlist_id),
            add_to_start: Set(data.add_to_start),
            created_at: Set(now),
            updated_at: Set(now),
        };
        PapEntity::insert(model)
            .exec(&txn)
            .await
            .map_err(db_error("inserting auto-playlist link"))?;
    }

    txn.commit()
        .await
        .map_err(db_error("committing transaction"))?;
    info!(
        "Set {} auto-playlist(s) for podcast {}",
        target.len(),
        podcast_id
    );

    Ok(target
        .into_iter()
        .map(|playlist_id| PodcastAutoPlaylistData {
            podcast_id,
            playlist_id,
            add_to_start: data.add_to_start,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use halogen_fixture::test_support::TestRoot;
    use halogen_migrate::connect_and_migrate;
    use halogen_orm::{playlist, podcast, user};
    use sea_orm::ActiveModelTrait;

    async fn mk_user(dbc: &sea_orm::DatabaseConnection, id: i32, name: &str) {
        let now = chrono::Utc::now();
        user::ActiveModel {
            id: Set(id),
            username: Set(name.to_string()),
            password_hash: Set("x".to_string()),
            is_admin: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(dbc)
        .await
        .expect("insert user");
    }

    async fn mk_podcast(dbc: &sea_orm::DatabaseConnection, id: i32, owner_id: i32) {
        let now = chrono::Utc::now();
        podcast::ActiveModel {
            id: Set(id),
            title: Set("P".to_string()),
            description: Set(String::new()),
            feed_url: Set(format!("https://feed.test/{id}")),
            art_url: Set(None),
            author: Set(None),
            polled_at: Set(None),
            podcast_config_id: Set(None),
            owner_id: Set(owner_id),
            art_file_path: Set(None),
            etag: Set(None),
            last_modified: Set(None),
            feed_url_redirects: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(dbc)
        .await
        .expect("insert podcast");
    }

    async fn mk_playlist(dbc: &sea_orm::DatabaseConnection, id: i32, user_id: i32) {
        let now = chrono::Utc::now();
        playlist::ActiveModel {
            id: Set(id),
            name: Set(format!("pl{id}")),
            description: Set(None),
            user_id: Set(user_id),
            is_default: Set(false),
            position: Set(0),
            on_remove_delete_file_server: Set(false),
            on_remove_delete_file_client: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(dbc)
        .await
        .expect("insert playlist");
    }

    /// Regression (H2 IDOR): `set_for_podcast` may only link playlists owned by the
    /// PODCAST'S OWNER. A playlist owned by another user must be silently dropped, so
    /// the poller can never inject episodes into a victim's playlist.
    #[tokio::test]
    async fn set_for_podcast_drops_foreign_owned_playlists() {
        let mut root = TestRoot::new("auto_playlist_owner_scope");
        let db_path = root.path().join("halogen.db");
        let dbc = connect_and_migrate(&db_path, true).await.expect("test db");

        mk_user(&dbc, 1, "owner").await;
        mk_user(&dbc, 2, "victim").await;
        mk_podcast(&dbc, 10, 1).await; // podcast owned by user 1
        mk_playlist(&dbc, 100, 1).await; // owned by the podcast owner
        mk_playlist(&dbc, 200, 2).await; // owned by a DIFFERENT user (victim)

        let data = PodcastAutoPlaylistSetData {
            playlist_ids: vec![100, 200],
            add_to_start: None,
        };
        let kept = handle(&dbc, 10, data).await.expect("set auto-playlists");

        // Only the owner's playlist survives; the victim's (200) is dropped.
        let ids: Vec<i32> = kept.iter().map(|r| r.playlist_id).collect();
        assert_eq!(ids, vec![100], "foreign-owned playlist must be dropped");

        // And no auto-playlist row references the foreign playlist.
        let rows = PapEntity::find().all(&dbc).await.unwrap();
        assert!(
            rows.iter().all(|r| r.playlist_id != 200),
            "no link to a playlist owned by another user"
        );

        let _ = dbc.close().await;
        root.mark_success();
    }
}
