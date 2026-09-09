//! DTOs for the admin DB export/import endpoints (`/admin/db/export|import`)
//! — the server↔server / embedded↔server migration and full-backup story.

use serde::{Deserialize, Serialize};

use crate::meta::response::ResponsableData;

/// Per-entity import results. Merge users by normalized username, podcasts by shared feed URL, episodes by
/// guid/content_url within a podcast, and playlists by user/default or user/name. created_usernames identifies new
/// random-password accounts so local hosts can provision silent-login secrets.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DbImportSummaryData {
    pub users_merged: u32,
    pub users_created: u32,
    pub created_usernames: Vec<String>,
    pub podcasts_merged: u32,
    pub podcasts_created: u32,
    pub subscriptions_created: u32,
    pub episodes_merged: u32,
    pub episodes_created: u32,
    pub chapters_created: u32,
    pub playbacks_upserted: u32,
    pub statuses_upserted: u32,
    pub playlists_merged: u32,
    pub playlists_created: u32,
    pub playlist_links_created: u32,
    pub auto_playlists_created: u32,
}

impl ResponsableData for DbImportSummaryData {}
