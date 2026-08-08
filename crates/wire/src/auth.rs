//! Session + status DTOs shared by the client and server: login credentials, the
//! issued token, and the liveness/status probe payload.

use serde::{Deserialize, Serialize};
use validator::Validate;

use crate::ResponsableData;
use typeshare::typeshare;

#[typeshare]
/// Credentials for the login endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct LoginData {
    #[validate(length(
        min = 1,
        max = 64,
        message = "Username must be between 1 and 64 characters"
    ))]
    pub username: String,
    #[validate(length(
        min = 1,
        max = 256,
        message = "Password must be between 1 and 256 characters"
    ))]
    pub password: String,
}

#[typeshare]
/// Token returned from the login / refresh endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenData {
    pub token: String,
}

impl ResponsableData for TokenData {}

#[typeshare]
/// Liveness payload for the health/status probe — `running: true` means the
/// server is up. Shared so the client's bootstrap probe and the server's
/// `/healthz` response are the same contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusData {
    pub running: bool,
}

impl ResponsableData for StatusData {}

#[typeshare]
/// Server build version for the public `GET /api/v1/version` probe — the
/// workspace `Cargo.toml` version baked in at compile time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionData {
    pub version: String,
}

impl ResponsableData for VersionData {}
