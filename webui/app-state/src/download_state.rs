//! Per-episode device/server download state, the worker-owned reactive slice. Lives in its own root signal so a
//! download-progress byte update (which can arrive ~1 Hz per in-flight download) only re-renders download consumers,
//! the episode-row badges and the Downloads list, not every component that reads the main app state. The sync worker is
//! the only writer; the UI reads it reactively (see `halogen_webui_hooks::use_downloads`).

use std::collections::HashMap;

use crate::state::ClientDownloadState;

/// Device + server download state for episodes, keyed by episode id; the worker is the sole writer. `PartialEq` is
/// load-bearing here for the same reason as on `EpisodeState`: this is read through a signal and sliced by memos, so a
/// derive that silently degraded to "always changed" would defeat the segmentation. The `downloadstate_is_partialeq`
/// canary guards it.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct DownloadState {
    /// Per-episode DEVICE download state (distinct from the server's
    /// `download_status`). `Downloading`/`Failed` are in-memory only;
    /// `Downloaded` is re-derived at boot from the media store's committed
    /// contents. See [`ClientDownloadState`] for the lifecycle.
    pub client_downloads: HashMap<i32, ClientDownloadState>,
    /// Download progress percent (0–100) for episodes currently `Downloading` on
    /// THIS device, updated as chunks land. Only present while in flight (cleared
    /// on complete/fail/remove); absent before the first byte.
    pub download_progress: HashMap<i32, u8>,
    /// SERVER-download progress percent (0–100) for episodes whose server copy is
    /// being fetched, mirrored from the server's progress tracker (polled ≈1 Hz).
    /// Distinct from `download_progress`, which is the DEVICE byte fetch.
    pub server_download_progress: HashMap<i32, u8>,
}

impl DownloadState {
    /// This episode's device download state, if any.
    pub fn device_state(&self, episode_id: i32) -> Option<ClientDownloadState> {
        self.client_downloads.get(&episode_id).copied()
    }

    /// Download progress percent (0–100) for an in-flight device download, or
    /// `None` before the first byte / when not downloading.
    pub fn download_pct(&self, episode_id: i32) -> Option<u8> {
        self.download_progress.get(&episode_id).copied()
    }

    /// Server-download progress percent (0–100) for an in-flight SERVER download,
    /// or `None` before the tracker reports / when the server isn't downloading it.
    pub fn server_download_pct(&self, episode_id: i32) -> Option<u8> {
        self.server_download_progress.get(&episode_id).copied()
    }

    /// True when the episode's bytes are stored on this device (playable offline).
    pub fn device_downloaded(&self, episode_id: i32) -> bool {
        self.device_state(episode_id) == Some(ClientDownloadState::Downloaded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time canary for the load-bearing derive (mirrors
    /// `appstate_is_partialeq`): memo slices over this signal need
    /// `DownloadState: PartialEq` to gate re-renders.
    #[test]
    fn downloadstate_is_partialeq() {
        fn assert_partial_eq<T: PartialEq>() {}
        assert_partial_eq::<DownloadState>();
    }

    #[test]
    fn accessors_read_the_maps() {
        let mut d = DownloadState::default();
        d.client_downloads
            .insert(1, ClientDownloadState::Downloaded);
        d.client_downloads
            .insert(2, ClientDownloadState::Downloading);
        d.download_progress.insert(2, 40);
        d.server_download_progress.insert(3, 75);

        assert_eq!(d.device_state(1), Some(ClientDownloadState::Downloaded));
        assert!(d.device_downloaded(1));
        assert!(!d.device_downloaded(2));
        assert_eq!(d.download_pct(2), Some(40));
        assert_eq!(d.download_pct(1), None);
        assert_eq!(d.server_download_pct(3), Some(75));
        assert_eq!(d.server_download_pct(1), None);
    }
}
