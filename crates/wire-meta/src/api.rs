//! Request and response envelopes shared by native dispatch and HTTP adapters.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiRequest {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub body: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiResponse {
    pub status: u16,
    pub body: Vec<u8>,
}
