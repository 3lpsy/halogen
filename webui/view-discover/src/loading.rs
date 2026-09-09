use dioxus::prelude::*;
use halogen_webui_app_state::{DiscoverMode, DiscoverState};
use halogen_webui_config::ClientConfig;
use halogen_wire::DiscoverPageParams;

pub fn load_page(mut store: Signal<DiscoverState>, config: Signal<ClientConfig>) {
    let Some(client) = config.read().api_client() else {
        return;
    };
    let (generation, mode, params) = {
        let mut state = store.write();
        if state.loading || state.query.trim().chars().count() < 2 || state.providers.is_empty() {
            return;
        }
        if state.page.as_ref().is_some_and(|page| !page.has_more) {
            return;
        }
        state.loading = true;
        state.searched = true;
        state.failure = None;
        (
            state.generation,
            state.mode,
            DiscoverPageParams {
                q: state.query.clone(),
                providers: Some(state.providers.clone()),
                cursor: state
                    .page
                    .as_ref()
                    .and_then(|page| page.next_cursor.clone()),
            },
        )
    };
    spawn(async move {
        match mode {
            DiscoverMode::Podcasts => {
                let response = client.discover_podcast_page(params).await;
                if store.peek().generation != generation {
                    return;
                }
                let mut state = store.write();
                match response {
                    Ok(data) => state.append_podcasts(data),
                    Err(error) => state.failure = Some(error.to_string()),
                }
                state.loading = false;
            }
            DiscoverMode::Episodes => {
                let response = client.discover_episode_page(params).await;
                if store.peek().generation != generation {
                    return;
                }
                let mut state = store.write();
                match response {
                    Ok(data) => state.append_episodes(data),
                    Err(error) => state.failure = Some(error.to_string()),
                }
                state.loading = false;
            }
        }
    });
}
