//! Local-first playback routing for [`PlayerController`](super::PlayerController): choose device bytes, server
//! streaming, or download. Backend wiring, reflected state, and polling remain in controller.rs.

use dioxus::prelude::*;

use super::{MediaSource, NowPlaying, PlaybackState, PlayerController};
use halogen_webui_app_state::ClientDownloadState;
use halogen_webui_commands::Command;
use halogen_webui_config::PlaybackPreference;
use halogen_webui_logging::{info, warn};

impl PlayerController {
    /// Stop current audio and enter Preparing until the download watcher sees Downloaded. Prime the backend inside this
    /// user gesture so delayed playback is still allowed by browser autoplay policy.
    pub(super) fn enter_preparing(&self, episode_id: i32) {
        self.bump_generation(); // invalidate any in-flight source resolution
        self.backend.borrow().stop();
        self.backend.borrow().prime();
        self.set_now_playing(NowPlaying::preparing(episode_id));
    }

    /// `DownloadOnly` has no playable device copy — (re-)fetch it to the device
    /// and show the spinner. NEVER streams. If the state still claims
    /// `Downloaded` (stale: bytes evicted/corrupt, or no media store on this
    /// platform), clear it first so the worker doesn't no-op the re-request.
    pub(super) fn redownload_and_prepare(&self, episode_id: i32, stale_downloaded: bool) {
        if stale_downloaded {
            self.sync_handle.send(Command::RemoveDownload {
                episode_ids: vec![episode_id],
            });
        }
        self.sync_handle.send(Command::DownloadToDevice {
            episode_ids: vec![episode_id],
        });
        self.enter_preparing(episode_id);
    }

    /// React to `episode_id`'s device-download state landing while we're `Preparing` it (the `PlayerProvider` watches
    /// `client_downloads` and calls this): play it once the bytes are `Downloaded`, or surface an error if the download
    /// `Failed`. No-op unless we're currently preparing exactly this episode, so every write to `now_playing` stays
    /// inside the controller.
    pub fn handle_preparing_download_state(
        &self,
        episode_id: i32,
        state: Option<ClientDownloadState>,
    ) {
        let preparing = self
            .now_playing
            .peek()
            .as_ref()
            .is_some_and(|n| n.episode_id == episode_id && n.state == PlaybackState::Preparing);
        if !preparing {
            return;
        }
        match state {
            Some(ClientDownloadState::Downloaded) => self.play_episode(episode_id),
            Some(ClientDownloadState::Failed) => {
                // Stop first: the silent autoplay primer (see `enter_preparing`)
                // would otherwise keep looping — silently holding audio focus —
                // behind the error state.
                self.backend.borrow().stop();
                self.set_error(episode_id, "Download failed");
            }
            // Still downloading: forward progress, so give the `Preparing` stall
            // guard (`check_preparing_stall`) a fresh window. `None` (no state yet)
            // leaves the window running.
            Some(ClientDownloadState::Downloading) => self.note_preparing_progress(),
            None => {}
        }
    }

    /// Prefer device bytes. If missing, streaming modes fall back to the server while DownloadOnly downloads again.
    /// Async source resolution uses a generation guard so an older load cannot replace newer user intent.
    pub fn play_episode(&self, episode_id: i32) {
        let generation = self.bump_generation();
        let this = self.clone();
        spawn(async move {
            let device_downloaded = this.downloads.peek().device_downloaded(episode_id);
            let start_at = this.saved_cursor(episode_id);

            // 1) Device copy.
            if device_downloaded && let Some(media) = this.media.as_ref() {
                match media.audio_url(episode_id).await {
                    Ok(Some(local)) => {
                        if this.load_generation.get() != generation {
                            // Superseded by a newer load: revoke the blob URL we
                            // just minted (the backend never took ownership, so it
                            // would otherwise leak for the page life).
                            #[cfg(target_arch = "wasm32")]
                            halogen_webui_media::revoke_object_url(&local.url);
                            return;
                        }
                        this.load_and_play(episode_id, MediaSource::Local(local), start_at);
                        return;
                    }
                    // `None` = no committed record (evicted); `Err` = a record
                    // whose bytes stayed unreadable through the store's probe
                    // retries. Either way there's nothing playable on-device.
                    Ok(None) | Err(_) => {
                        warn!(
                            episode_id,
                            "Device copy marked Downloaded but bytes missing (evicted/corrupt)"
                        );
                        // Lost bytes. DownloadOnly: never stream — clear the
                        // stale state and re-download.
                        if this.preference() == PlaybackPreference::DownloadOnly {
                            if this.load_generation.get() != generation {
                                return;
                            }
                            this.redownload_and_prepare(episode_id, true);
                            return;
                        }
                        // Streaming modes fall through to the server copy.
                    }
                }
            }

            if this.load_generation.get() != generation {
                return;
            }

            // 2) Server copy (streaming prefs / lost-bytes fallback). Funnel through
            //    `stream_episode`, the single chokepoint that enforces "DownloadOnly
            //    NEVER streams" — under that preference it refuses and (re-)downloads
            //    instead, so we don't re-check the preference here.
            this.stream_episode(episode_id);
        });
    }

    /// Stream the SERVER copy explicitly (the "Stream from server" action, the
    /// streaming playback preferences, and the lost-bytes fallback). Creates no
    /// device copy.
    pub fn stream_episode(&self, episode_id: i32) {
        // Invariant: DownloadOnly NEVER streams (not even from the server). Any
        // path that reaches here under DownloadOnly is a bug — refuse and
        // (re-)fetch to the device instead of silently streaming.
        if self.preference() == PlaybackPreference::DownloadOnly {
            warn!(
                episode_id,
                "stream_episode under DownloadOnly; refusing to stream"
            );
            let downloaded = self.downloads.peek().device_downloaded(episode_id);
            self.redownload_and_prepare(episode_id, downloaded);
            return;
        }
        self.bump_generation();
        let resolved = {
            let st = self.app_state.peek();
            let cfg = self.config.peek();
            st.audio_url_for_episode(
                episode_id,
                cfg.server_url.as_deref(),
                cfg.access_token.as_deref(),
            )
            .map(|url| (MediaSource::Remote(url), self.saved_cursor(episode_id)))
        };
        match resolved {
            Some((src, start_at)) => {
                info!(episode_id, "Streaming from server");
                self.load_and_play(episode_id, src, start_at);
            }
            None => {
                // No server copy: stop any audio still playing underneath so the
                // error state and what's audible agree.
                self.backend.borrow().stop();
                self.set_error(
                    episode_id,
                    "Not available yet — this episode isn't downloaded on the server.",
                );
            }
        }
    }

    /// [`request_play`](Self::request_play) from a playlist list: sets the play context (the continuation playlist for
    /// auto-advance / "up next" / transport next-prev) before routing. `None` resets to queue semantics. The
    /// context-less methods below preserve the current context, they're also the continuation paths (auto-advance,
    /// transport next/prev, the stream fallback, the download-then-play watcher handoff).
    pub fn request_play_in(&self, episode_id: i32, context: Option<i32>) {
        self.set_play_context(context);
        self.request_play(episode_id);
    }

    /// [`stream_episode`](Self::stream_episode) from a playlist list: sets the
    /// play context first. See [`request_play_in`](Self::request_play_in).
    pub fn stream_episode_in(&self, episode_id: i32, context: Option<i32>) {
        self.set_play_context(context);
        self.stream_episode(episode_id);
    }

    /// [`download_and_play`](Self::download_and_play) from a playlist list: sets
    /// the play context first. See [`request_play_in`](Self::request_play_in).
    pub fn download_and_play_in(&self, episode_id: i32, context: Option<i32>) {
        self.set_play_context(context);
        self.download_and_play(episode_id);
    }

    /// DownloadOnly waits for device bytes; StreamFirstAndDownload also fetches while streaming; StreamFallback streams
    /// only without local bytes; StreamOnly always streams. Preserve the continuation context; list entry points use
    /// [`request_play_in`](Self::request_play_in).
    pub fn request_play(&self, episode_id: i32) {
        // Persist the OUTGOING episode's position before we switch sources, a direct play→play switch (tapping a new
        // episode mid-playback) otherwise loses up to the last cursor-debounce window (~10s). Gate on Playing/Paused so
        // the auto-advance path is excluded: there the just-ended track was already marked played (cursor reset to 0)
        // and latched to `Ended`, so re-saving its near-end position would undo that.
        let outgoing = self
            .now_playing
            .peek()
            .as_ref()
            .map(|n| (n.episode_id, n.state.clone()));
        if let Some((ep, state)) = outgoing
            && ep != episode_id
            && matches!(state, PlaybackState::Playing | PlaybackState::Paused)
        {
            self.persist_cursor();
        }

        let device = self.downloads.peek().device_state(episode_id);
        info!(
            episode_id,
            preference = ?self.preference(),
            device = ?device,
            "Play requested"
        );

        // A fresh play intent gets a fresh local-error retry budget (the budget
        // exists per playback start, not per session).
        self.reset_local_retries();

        match self.preference() {
            PlaybackPreference::DownloadOnly => self.download_and_play(episode_id),
            PlaybackPreference::StreamFirstAndDownload => {
                if device == Some(ClientDownloadState::Downloaded) {
                    self.play_episode(episode_id);
                } else {
                    // Background download (worker no-ops if already in flight).
                    self.sync_handle.send(Command::DownloadToDevice {
                        episode_ids: vec![episode_id],
                    });
                    self.stream_episode(episode_id);
                }
            }
            PlaybackPreference::StreamFallback => {
                if device == Some(ClientDownloadState::Downloaded) {
                    self.play_episode(episode_id);
                } else {
                    self.stream_episode(episode_id);
                }
            }
            PlaybackPreference::StreamOnly => self.stream_episode(episode_id),
        }
    }

    /// Force download-then-play regardless of preference. Existing bytes play immediately; otherwise enter Preparing
    /// while the worker chains server/device downloads, and the download-state watcher starts playback when bytes land.
    pub fn download_and_play(&self, episode_id: i32) {
        let device = self.downloads.peek().device_state(episode_id);
        info!(episode_id, device = ?device, "Download & play requested");
        match device {
            Some(ClientDownloadState::Downloaded) => self.play_episode(episode_id),
            Some(ClientDownloadState::Downloading) => self.enter_preparing(episode_id),
            Some(ClientDownloadState::Failed) | None => {
                self.sync_handle.send(Command::DownloadToDevice {
                    episode_ids: vec![episode_id],
                });
                self.enter_preparing(episode_id);
            }
        }
    }
}
