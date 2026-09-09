use dioxus::prelude::*;
use halogen_webui_app_state::media_url;
use halogen_wire::{DownloadStatus, EpisodeInclude};

use crate::components::{
    Artwork, CloudProgressOrSpinner, ConfirmLinkModal, DetailHeaderBar, EpisodeMenuArgs,
    EpisodeRowState, KebabButton, MenuListContext, ProgressOrSpinner, RichText,
    episode_menu_sections, resolve_episode_row_state, use_confirm, use_quick_menu,
};
use halogen_webui_commands::actions as commands;
use halogen_webui_component_icons::{CloudArrowDown, Download, Pause, Play};
use halogen_webui_hooks::{
    deep_link_placeholder, use_config, use_connection, use_deep_link_resource, use_dispatch,
    use_downloads, use_episodes, use_now_playing_identity, use_playbacks, use_player_controller,
    use_playlists, use_podcasts,
};
use halogen_webui_platform::time::sleep_ms;

/// Key episode detail through a one-item list by ID. Parameter-only navigation otherwise reuses guards and poll budgets
/// from the old episode; Dioxus honors the key through keyed children.
#[component]
pub fn EpisodeDetail(id: i32) -> Element {
    rsx! {
        for id in [id] {
            EpisodeDetailBody { key: "{id}", id }
        }
    }
}

#[component]
fn EpisodeDetailBody(id: i32) -> Element {
    let app_state = use_episodes();
    let podcasts = use_podcasts();
    let playlists = use_playlists();
    let playbacks = use_playbacks();
    let downloads = use_downloads();
    let connection = use_connection();
    let player = use_player_controller();
    let playing = use_now_playing_identity();
    let dispatch = use_dispatch();
    let config = use_config();
    let quick = use_quick_menu();
    let confirm = use_confirm();
    let nav = use_navigator();
    // Pending external link awaiting user confirmation (from description links).
    let mut pending_link = use_signal(|| Option::<String>::None);

    // Episodes are no longer bulk-hydrated, so a deep-link (or any not-yet-browsed
    // episode) may not be in the pool. Fetch + cache it on miss; `load_failed`
    // distinguishes "loading" from "not found".
    let load_failed = use_deep_link_resource(
        config,
        move || app_state.read().episode(id).is_some(),
        move |client| async move {
            let ep = client
                .get_episode(id, &[EpisodeInclude::Playback])
                .await
                .map_err(|e| e.to_string())?;
            halogen_webui_commands::actions::cache_episodes(&dispatch, vec![ep]);
            Ok(())
        },
    );

    // Poll durable server-download status about every four seconds until completion or the cap, preventing overlapping
    // loops. This updates actions live and remains independent from the worker's one-second progress-ring endpoint.
    let mut polling = use_signal(|| false);
    // Sticky cap: once the bounded poll budget is spent we stop respawning. Without
    // this the cap was ineffective — `polling.set(false)` at the end re-runs this
    // effect, and a still-`Downloading` status immediately spawned another loop, so
    // it polled forever. After the cap the worker's periodic pull finishes the update.
    let mut poll_exhausted = use_signal(|| false);
    use_effect(move || {
        let downloading = app_state
            .read()
            .episode(id)
            .map(|e| e.download_status.clone())
            == Some(DownloadStatus::Downloading);
        if !downloading || polling() || poll_exhausted() {
            return;
        }
        polling.set(true);
        let cfg = config.peek().clone();
        spawn(async move {
            // Assume the cap was hit unless we break out early (status left
            // `Downloading`, or a fetch failed) — only then mark the budget spent.
            let mut exhausted = true;
            for _ in 0..20 {
                sleep_ms(4000).await;
                // Bail if it left `Downloading` by some other path (e.g. removed).
                let still = app_state
                    .peek()
                    .episode(id)
                    .map(|e| e.download_status.clone())
                    == Some(DownloadStatus::Downloading);
                if !still {
                    exhausted = false;
                    break;
                }
                let Some(client) = cfg.api_client() else {
                    exhausted = false;
                    break;
                };
                // Re-request Playback so re-caching this body doesn't blank the
                // embedded cursor (the rows read it for the progress bar).
                match client.get_episode(id, &[EpisodeInclude::Playback]).await {
                    Ok(ep) => {
                        let done = ep.download_status != DownloadStatus::Downloading;
                        commands::cache_episodes(&dispatch, vec![ep]);
                        if done {
                            exhausted = false;
                            break;
                        }
                    }
                    Err(_) => {
                        exhausted = false;
                        break;
                    }
                }
            }
            if exhausted {
                poll_exhausted.set(true);
            }
            polling.set(false);
        });
    });

    // Lazily pull the parent podcast into the pool for the title link. Dispatch AT MOST ONCE: this effect reads
    // EpisodeState (so it re-runs on every worker publish) and dispatching `EnsurePodcast` itself causes a publish,
    // without the guard that's an infinite publish→dispatch loop that starves the fetch. Once the guard flips, the
    // effect's only remaining dependency is the guard, so it stops re-running on EpisodeState.
    let mut podcast_requested = use_signal(|| false);
    use_effect(move || {
        if podcast_requested() {
            return;
        }
        let st = app_state.read();
        if let Some(ep) = st.episode(id)
            && podcasts.read().podcast(ep.podcast_id).is_none()
        {
            podcast_requested.set(true);
            commands::ensure_podcast(&dispatch, ep.podcast_id);
        }
    });

    let state = app_state.read();
    let Some(mut ep) = state.episode(id).cloned() else {
        drop(state);
        // Read reactively so the offline-vs-not-found wording updates when
        // connectivity returns (this read only happens in the placeholder branch, so
        // the found path keeps no connection subscription).
        let is_offline = connection.read().is_offline();
        return rsx! {
            div { class: "p-2",
                p { class: "text-muted",
                    {deep_link_placeholder(load_failed(), is_offline, "episode")}
                }
            }
        };
    };
    // Overlay-wins resume cursor for the progress bar: a just-seeked local cursor
    // beats the one embedded on the cached body.
    ep.playback = playbacks.read().playback_for(&state, id);
    // Podcast name from the local pool (episodes no longer carry a nested join);
    // a stale embedded copy is a last resort. Lazily fetched on miss (effect
    // above) with a '…' placeholder until it lands.
    let podcast_title = podcasts
        .read()
        .podcast(ep.podcast_id)
        .map(|p| p.title.clone())
        .or_else(|| ep.podcast.as_ref().map(|p| p.title.clone()))
        .unwrap_or_default();
    // Live per-episode state (download/queue/playback/play-gating) resolved by the SHARED builder, the SAME formulas
    // the list item uses, so the detail page can't drift from it (the hand-synced "Same rule as the list item"
    // invariants live in `row_state`). `ep` was cloned from this same pool, so the resolver reading the pool for `id`
    // sees identical server-download status.
    let row = resolve_episode_row_state(
        &state,
        &playlists.read(),
        &downloads.read(),
        &connection.read(),
        &playing.read(),
        id,
    );
    drop(state);
    let EpisodeRowState {
        downloaded_on_device,
        device_downloading,
        device_download_progress,
        server_downloaded,
        server_downloading,
        server_download_progress,
        is_current,
        is_playing,
        is_preparing,
        is_offline,
        play_disabled,
        ..
    } = row;

    let minutes = ep.duration_secs.map(|s| (s / 60).max(1));
    let published = ep
        .published_at
        .map(|d| d.format("%b %d, %Y").to_string())
        .unwrap_or_else(|| "Unknown".to_string());
    let progress_pct = ep.playback.as_ref().and_then(|p| {
        ep.duration_secs
            .filter(|d| *d > 0)
            .map(|d| ((p.cursor as f64 / d as f64).clamp(0.0, 1.0) * 100.0) as i32)
    });

    let playback_pref = config.read().playback_prefs.playback_preference;
    let embedded = config.read().server_kind.is_embedded();

    // Actions menu — the SAME builder the list-item kebab uses, minus "View
    // episode" (you're already on it). No playlist context here, so the reorder
    // section is empty (the menu host filters it out).
    let menu_title = ep.title.clone();
    let menu_sections = episode_menu_sections(
        EpisodeMenuArgs::from_row_state(
            &row,
            ep.podcast_id,
            playback_pref,
            embedded,
            MenuListContext::default(),
            false,
        ),
        player,
        dispatch,
        nav,
        confirm.purge_episode_callback(id),
    );

    rsx! {
        // Column layout: a pinned header (back/context row + art/title/date/play)
        // and a single scrollable region (the description). Mirrors the list views,
        // whose header bars stay put while the list scrolls — here it's the
        // description that scrolls beneath a fixed header.
        div { class: "flex flex-col h-full overflow-hidden",
        // Back-button row with the actions menu on the right — mirrors podcast
        // detail (kebab carries the download/stream actions).
        DetailHeaderBar {
            KebabButton {
                label: "Episode actions",
                onclick: move |_| quick.open(menu_title.clone(), menu_sections.clone()),
            }
        }
        div { class: "p-2",
            // Header — mirrors the list item: art (left, ~1/3 width) with the title
            // and podcast name stacked to its right.
            div { class: "flex gap-4 items-start",
                div { class: "flex-shrink-0 w-1/3 max-w-48 aspect-square rounded-lg bg-base-200 overflow-hidden flex items-center justify-center",
                    // Server art cache (optimistic; placeholder on miss). Detail view
                    // wants the full image, with the list's cached thumbnail shown
                    // instantly underneath while it loads.
                    Artwork {
                        src: media_url::art_url_for_episode(config.read().server_url.as_deref(), &ep),
                        placeholder_src: media_url::art_url_for_episode_small(config.read().server_url.as_deref(), &ep),
                        alt: "Episode art",
                        img_class: "w-full h-full object-cover",
                        placeholder_class: "text-3xl",
                    }
                }
                div { class: "min-w-0 flex-1",
                    h1 { class: "text-xl font-bold break-words", "{ep.title}" }
                    // Podcast name under the title — links to the podcast page;
                    // '…' while it lazily loads into the pool.
                    if !podcast_title.is_empty() {
                        Link {
                            to: format!("/podcasts/{id}", id = ep.podcast_id),
                            class: "text-sm font-medium text-primary hover:underline",
                            "{podcast_title}"
                        }
                    } else {
                        span { class: "text-sm font-medium text-muted", "…" }
                    }
                }
            }
            // Date on the left, length on the right.
            div { class: "flex items-center justify-between gap-2 mt-3 text-xs text-muted",
                span { "{published}" }
                if let Some(m) = minutes {
                    span { "{m} min" }
                }
            }

            // Play, the download/stream actions otherwise live in the back-row menu. When the episode is held neither
            // on the server nor this device (and nothing's in flight), the actionable control is the explicit "Download
            // & Play" instead: it forces fetch→device→play with progress regardless of the playback preference. The
            // plain Play button is disabled in that state anyway (nothing to stream, no local copy), so we swap it.
            div { class: "mt-4",
                if !server_downloaded && !downloaded_on_device
                    && !server_downloading && !device_downloading && !is_preparing
                {
                    // Embedded mode: a plain server download, the button flips to the Play state (with in-button
                    // progress) as the status streams in, and Play then streams from the built-in server.
                    // `download_and_play` must NOT be used here: it forces the device pipeline, which doesn't exist in
                    // embedded mode.
                    if embedded {
                        button {
                            class: "btn btn-primary gap-2 w-full sm:w-auto",
                            disabled: is_offline,
                            onclick: move |_| commands::download_on_server(&dispatch, vec![id]),
                            Download { class: "w-5 h-5" }
                            "Download"
                        }
                    } else {
                        button {
                            class: "btn btn-primary gap-2 w-full sm:w-auto",
                            disabled: is_offline,
                            onclick: move |_| player().download_and_play_in(id, None),
                            CloudArrowDown { class: "w-5 h-5" }
                            "Download & Play"
                        }
                    }
                } else {
                    button {
                        class: "btn btn-primary gap-2 w-full sm:w-auto",
                        disabled: play_disabled || is_preparing,
                        onclick: move |_| {
                            if is_current {
                                player().toggle();
                            } else {
                                // Contextless play: reset continuation to queue semantics.
                                player().request_play_in(id, None);
                            }
                        },
                        if is_preparing {
                            span { class: "loading loading-spinner" }
                            "Preparing…"
                        } else if is_playing {
                            Pause { class: "w-5 h-5" }
                            "Pause"
                        // Download progress lives INSIDE the button — no status rows
                        // beneath it. Device phase wins when both are in flight (it's
                        // the later phase: server copy → this device's bytes); cloud
                        // ring vs plain ring distinguishes the two.
                        } else if device_downloading {
                            ProgressOrSpinner { percent: device_download_progress }
                            if let Some(pct) = device_download_progress {
                                "Downloading to device… {pct}%"
                            } else {
                                "Downloading to device…"
                            }
                        } else if server_downloading {
                            // Embedded: plain ring + unqualified label (nothing is
                            // remote); otherwise the cloud ring marks the server phase.
                            if embedded {
                                ProgressOrSpinner { percent: server_download_progress }
                                if let Some(pct) = server_download_progress {
                                    "Downloading… {pct}%"
                                } else {
                                    "Downloading…"
                                }
                            } else {
                                CloudProgressOrSpinner { percent: server_download_progress }
                                if let Some(pct) = server_download_progress {
                                    "Downloading on server… {pct}%"
                                } else {
                                    "Downloading on server…"
                                }
                            }
                        } else {
                            Play { class: "w-5 h-5" }
                            "Play"
                        }
                    }
                }
            }

            if let Some(pct) = progress_pct {
                div { class: "w-full h-1 mt-3 bg-base-300 rounded",
                    div { class: "h-1 bg-primary rounded", style: "width: {pct}%" }
                }
            }
        }
        // Description — the ONLY scrollable region. Its top padding plus the
        // header's bottom padding reproduce the old `mt-4` gap below the play row.
        div { class: "flex-1 overflow-y-auto overflow-x-hidden overscroll-y-contain p-2",
            if let Some(desc) = ep.description.clone() {
                div { class: "text-sm text-base-content/80 break-words overflow-hidden",
                    RichText {
                        html: desc,
                        on_link: move |href| pending_link.set(Some(href)),
                    }
                }
            }
        }
        ConfirmLinkModal { pending: pending_link }
        }
    }
}
