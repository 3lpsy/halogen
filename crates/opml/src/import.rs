//! Insert podcasts parsed from OPML. Only `type="rss"` outlines with a non-empty
//! `xmlUrl` become rows; existing feeds (matched by `feed_url`) are skipped, not
//! updated. The owner is subscribed to each imported feed (idempotently) so the
//! library is actually visible — the episode/podcast list endpoints scope to
//! `user_podcast` subscriptions, not ownership. Episodes are populated by a
//! subsequent feed sync, not here.

use std::path::Path;

use chrono::Utc;
use halogen_orm::podcast::ActiveModel as PodcastActiveModel;
use halogen_orm::podcast::Column;
use halogen_orm::podcast::Entity as PodcastEntity;
use halogen_orm::user_podcast;
use halogen_utils::opml::{ImportPodcastResult, OpmlDocument};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use tracing::{info, warn};

/// Import podcasts from an OPML file on disk (server startup path).
pub async fn import_podcasts_from_opml(
    dbc: &sea_orm::DatabaseConnection,
    opml_path: &Path,
    owner_id: i32,
) -> Result<ImportPodcastResult, String> {
    info!("Importing podcasts from OPML file: {}", opml_path.display());

    let doc = super::parser::parse_opml_file(opml_path)
        .map_err(|e| format!("Failed to parse OPML file: {}", e))?;

    import_podcasts_from_doc(dbc, &doc, owner_id).await
}

/// Import podcasts from raw OPML XML (the admin upload path). Bare inserts only;
/// episodes are populated by a subsequent feed sync.
pub async fn import_podcasts_from_opml_str(
    dbc: &sea_orm::DatabaseConnection,
    xml: &str,
    owner_id: i32,
) -> Result<ImportPodcastResult, String> {
    let doc = halogen_utils::opml::parse_opml_str(xml)
        .map_err(|e| format!("Failed to parse OPML: {}", e))?;

    import_podcasts_from_doc(dbc, &doc, owner_id).await
}

/// Shared per-outline insert loop used by both the file and string entrypoints.
async fn import_podcasts_from_doc(
    dbc: &sea_orm::DatabaseConnection,
    doc: &OpmlDocument,
    owner_id: i32,
) -> Result<ImportPodcastResult, String> {
    let mut created_count = 0;
    let mut skipped_count = 0;
    let mut error_count = 0;
    let mut podcast_names = Vec::new();

    for outline in &doc.outlines {
        if outline.outline_type.as_deref() != Some("rss") {
            continue;
        }

        let Some(feed_url) = &outline.xml_url else {
            warn!("Skipping podcast '{}' - no xmlUrl attribute", outline.text);
            skipped_count += 1;
            continue;
        };

        if feed_url.is_empty() {
            warn!("Skipping podcast '{}' - empty xmlUrl", outline.text);
            skipped_count += 1;
            continue;
        }

        let title = outline.text.clone();
        match import_single_podcast(dbc, &title, feed_url, owner_id).await {
            Ok(true) => {
                created_count += 1;
                podcast_names.push(title);
            }
            Ok(false) => {
                skipped_count += 1;
            }
            Err(e) => {
                warn!("Failed to import podcast '{}': {}", title, e);
                error_count += 1;
            }
        }
    }

    info!(
        "OPML import completed: {} created, {} skipped, {} errors",
        created_count, skipped_count, error_count
    );

    Ok(ImportPodcastResult {
        total: podcast_names.len(),
        created: created_count,
        skipped: skipped_count,
        errors: error_count,
        podcast_names,
    })
}

pub async fn import_single_podcast(
    dbc: &sea_orm::DatabaseConnection,
    title: &str,
    feed_url: &str,
    owner_id: i32,
) -> Result<bool, String> {
    let existing = PodcastEntity::find()
        .filter(Column::FeedUrl.eq(feed_url))
        .one(dbc)
        .await
        .map_err(|e| format!("Failed to check if podcast exists: {}", e))?;

    if let Some(existing) = existing {
        // Already imported on a prior run — but a podcast created before this code
        // existed is owned-but-unsubscribed, so still ensure the owner subscription.
        // This is how an existing dev/prod DB heals on the next restart.
        subscribe_owner(dbc, owner_id, existing.id).await?;
        info!("Podcast '{}' already exists, skipping", title);
        return Ok(false);
    }

    let now = Utc::now();
    let podcast = PodcastActiveModel {
        id: sea_orm::ActiveValue::NotSet,
        title: Set(title.to_string()),
        description: Set(String::new()),
        feed_url: Set(feed_url.to_string()),
        art_url: Set(None),
        art_file_path: Set(None),
        author: Set(None),
        etag: Set(None),
        last_modified: Set(None),
        feed_url_redirects: Set(None),
        polled_at: Set(None),
        podcast_config_id: Set(None),
        owner_id: Set(owner_id),
        created_at: Set(now),
        updated_at: Set(now),
    };

    let inserted = podcast
        .insert(dbc)
        .await
        .map_err(|e| format!("Failed to insert podcast '{}': {}", title, e))?;

    subscribe_owner(dbc, owner_id, inserted.id).await?;

    info!("Successfully imported podcast: {}", title);
    Ok(true)
}

/// Idempotently subscribe `owner_id` to `podcast_id` (the `user_podcast` junction
/// has a composite PK `(user_id, podcast_id)`). Without this row the imported feed
/// is owned but invisible, since `GET /episodes` and `GET /podcasts` scope to
/// subscriptions. Mirrors the manual add-podcast path in `podcast_store`.
async fn subscribe_owner(
    dbc: &sea_orm::DatabaseConnection,
    owner_id: i32,
    podcast_id: i32,
) -> Result<(), String> {
    let already = user_podcast::Entity::find_by_id((owner_id, podcast_id))
        .one(dbc)
        .await
        .map_err(|e| format!("Failed to check subscription: {}", e))?
        .is_some();
    if already {
        return Ok(());
    }

    let now = Utc::now();
    user_podcast::ActiveModel {
        user_id: Set(owner_id),
        podcast_id: Set(podcast_id),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(dbc)
    .await
    .map_err(|e| format!("Failed to subscribe owner to podcast {}: {}", podcast_id, e))?;

    Ok(())
}
