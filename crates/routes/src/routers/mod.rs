pub mod auth;
pub mod config;
// pub: the import handler shares this router's size caps.
pub mod db_transfer;
pub mod discover;
pub mod episodes;
pub mod extractors;
pub mod guards;
pub mod health;
pub mod middleware;
pub mod opml;
pub mod playbacks;
pub mod playlists;
pub mod podcast_configs;
pub mod podcasts;
pub mod polling;
pub mod restart;
pub mod server_errors;
pub mod server_logs;
pub mod users;
pub mod ws;

pub mod errors;
mod media_auth;
pub mod media_path;

pub use errors::ApiError;
#[cfg(test)]
pub use halogen_router::build_router;

pub mod sync;
