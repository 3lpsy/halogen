//! DTOs for the admin DB export/import endpoints (`/admin/db/export|import`)
//! — the server↔server / embedded↔server migration and full-backup story.

use serde::{Deserialize, Serialize};

use crate::meta::response::ResponsableData;

/// What `POST /admin/db/import` did, per entity. Imports MERGE (never drop and
/// replace): users match by lowercased username, podcasts by `(owner,
/// feed_url)`, episodes by guid (falling back to `content_url`) within a
/// podcast, playlists by the per-user default flag or `(user, name)`.
/// `created_usernames` lists users the import created (with random passwords —
/// hashes are stripped from exports); an embedded host uses it to re-provision
/// its silent-login secrets for those users.
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
