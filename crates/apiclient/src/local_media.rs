use crate::{ApiError, LocalTransport, RawMediaResponse};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

const CHUNK: u64 = 8 * 1024 * 1024;
fn error(message: impl ToString) -> ApiError {
    ApiError::Server {
        status: 500,
        message: message.to_string(),
    }
}
fn response(status: u16, bytes: Vec<u8>, content_range: Option<String>) -> RawMediaResponse {
    RawMediaResponse {
        status,
        bytes,
        content_range,
        content_type: None,
        cache_control: Some("private, max-age=60".into()),
        etag: None,
    }
}

/// Read a bounded piece of a profile-authorized local media file for the webview bridge.
pub async fn fetch(
    transport: &dyn LocalTransport,
    route: &str,
    range: Option<&str>,
) -> Result<RawMediaResponse, ApiError> {
    let Some(path) = transport.media_path(route).await.map_err(error)? else {
        return Ok(response(
            if route.contains("/art") { 204 } else { 404 },
            Vec::new(),
            None,
        ));
    };
    let mut file = tokio::fs::File::open(&path).await.map_err(error)?;
    let size = file.metadata().await.map_err(error)?.len();
    let (start, end) = if let Some(range) = range {
        let parsed = range
            .strip_prefix("bytes=")
            .and_then(|s| s.split_once('-'))
            .and_then(|(start, end)| {
                Some((
                    start.parse::<u64>().ok()?,
                    if end.is_empty() {
                        None
                    } else {
                        Some(end.parse::<u64>().ok()?)
                    },
                ))
            });
        let Some((start, end)) = parsed else {
            return Ok(response(416, Vec::new(), Some(format!("bytes */{size}"))));
        };
        let end = end
            .unwrap_or(size.saturating_sub(1))
            .min(size.saturating_sub(1));
        if start >= size || end < start {
            return Ok(response(416, Vec::new(), Some(format!("bytes */{size}"))));
        }
        (start, end.min(start.saturating_add(CHUNK - 1)))
    } else {
        if size > CHUNK {
            return Err(error(
                "large local media requires streaming or a byte range",
            ));
        }
        (0, size.saturating_sub(1))
    };
    let length = if size == 0 { 0 } else { end - start + 1 };
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(error)?;
    let mut bytes = vec![0; length as usize];
    file.read_exact(&mut bytes).await.map_err(error)?;
    let partial = range.is_some() || length < size;
    let mut result = response(
        if partial { 206 } else { 200 },
        bytes,
        partial.then(|| format!("bytes {start}-{end}/{size}")),
    );
    result.content_type = Some(
        match std::path::Path::new(&path)
            .extension()
            .and_then(|s| s.to_str())
        {
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("png") => "image/png",
            Some("webp") => "image/webp",
            Some("mp3") => "audio/mpeg",
            Some("m4a" | "mp4") => "audio/mp4",
            Some("ogg" | "opus") => "audio/ogg",
            _ => "application/octet-stream",
        }
        .into(),
    );
    Ok(result)
}

#[cfg(test)]
#[path = "local_media_tests.rs"]
mod tests;
