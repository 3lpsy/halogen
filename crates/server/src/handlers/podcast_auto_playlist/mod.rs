//! Handlers for a podcast's auto-playlist set — the playlists every newly
//! ingested episode is appended to. Backs the nested
//! `GET`/`PUT /podcasts/{id}/auto-playlists` endpoints (see
//! `routers/podcasts/auto_playlists.rs`).
pub mod get_for_podcast;
pub mod set_for_podcast;
