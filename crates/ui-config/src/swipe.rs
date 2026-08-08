//! Per-page episode swipe-action preferences persisted in
//! [`ClientConfig`](super::ClientConfig).

use serde::{Deserialize, Serialize};

use halogen_ui_listview::{SwipeAction, SwipeConfig};

/// User-configurable swipe actions per list page. Each side (`left` = swipe-right
/// gesture, `right` = swipe-left gesture) is an independent `SwipeConfig`. The
/// `Default` reproduces the historical hardcoded behavior so a fresh config (or an
/// old one without this field) keeps the same swipes.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct SwipePrefs {
    #[serde(default)]
    pub latest: SwipeConfig,
    #[serde(default)]
    pub queue: SwipeConfig,
    #[serde(default)]
    pub podcast: SwipeConfig,
    #[serde(default)]
    pub downloads: SwipeConfig,
    #[serde(default)]
    pub history: SwipeConfig,
    #[serde(default)]
    pub playlist: SwipeConfig,
}

impl Default for SwipePrefs {
    fn default() -> Self {
        use SwipeAction::*;
        // `left` fires on a swipe-RIGHT gesture, `right` on a swipe-LEFT gesture.
        let cfg = |swipe_right, swipe_left| SwipeConfig {
            left: Some(swipe_right),
            right: Some(swipe_left),
        };
        Self {
            // (swipe-right action, swipe-left action)
            latest: cfg(AddToQueue, DownloadToDevice),
            queue: cfg(RemoveFromQueue, DownloadToDevice),
            podcast: cfg(AddToPlaylist, DownloadToDevice),
            downloads: cfg(AddToPlaylist, RedownloadDevice),
            history: cfg(AddToPlaylist, DownloadToDevice),
            playlist: cfg(RemoveFromList, DownloadToDevice),
        }
    }
}

/// The list pages that expose configurable swipes. Drives both the per-page config
/// lookup and the configure-page UI (`SwipePage::ALL`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwipePage {
    Latest,
    Queue,
    Podcast,
    Downloads,
    History,
    Playlist,
}

impl SwipePage {
    pub const ALL: [SwipePage; 6] = [
        SwipePage::Latest,
        SwipePage::Queue,
        SwipePage::Podcast,
        SwipePage::Downloads,
        SwipePage::History,
        SwipePage::Playlist,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            SwipePage::Latest => "Latest",
            SwipePage::Queue => "Queue",
            SwipePage::Podcast => "Podcast",
            SwipePage::Downloads => "Downloads",
            SwipePage::History => "History",
            SwipePage::Playlist => "Playlist",
        }
    }

    /// This page's stored swipe config.
    pub fn get(&self, p: &SwipePrefs) -> SwipeConfig {
        match self {
            SwipePage::Latest => p.latest,
            SwipePage::Queue => p.queue,
            SwipePage::Podcast => p.podcast,
            SwipePage::Downloads => p.downloads,
            SwipePage::History => p.history,
            SwipePage::Playlist => p.playlist,
        }
    }

    /// Overwrite this page's swipe config.
    pub fn set(&self, p: &mut SwipePrefs, cfg: SwipeConfig) {
        match self {
            SwipePage::Latest => p.latest = cfg,
            SwipePage::Queue => p.queue = cfg,
            SwipePage::Podcast => p.podcast = cfg,
            SwipePage::Downloads => p.downloads = cfg,
            SwipePage::History => p.history = cfg,
            SwipePage::Playlist => p.playlist = cfg,
        }
    }

    /// Actions offered for this page, in menu order. `RemoveFromList` only makes
    /// sense where the list *is* a playlist (Queue, Playlist).
    pub fn allowed_actions(&self) -> Vec<SwipeAction> {
        SwipeAction::ALL
            .into_iter()
            .filter(|a| match a {
                SwipeAction::RemoveFromList => {
                    matches!(self, SwipePage::Queue | SwipePage::Playlist)
                }
                _ => true,
            })
            .collect()
    }
}
