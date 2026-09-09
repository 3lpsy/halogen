//! Audio download response types returned by the streaming download methods.

use std::pin::Pin;

use futures_util::Stream;

use crate::error::ApiError;

pub struct AudioChunk {
    pub bytes: Vec<u8>,
    pub total: Option<u64>,
    /// Byte offset the body actually starts at — the `Content-Range` start on a
    /// `206`, or `0` when the server ignored the range and answered `200` with the
    /// whole file. Lets the chunked download verify the server honored the
    /// requested offset instead of silently writing bytes at the wrong position.
    pub served_from: u64,
    pub content_type: Option<String>,
}

/// A streamed audio download for the "no chunking" mode: header metadata plus the
/// body as a stream of byte pieces. The whole (often 100+ MB) file is never buffered
/// — the caller writes each piece to storage and drops it. Built by
/// [`ApiClient::download_audio_stream`].
pub struct AudioStream {
    /// Full file size from `Content-Range`/`Content-Length`, when advertised.
    pub total: Option<u64>,
    /// Byte offset the body actually starts at — the `Content-Range` start on a
    /// `206`, or `0` when the server ignored the range and answered `200` with the
    /// whole file. Lets a resuming caller tell an honored offset from a restart.
    pub served_from: u64,
    pub content_type: Option<String>,
    /// The response body as `Result<bytes, ApiError>` pieces, in order.
    pub body: Pin<Box<dyn Stream<Item = Result<Vec<u8>, ApiError>>>>,
}

/// A raw authed media response for the native webview media proxy ([`ApiClient::fetch_media_raw`]): the status
/// plus the few headers the webview needs, body fully buffered. Non-2xx statuses are RETURNED, not errors, so
/// the proxy forwards them verbatim (e.g. a 404 for missing artwork drives the UI placeholder exactly like on
/// web).
#[cfg(not(target_arch = "wasm32"))]
pub struct RawMediaResponse {
    pub status: u16,
    pub content_type: Option<String>,
    /// `Content-Range` on a 206 — forwarded so the webview's media stack can
    /// keep issuing range requests.
    pub content_range: Option<String>,
    pub cache_control: Option<String>,
    pub etag: Option<String>,
    pub bytes: Vec<u8>,
}
