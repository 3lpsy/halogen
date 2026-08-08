pub mod cookie;
pub mod login;
pub mod logout;
pub mod password;
pub mod refresh;
#[cfg(test)]
pub mod tests;
pub mod token;

pub use halogen_wire::TokenData;
pub use login::login;
pub use logout::logout;
pub use password::change_password;
pub use refresh::{RefreshRequestData, refresh};
// The auth layer's config (JWT secret + expiry) lives in `middleware` next to the
// layer that consumes it; re-exported here so handlers read it as `auth::AuthConfig`.
pub use crate::routers::middleware::AuthConfig;
