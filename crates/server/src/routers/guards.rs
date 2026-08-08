//! Authorization guards — "is this actor allowed to do this?" checks run in the
//! router body AFTER validation and BEFORE delegating to the handler.
//!
//! Every guard takes the authenticated [`Actor`] (id + admin flag) and the id of
//! the resource being acted on, does a minimal ownership lookup, and returns
//! `Ok(())` or an `ApiError`:
//!   - **403** (`unauthorized` — authenticated but not permitted) when the
//!     resource exists but the actor doesn't own it
//!   - **404** (`exists` — the referenced thing doesn't exist) when the resource
//!     is absent (also keeps reads from leaking existence to non-owners)
//!
//! Admins bypass every ownership guard.

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect};

use halogen_orm::{episode, playlist, podcast, podcast_config, user, user_podcast};
use halogen_utils::constants::{
    VALIDATION_DATABASE_FIELD, VALIDATION_EXISTS_CODE, VALIDATION_ID_FIELD, VALIDATION_PANIC_CODE,
    VALIDATION_REQUEST_FIELD, VALIDATION_UNAUTHORIZED_CODE,
};

use crate::routers::errors::ApiError;
use crate::routers::extractors::Actor;

/// 403 — authenticated but not permitted to act on this resource.
fn forbidden(msg: &str) -> ApiError {
    ApiError::new(
        VALIDATION_REQUEST_FIELD,
        VALIDATION_UNAUTHORIZED_CODE,
        msg.to_string(),
    )
}

/// 404 — the resource doesn't exist (also used so reads don't reveal existence).
/// Keyed `id`/`exists`: location is the looked-up id, reason is non-existence.
fn not_found(msg: &str) -> ApiError {
    ApiError::new(VALIDATION_ID_FIELD, VALIDATION_EXISTS_CODE, msg.to_string())
}

/// Map an unexpected DB error to a 500. Logs the cause; the response stays
/// generic (`database`/`panic`), leaking nothing.
fn db_err(e: sea_orm::DbErr) -> ApiError {
    tracing::warn!("guard db error: {e}");
    ApiError::new(
        VALIDATION_DATABASE_FIELD,
        VALIDATION_PANIC_CODE,
        "Database error".to_string(),
    )
}

/// A podcast's `owner_id`, or `None` if the podcast doesn't exist.
async fn podcast_owner(dbc: &DatabaseConnection, podcast_id: i32) -> Result<Option<i32>, ApiError> {
    podcast::Entity::find_by_id(podcast_id)
        .select_only()
        .column(podcast::Column::OwnerId)
        .into_tuple::<i32>()
        .one(dbc)
        .await
        .map_err(db_err)
}

/// PUT/DELETE a podcast: owner or admin only.
pub async fn require_podcast_owner_or_admin(
    dbc: &DatabaseConnection,
    actor: Actor,
    podcast_id: i32,
) -> Result<(), ApiError> {
    if actor.is_admin {
        return Ok(());
    }
    match podcast_owner(dbc, podcast_id).await? {
        None => Err(not_found("Podcast not found")),
        Some(owner) if owner == actor.id => Ok(()),
        Some(_) => Err(forbidden(
            "Only the owner or an admin may modify this podcast",
        )),
    }
}

/// Writing a podcast's config (per-podcast routes + auto-playlists) is gated on
/// owning the podcast (or admin).
pub async fn require_podcast_config_writer(
    dbc: &DatabaseConnection,
    actor: Actor,
    podcast_id: i32,
) -> Result<(), ApiError> {
    if actor.is_admin {
        return Ok(());
    }
    match podcast_owner(dbc, podcast_id).await? {
        None => Err(not_found("Podcast not found")),
        Some(owner) if owner == actor.id => Ok(()),
        Some(_) => Err(forbidden(
            "Only the owner or an admin may change this podcast's config",
        )),
    }
}

/// PUT/DELETE a standalone `/podcast-configs/{id}`: gated on owning the podcast
/// that references the config (or admin). A config no podcast references is
/// owner-undecidable → forbidden for non-admins.
pub async fn require_config_owner_or_admin(
    dbc: &DatabaseConnection,
    actor: Actor,
    config_id: i32,
) -> Result<(), ApiError> {
    if actor.is_admin {
        return Ok(());
    }
    let exists = podcast_config::Entity::find_by_id(config_id)
        .one(dbc)
        .await
        .map_err(db_err)?
        .is_some();
    if !exists {
        return Err(not_found("Podcast config not found"));
    }
    let owner = podcast::Entity::find()
        .filter(podcast::Column::PodcastConfigId.eq(config_id))
        .select_only()
        .column(podcast::Column::OwnerId)
        .into_tuple::<i32>()
        .one(dbc)
        .await
        .map_err(db_err)?;
    match owner {
        Some(owner) if owner == actor.id => Ok(()),
        _ => Err(forbidden(
            "Only the owner or an admin may modify this podcast config",
        )),
    }
}

/// PUT/DELETE a playlist (and its membership routes): owner or admin only.
pub async fn require_playlist_owner_or_admin(
    dbc: &DatabaseConnection,
    actor: Actor,
    playlist_id: i32,
) -> Result<(), ApiError> {
    // Existence is checked first (even for admins) so acting on a missing playlist
    // is a clean 404, not a downstream FK 500.
    let owner = playlist::Entity::find_by_id(playlist_id)
        .select_only()
        .column(playlist::Column::UserId)
        .into_tuple::<i32>()
        .one(dbc)
        .await
        .map_err(db_err)?;
    match owner {
        None => Err(not_found("Playlist not found")),
        Some(_) if actor.is_admin => Ok(()),
        Some(owner) if owner == actor.id => Ok(()),
        Some(_) => Err(forbidden(
            "Only the owner or an admin may modify this playlist",
        )),
    }
}

/// Create/update/delete an episode: gated on owning the episode's podcast (or
/// admin). For create, guard the parent podcast id directly with
/// [`require_podcast_owner_or_admin`].
pub async fn require_episode_writer(
    dbc: &DatabaseConnection,
    actor: Actor,
    episode_id: i32,
) -> Result<(), ApiError> {
    if actor.is_admin {
        return Ok(());
    }
    let podcast_id = episode::Entity::find_by_id(episode_id)
        .select_only()
        .column(episode::Column::PodcastId)
        .into_tuple::<i32>()
        .one(dbc)
        .await
        .map_err(db_err)?;
    let Some(podcast_id) = podcast_id else {
        return Err(not_found("Episode not found"));
    };
    require_podcast_owner_or_admin(dbc, actor, podcast_id).await
}

/// Trigger/remove a server-side download of an episode: gated on being subscribed
/// to the episode's podcast (or owning it / admin) — the same access read gives.
/// Resolves the episode's `podcast_id`, then defers to [`require_subscribed`].
/// Returns **404** for a missing episode (and for non-subscribers, since
/// `require_subscribed` hides existence).
pub async fn require_episode_subscribed(
    dbc: &DatabaseConnection,
    actor: Actor,
    episode_id: i32,
) -> Result<(), ApiError> {
    if actor.is_admin {
        return Ok(());
    }
    let podcast_id = episode::Entity::find_by_id(episode_id)
        .select_only()
        .column(episode::Column::PodcastId)
        .into_tuple::<i32>()
        .one(dbc)
        .await
        .map_err(db_err)?;
    let Some(podcast_id) = podcast_id else {
        return Err(not_found("Episode not found"));
    };
    require_subscribed(dbc, actor, podcast_id).await
}

/// Build an [`Actor`] from a bare authenticated user id, resolving the admin flag
/// from the DB the way the API middleware does.
///
/// The media routes (`/episodes/{id}/audio` + `/art`) authenticate by decoding a
/// signed token directly (see `media_auth`) rather than going through the `Actor`
/// extractor, so they have an id but not the admin flag. This recovers it so the
/// subsequent [`require_episode_subscribed`] applies the same admin-bypass /
/// owner / subscriber rules as every other episode route. An unknown id resolves
/// to a non-admin actor — the subscription guard then 404s it.
pub async fn actor_from_id(dbc: &DatabaseConnection, user_id: i32) -> Actor {
    let is_admin = user::Entity::find_by_id(user_id)
        .select_only()
        .column(user::Column::IsAdmin)
        .into_tuple::<bool>()
        .one(dbc)
        .await
        .ok()
        .flatten()
        .unwrap_or(false);
    Actor {
        id: user_id,
        is_admin,
    }
}

/// Read gate for a single podcast: the actor must be subscribed (or own it / be
/// admin). Returns **404** (not 403) so non-subscribers can't probe existence.
pub async fn require_subscribed(
    dbc: &DatabaseConnection,
    actor: Actor,
    podcast_id: i32,
) -> Result<(), ApiError> {
    if actor.is_admin {
        return Ok(());
    }
    let subscribed = user_podcast::Entity::find_by_id((actor.id, podcast_id))
        .one(dbc)
        .await
        .map_err(db_err)?
        .is_some();
    if subscribed {
        return Ok(());
    }
    // An owner who hasn't got a pivot row (shouldn't happen, but be lenient) can
    // still read their own podcast.
    match podcast_owner(dbc, podcast_id).await? {
        Some(owner) if owner == actor.id => Ok(()),
        _ => Err(not_found("Podcast not found")),
    }
}

/// Read/modify a user record: the actor must be that same user, or an admin.
///
/// Unlike the resource guards this needs no DB lookup — identity is already in
/// the [`Actor`]. A non-self, non-admin actor gets a **403** (the actor knows
/// their own id, so refusing another id leaks nothing about whether it exists).
pub fn require_self_or_admin(actor: Actor, target_user_id: i32) -> Result<(), ApiError> {
    if actor.is_admin || actor.id == target_user_id {
        Ok(())
    } else {
        Err(forbidden("You may only access your own account"))
    }
}
