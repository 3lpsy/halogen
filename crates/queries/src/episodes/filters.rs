use halogen_orm::{
    episode::{Column, Entity as EpisodeEntity, Relation as EpisodeRelation},
    podcast as podcast_entity,
};
use halogen_wire::FilterParams;
use sea_orm::sea_query::{CaseStatement, SimpleExpr};
use sea_orm::{
    ColumnTrait, Condition, JoinType, Order as SeaOrder, QueryFilter, QueryOrder, QuerySelect,
    RelationTrait,
};

pub(super) fn apply_episode_filters(
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
