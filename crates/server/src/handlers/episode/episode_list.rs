use halogen_wire::{
    DefaultListParams, EpisodeData, EpisodeInclude, FilterParams, Paginator, PlaybackStatus,
    ValidationErrors,
};
use sea_orm::sea_query::{CaseStatement, SimpleExpr};
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, JoinType, Order as SeaOrder, QueryFilter, QueryOrder,
    QuerySelect, RelationTrait,
};
use tracing::info;

use crate::handlers::{db_error, wants};
use halogen_orm::episode::{Column, Entity as EpisodeEntity, Relation as EpisodeRelation};
use halogen_orm::podcast as podcast_entity;
use halogen_orm::user_episode_status as ues_entity;
use halogen_orm::user_podcast;

use super::user_playback::user_playback_map;
use super::user_status::user_status_map;

pub async fn handle(
    dbc: &sea_orm::DatabaseConnection,
    user_id: i32,
    params: &DefaultListParams<EpisodeInclude>,
) -> Result<(Vec<EpisodeData>, Paginator), ValidationErrors> {
    let pagination = params.pagination.clone().unwrap_or_default();
    let order = params.order.clone().unwrap_or_default();
    let load_podcast = wants(params.includes.as_ref(), EpisodeInclude::Podcast);
    let load_playback = wants(params.includes.as_ref(), EpisodeInclude::Playback);
    let load_chapters = wants(params.includes.as_ref(), EpisodeInclude::Chapters);

    // Scope to the caller's subscriptions: a user only sees episodes from
    // podcasts they're subscribed to (via `user_podcast`). Mirrors `podcast_list`,
    // which scopes the podcast list the same way. Without this base filter,
    // `GET /episodes` (and `GET /podcasts/{id}/episodes`, which delegates here)
    // would leak every owner's episodes across the whole server.
    let subscribed: Vec<i32> = user_podcast::Entity::find()
        .filter(user_podcast::Column::UserId.eq(user_id))
        .select_only()
        .column(user_podcast::Column::PodcastId)
        .into_tuple::<i32>()
        .all(dbc)
        .await
        .map_err(db_error("loading subscriptions"))?;

    let mut query = EpisodeEntity::find().filter(Column::PodcastId.is_in(subscribed));

    // Filters, including multi-column search and its relevance ranking (which
    // orders *before* the requested sort applied just below).
    if let Some(ref filter) = params.filter {
        query = apply_episode_filters(query, filter);

        // Per-user playback-status filter. The status moved off `episode` into the
        // per-user `user_episode_status` table, so it can't be a plain column
        // predicate; we resolve it against the caller's rows and constrain the
        // episode id set accordingly. Episodes with no row count as UNPLAYED.
        if let Some(ref status) = filter.playback_status {
            // Map the raw filter string to a known status. An unknown/garbage
            // value matches nothing — preserving the pre-refactor behaviour where
            // a column-equality on a bad value simply never matched.
            let wanted = match status.as_str() {
                "UNPLAYED" => Some(PlaybackStatus::Unplayed),
                "PLAYED" => Some(PlaybackStatus::Played),
                "FINISHED" => Some(PlaybackStatus::Finished),
                _ => None,
            };
            match wanted {
                None => {
                    query = query.filter(Column::Id.is_in(Vec::<i32>::new()));
                }
                Some(PlaybackStatus::Unplayed) => {
                    // Unplayed = no row OR a row still marked Unplayed → exclude
                    // every episode the caller has marked PLAYED/FINISHED.
                    let non_unplayed: Vec<i32> = ues_entity::Entity::find()
                        .filter(ues_entity::Column::UserId.eq(user_id))
                        .filter(ues_entity::Column::PlaybackStatus.ne(PlaybackStatus::Unplayed))
                        .select_only()
                        .column(ues_entity::Column::EpisodeId)
                        .into_tuple::<i32>()
                        .all(dbc)
                        .await
                        .map_err(db_error("loading played episodes"))?;
                    if !non_unplayed.is_empty() {
                        query = query.filter(Column::Id.is_not_in(non_unplayed));
                    }
                }
                Some(wanted) => {
                    // PLAYED / FINISHED: restrict to episodes whose per-user row
                    // has that exact status. No matches → an empty id set.
                    let matching: Vec<i32> = ues_entity::Entity::find()
                        .filter(ues_entity::Column::UserId.eq(user_id))
                        .filter(ues_entity::Column::PlaybackStatus.eq(wanted))
                        .select_only()
                        .column(ues_entity::Column::EpisodeId)
                        .into_tuple::<i32>()
                        .all(dbc)
                        .await
                        .map_err(db_error("loading episodes by status"))?;
                    query = query.filter(Column::Id.is_in(matching));
                }
            }
        }
    }

    // Page the filtered query (0-based). The requested sort is applied inside
    // `paginate` as a secondary tiebreak under any search-relevance rank already on
    // the query, and the paginator yields the total item/page counts for the envelope.
    let (episodes, paginator) =
        halogen_orm::common::paginate(dbc, query, &pagination, &order).await?;

    // Per-user listen state for exactly this page's episodes. The status lives in
    // `user_episode_status` now; absence == UNPLAYED. Collected before `episodes`
    // is consumed below so we can overwrite each `EpisodeData.playback_status`.
    let episode_ids: Vec<i32> = episodes.iter().map(|e| e.id).collect();
    let status_map = user_status_map(dbc, user_id, &episode_ids).await;

    let mut responses: Vec<EpisodeData> = episodes.into_iter().map(EpisodeData::from).collect();
    super::attach_podcasts(dbc, &mut responses, load_podcast).await?;

    // Overwrite the per-user listen state the `From<Model>` defaulted to Unplayed.
    for r in responses.iter_mut() {
        r.playback_status = status_map.get(&r.id).cloned().unwrap_or_default();
    }

    // Embed the caller's resume cursor when requested (scoped to this user via the
    // `playback` pivot). Left `None` otherwise — clients then fall back to their
    // optimistic overlay. Same page-id set as the status map above.
    if load_playback {
        let playback_map = user_playback_map(dbc, user_id, &episode_ids).await;
        for r in responses.iter_mut() {
            r.playback = playback_map.get(&r.id).cloned();
        }
    }

    // Embed read-only chapter markers when requested (shared across users).
    // Same page-id set; episodes without chapters get an empty vec.
    if load_chapters {
        let mut chapters_map = super::chapters::chapters_map(dbc, &episode_ids).await;
        for r in responses.iter_mut() {
            r.chapters = Some(chapters_map.remove(&r.id).unwrap_or_default());
        }
    }

    info!(
        "Fetched {} episodes (page {}, size {}, total {})",
        responses.len(),
        pagination.page,
        pagination.size,
        paginator.total
    );
    Ok((responses, paginator))
}

fn apply_episode_filters(
    query: sea_orm::Select<EpisodeEntity>,
    filter: &FilterParams,
) -> sea_orm::Select<EpisodeEntity> {
    let mut q = query;

    // Multi-column search: match the term in the episode title, its podcast's
    // name, or the description. All `like` values bind as parameters (no SQL
    // interpolation). A LEFT JOIN to podcast makes the name searchable + rankable;
    // episode→podcast is 1:1 so it doesn't fan rows out.
    if let Some(ref search) = filter.search {
        let like = format!("%{}%", search);
        q = q
            .join(JoinType::LeftJoin, EpisodeRelation::Podcast.def())
            .filter(
                Condition::any()
                    .add(Column::Title.like(like.as_str()))
                    .add(Column::Description.like(like.as_str()))
                    .add(podcast_entity::Column::Title.like(like.as_str())),
            );
        // Relevance rank: title match (0) beats podcast-name match (1) beats a
        // description-only match (2). Added before the caller's requested sort, so
        // it's the primary ordering; the response is returned in ranked order.
        let rank: SimpleExpr = CaseStatement::new()
            .case(Column::Title.like(like.as_str()), 0)
            .case(podcast_entity::Column::Title.like(like.as_str()), 1)
            .finally(2)
            .into();
        q = q.order_by(rank, SeaOrder::Asc);
    }

    if let Some(podcast_id) = filter.podcast_id {
        q = q.filter(Column::PodcastId.eq(podcast_id));
    }
    if let Some(ref download_status) = filter.download_status {
        q = q.filter(Column::DownloadStatus.eq(download_status.as_str()));
    }
    if let Some(published_after) = filter.published_after {
        q = q.filter(Column::PublishedAt.gte(published_after));
    }
    // `filter.playback_status` is per-user now (`user_episode_status`) and can't be
    // a plain `episode` column predicate; it's resolved in `handle` against the
    // caller's rows and applied as an episode-id restriction.
    if let Some(ref ids) = filter.ids
        && !ids.is_empty()
    {
        q = q.filter(Column::Id.is_in(ids.iter().copied()));
    }
    // `unplayed_only` is deprecated/superseded by `playback_status`.
    let _ = filter.unplayed_only;
    q
}

#[cfg(test)]
mod tests {
    use super::handle;
    use halogen_fixture::test_support::TestRoot;
    use halogen_migrate::connect_and_migrate;
    use halogen_orm::{episode, podcast, user, user_podcast};
    use halogen_wire::{DefaultListParams, DownloadStatus, EpisodeInclude};
    use sea_orm::{
        ActiveModelTrait,
        ActiveValue::{NotSet, Set},
        DatabaseConnection,
    };

    async fn seed_user(dbc: &DatabaseConnection, id: i32, username: &str) {
        user::ActiveModel {
            id: Set(id),
            username: Set(username.to_string()),
            password_hash: Set("x".to_string()),
            is_admin: Set(false),
            created_at: Set(chrono::Utc::now()),
            updated_at: Set(chrono::Utc::now()),
        }
        .insert(dbc)
        .await
        .expect("seed user");
    }

    /// Insert a podcast owned by `owner_id` and return its id.
    async fn seed_podcast(dbc: &DatabaseConnection, owner_id: i32, title: &str) -> i32 {
        podcast::ActiveModel {
            id: NotSet,
            title: Set(title.to_string()),
            description: Set(String::new()),
            feed_url: Set(format!("https://feeds.example.com/{title}")),
            art_url: Set(None),
            author: Set(None),
            polled_at: Set(None),
            podcast_config_id: Set(None),
            owner_id: Set(owner_id),
            art_file_path: Set(None),
            etag: Set(None),
            last_modified: Set(None),
            feed_url_redirects: Set(None),
            created_at: Set(chrono::Utc::now()),
            updated_at: Set(chrono::Utc::now()),
        }
        .insert(dbc)
        .await
        .expect("seed podcast")
        .id
    }

    async fn seed_episode(dbc: &DatabaseConnection, podcast_id: i32, title: &str) -> i32 {
        episode::ActiveModel {
            id: NotSet,
            podcast_id: Set(podcast_id),
            title: Set(title.to_string()),
            description: Set(String::new()),
            content_url: Set(format!("https://audio.example.com/{title}.mp3")),
            guid: Set(None),
            art_url: Set(None),
            published_at: Set(None),
            downloaded_at: Set(None),
            content_file_path: Set(None),
            download_size: Set(None),
            art_file_path: Set(None),
            download_status: Set(DownloadStatus::NotDownloaded),
            download_started_at: Set(None),
            download_attempts: Set(0),
            duration_secs: Set(None),
            created_at: Set(chrono::Utc::now()),
            updated_at: Set(chrono::Utc::now()),
        }
        .insert(dbc)
        .await
        .expect("seed episode")
        .id
    }

    async fn subscribe(dbc: &DatabaseConnection, user_id: i32, podcast_id: i32) {
        user_podcast::ActiveModel {
            user_id: Set(user_id),
            podcast_id: Set(podcast_id),
            created_at: Set(chrono::Utc::now()),
            updated_at: Set(chrono::Utc::now()),
        }
        .insert(dbc)
        .await
        .expect("subscribe");
    }

    async fn seed_playback(dbc: &DatabaseConnection, user_id: i32, episode_id: i32, cursor: i64) {
        use halogen_orm::playback;
        playback::ActiveModel {
            id: NotSet,
            user_id: Set(user_id),
            episode_id: Set(episode_id),
            cursor: Set(cursor),
            completed: Set(false),
            created_at: Set(chrono::Utc::now()),
            updated_at: Set(chrono::Utc::now()),
        }
        .insert(dbc)
        .await
        .expect("seed playback");
    }

    /// `EpisodeInclude::Playback` embeds the **caller's** resume cursor and only
    /// the caller's: alice sees her cursor, bob (same episode, no row) sees `None`,
    /// and omitting the include leaves `playback` `None` for everyone.
    #[tokio::test]
    async fn playback_include_embeds_only_callers_cursor() {
        let mut root = TestRoot::new("episode_list_playback_include");
        let db_path = root.path().join("halogen.db");
        let dbc = connect_and_migrate(&db_path, true).await.unwrap();

        seed_user(&dbc, 1, "alice").await;
        seed_user(&dbc, 2, "bob").await;
        let pod = seed_podcast(&dbc, 1, "pod").await;
        let ep = seed_episode(&dbc, pod, "ep").await;
        subscribe(&dbc, 1, pod).await;
        subscribe(&dbc, 2, pod).await;
        // Only alice (user 1) has a saved position.
        seed_playback(&dbc, 1, ep, 123).await;

        let with_pb = DefaultListParams::<EpisodeInclude> {
            includes: Some(vec![EpisodeInclude::Playback]),
            ..Default::default()
        };

        // Alice: cursor embedded.
        let (alice_eps, _) = handle(&dbc, 1, &with_pb).await.unwrap();
        assert_eq!(
            alice_eps[0].playback.as_ref().map(|p| p.cursor),
            Some(123),
            "caller's cursor is embedded with the Playback include"
        );

        // Bob: same episode, no row of his own → no leak of alice's cursor.
        let (bob_eps, _) = handle(&dbc, 2, &with_pb).await.unwrap();
        assert!(
            bob_eps[0].playback.is_none(),
            "another user's cursor must never be embedded"
        );

        // No include → never populated, even for alice.
        let no_pb = DefaultListParams::<EpisodeInclude>::default();
        let (alice_no_inc, _) = handle(&dbc, 1, &no_pb).await.unwrap();
        assert!(
            alice_no_inc[0].playback.is_none(),
            "playback stays None unless the include is requested"
        );

        drop(dbc);
        root.mark_success();
    }

    /// `GET /episodes` must only return episodes from podcasts the caller is
    /// subscribed to — never another owner's, and never a non-subscribed
    /// podcast's, even when both live in the same database.
    #[tokio::test]
    async fn list_episodes_scoped_to_subscriptions() {
        let mut root = TestRoot::new("episode_list_scope");
        let db_path = root.path().join("halogen.db");
        let dbc = connect_and_migrate(&db_path, true).await.unwrap();

        // Two users; `alice` (id 1) is our caller, `bob` (id 2) owns a rival podcast.
        seed_user(&dbc, 1, "alice").await;
        seed_user(&dbc, 2, "bob").await;

        // Alice owns + is subscribed to `subbed`; she also owns `unsubbed` but is
        // NOT subscribed to it. Bob owns `bobcast`, which Alice can't see at all.
        let subbed = seed_podcast(&dbc, 1, "subbed").await;
        let unsubbed = seed_podcast(&dbc, 1, "unsubbed").await;
        let bobcast = seed_podcast(&dbc, 2, "bobcast").await;

        let visible = seed_episode(&dbc, subbed, "visible-ep").await;
        let _hidden_unsubbed = seed_episode(&dbc, unsubbed, "hidden-unsubbed-ep").await;
        let _hidden_bob = seed_episode(&dbc, bobcast, "hidden-bob-ep").await;

        subscribe(&dbc, 1, subbed).await;

        let params = DefaultListParams::<EpisodeInclude>::default();
        let (episodes, paginator) = handle(&dbc, 1, &params).await.unwrap();

        let ids: Vec<i32> = episodes.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![visible], "only the subscribed podcast's episode");
        assert_eq!(paginator.total, 1, "paginator total must reflect the scope");

        // A user with no subscriptions sees nothing — `is_in([])` returns no rows.
        let (none, none_paginator) = handle(&dbc, 2, &params).await.unwrap();
        assert!(none.is_empty(), "bob is subscribed to nothing");
        assert_eq!(none_paginator.total, 0);

        drop(dbc);
        root.mark_success();
    }
}
