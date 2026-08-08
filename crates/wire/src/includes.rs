//! Concrete include types for eager loading relationships.
//!
//! Each entity defines its own include enum that implements the `Includable` trait.

use serde::{Deserialize, Serialize};

use super::meta::includes::Includable;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum PodcastInclude {
    #[default]
    PodcastConfig,
}

impl Includable for PodcastInclude {}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum EpisodeInclude {
    #[default]
    Podcast,
    /// Embed the caller's resume cursor (`EpisodeData.playback`). Scoped to the
    /// authenticated user via the `playback` pivot `(user_id, episode_id)`; the
    /// server never returns another user's cursor.
    Playback,
    /// Embed the episode's ordered chapter markers (`EpisodeData.chapters`).
    /// Read-only, shared across users; absent when the episode has no chapters.
    Chapters,
}

impl Includable for EpisodeInclude {}
