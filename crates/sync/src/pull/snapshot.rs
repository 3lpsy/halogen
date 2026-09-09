use anyhow::{Result, bail, ensure};
use halogen_apiclient::ApiClient;
use halogen_sync_store::{EpisodeQuery, LocalStore, StoreChanges};
use halogen_wire::{
    DefaultListParams, EpisodeInclude, Order, OrderDirection, Pagination, PlaybackListParams,
    PlaylistInclude, PodcastInclude, SyncChangesParams,
};

const PAGE_SIZE: i32 = 500;

fn pagination(page: i32) -> Pagination {
    Pagination {
        page,
        size: PAGE_SIZE,
    }
}
fn order() -> Order {
    Order {
        direction: OrderDirection::Asc,
        order_by: "id".into(),
    }
}

/// Offset pagination is accepted only while the actor's change feed stays unchanged.
pub(super) async fn fetch_consistent_snapshot(
    api: &ApiClient,
    mut cursor: String,
) -> Result<StoreChanges> {
    for _ in 0..3 {
        let mut changes = fetch_snapshot(api).await?;
        let check = api
            .get_sync_changes(SyncChangesParams {
                cursor: Some(cursor),
                limit: Some(1),
            })
            .await?;
        if !check.reset && check.changes.is_empty() {
            changes.sync_cursor = Some(check.next_cursor);
            return Ok(changes);
        }
        cursor = api
            .get_sync_changes(SyncChangesParams {
                cursor: None,
                limit: Some(1),
            })
            .await?
            .next_cursor;
    }
    bail!("library changed during snapshot; sync will retry")
}

async fn fetch_snapshot(api: &ApiClient) -> Result<StoreChanges> {
    let mut changes = StoreChanges {
        reset_cache: true,
        ..Default::default()
    };
    macro_rules! pages {
        ($method:ident, $target:ident, $includes:expr) => {
            for page in 0..=65_536 {
                let response = api
                    .$method(DefaultListParams {
                        pagination: Some(pagination(page)),
                        order: Some(order()),
                        includes: Some($includes),
                        ..Default::default()
                    })
                    .await?;
                let count = response.data.len();
                let done = response
                    .paginator
                    .as_ref()
                    .map(|p| page + 1 >= p.pages)
                    .unwrap_or(count < PAGE_SIZE as usize);
                changes.$target.extend(response.data);
                if done {
                    break;
                }
                ensure!(page < 65_536, "snapshot exceeds pagination limit");
            }
        };
    }
    pages!(list_podcasts, podcasts, vec![PodcastInclude::PodcastConfig]);
    pages!(
        list_episodes,
        episodes,
        vec![EpisodeInclude::Playback, EpisodeInclude::Chapters]
    );
    pages!(list_playlists, playlists, vec![PlaylistInclude::EpisodeIds]);
    for page in 0..=65_536 {
        let response = api
            .list_playbacks(PlaybackListParams {
                pagination: Some(pagination(page)),
                order: Some(order()),
                ..Default::default()
            })
            .await?;
        let count = response.data.len();
        let done = response
            .paginator
            .as_ref()
            .map(|p| page + 1 >= p.pages)
            .unwrap_or(count < PAGE_SIZE as usize);
        changes.playbacks.extend(response.data);
        if done {
            break;
        }
        ensure!(page < 65_536, "snapshot exceeds pagination limit");
    }
    for podcast in &changes.podcasts {
        match api.get_podcast_auto_playlists(podcast.id).await {
            Ok(rows) => {
                changes.auto_playlists.insert(podcast.id, rows);
            }
            Err(error)
                if matches!(
                    halogen_sync_policy::status_of_error(&error),
                    Some(403 | 404)
                ) =>
            {
                changes.auto_playlists.insert(podcast.id, Vec::new());
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(changes)
}

/// Canonical host recovery image; absent until an initial snapshot has committed.
pub async fn cached_snapshot(store: &dyn LocalStore) -> Result<Option<StoreChanges>> {
    let Some(cursor) = store.sync_cursor().await? else {
        return Ok(None);
    };
    Ok(Some(StoreChanges {
        sync_cursor: Some(cursor),
        reset_cache: true,
        podcasts: store.list_podcasts().await?,
        episodes: store
            .list_episodes_page(&EpisodeQuery {
                size: i32::MAX,
                ..Default::default()
            })
            .await?,
        playlists: store.list_playlists().await?,
        playbacks: store.list_playbacks().await?,
        auto_playlists: store.list_auto_playlists().await?,
        ..Default::default()
    }))
}
