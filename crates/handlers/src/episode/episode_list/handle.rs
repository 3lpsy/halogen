use crate::wants;
use halogen_wire::{DefaultListParams, EpisodeData, EpisodeInclude, Paginator, ValidationErrors};
use tracing::info;

use crate::episode::user_playback::user_playback_map;
use crate::episode::user_status::user_status_map;

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

    let query = halogen_queries::episodes::scoped_query(dbc, user_id, params).await?;

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
    crate::episode::attach_podcasts(dbc, &mut responses, load_podcast).await?;

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
        let mut chapters_map = crate::episode::chapters::chapters_map(dbc, &episode_ids).await;
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
