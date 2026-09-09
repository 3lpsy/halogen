use super::*;

impl ApiClient {
    /// GET /episodes — list episodes with pagination.
    pub async fn list_episodes(
        &self,
        params: halogen_wire::DefaultListParams<EpisodeInclude>,
    ) -> Result<Page<Vec<EpisodeData>>, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(&self.base, "/episodes", &qs);
        let response = self.authed_get(url).await?;
        self.handle_response(response).await
    }

    /// GET /episodes/{id}. `includes` selects eager-loaded relations (e.g.
    /// `Playback` to embed the caller's resume cursor); empty = no includes.
    pub async fn get_episode(
        &self,
        id: i32,
        includes: &[EpisodeInclude],
    ) -> Result<EpisodeData, ApiError> {
        let path = format!("/episodes/{}", id);
        let qs = if includes.is_empty() {
            String::new()
        } else {
            let params = halogen_wire::EpisodeShowParams {
                id: None,
                podcast_id: None,
                includes: Some(includes.to_vec()),
            };
            serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?
        };
        let url = build_url(&self.base, &path, &qs);
        let response = self.authed_get(url).await?;
        let resp: Page<EpisodeData> = self.handle_response(response).await?;
        Ok(resp.data)
    }

    /// Fetch stored episode audio and Content-Type with bearer authentication, buffering the whole file in memory for
    /// device storage. Browser audio instead uses the endpoint's `auth_media` cookie support.
    pub async fn download_audio(&self, id: i32) -> Result<(Vec<u8>, Option<String>), ApiError> {
        let url = format!("{}/episodes/{}/audio", self.base, id);
        let response = self.authed_get(url).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(ApiError::Server {
                status: status.as_u16(),
                message: format!("audio fetch failed ({status})"),
            });
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let bytes = response.bytes().await?;
        Ok((bytes.to_vec(), content_type))
    }

    /// Fetch an inclusive audio range for resumable downloads. Parse total size from Content-Range on 206 or
    /// Content-Length on 200; `served_from` lets callers restart if a server ignored Range rather than write bytes at
    /// the wrong offset.
    pub async fn download_audio_range(
        &self,
        id: i32,
        start: u64,
        end: u64,
    ) -> Result<AudioChunk, ApiError> {
        let url = format!("{}/episodes/{}/audio", self.base, id);
        let mut builder = self.http.get(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        let response = builder
            .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(ApiError::Server {
                status: status.as_u16(),
                message: format!("audio chunk fetch failed ({status})"),
            });
        }
        let headers = response.headers();
        let content_type = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let content_range = headers
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok());
        // `Content-Range` start (206), else 0 (a 200 served the whole file from 0).
        let served_from = content_range
            .and_then(parse_content_range_start)
            .unwrap_or(0);
        // Total size from `Content-Range: bytes <s>-<e>/<total>` (206), else from
        // `Content-Length` (a 200 that ignored the Range = the whole file).
        let total = content_range
            .and_then(parse_content_range_total)
            .or_else(|| {
                headers
                    .get(reqwest::header::CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
            });
        // Mis-offset response (a proxy ignored the `Range` and answered from byte 0): return WITHOUT consuming
        // the body. The caller discards this chunk on the `served_from` check anyway — pre-fix, `bytes()` first
        // buffered the entire (often 100+ MB) file per in-flight chunk, × parallelism, just to throw it away.
        if served_from != start {
            return Ok(AudioChunk {
                bytes: Vec::new(),
                total,
                served_from,
                content_type,
            });
        }
        // In-range response: consume incrementally, CAPPED at the requested span. A 200 that served the whole
        // file from byte 0 (server ignored the range END) passes the offset check when `start == 0` — reading
        // it whole would buffer the entire file; capping keeps this a chunk. Fewer/truncated bytes are fine:
        // callers treat short chunks as ordinary partial progress and re-request the remainder.
        let requested = (end.saturating_sub(start) as usize).saturating_add(1);
        let mut body = response.bytes_stream();
        let mut bytes: Vec<u8> = Vec::with_capacity(requested.min(16 * 1024 * 1024));
        while bytes.len() < requested {
            match body.next().await {
                Some(piece) => {
                    let piece = piece?;
                    let take = piece.len().min(requested - bytes.len());
                    bytes.extend_from_slice(&piece[..take]);
                    if take < piece.len() {
                        break; // requested span filled mid-piece; drop the rest
                    }
                }
                None => break,
            }
        }
        Ok(AudioChunk {
            bytes,
            total,
            served_from,
            content_type,
        })
    }

    /// Stream audio without buffering the whole file, using `Range: bytes={start}-` to resume. Returned headers include
    /// total size, content type, and `served_from`; callers must restart when the server ignores Range and serves byte
    /// zero.
    pub async fn download_audio_stream(
        &self,
        id: i32,
        start: u64,
    ) -> Result<AudioStream, ApiError> {
        let url = format!("{}/episodes/{}/audio", self.base, id);
        let mut builder = self.http.get(&url);
        if let Some(token) = self.token.read().unwrap().as_ref() {
            builder = builder.bearer_auth(token);
        }
        let response = builder
            .header(reqwest::header::RANGE, format!("bytes={start}-"))
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(ApiError::Server {
                status: status.as_u16(),
                message: format!("audio stream fetch failed ({status})"),
            });
        }
        let headers = response.headers();
        let content_type = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let content_range = headers
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok());
        // `Content-Range` start (206), else 0 (a 200 served the whole file from 0).
        let served_from = content_range
            .and_then(parse_content_range_start)
            .unwrap_or(0);
        // Full size from `Content-Range` (206) — NOT `Content-Length`, which on a 206
        // is only the partial length. `Content-Length` is the fallback for a 200.
        let total = content_range
            .and_then(parse_content_range_total)
            .or_else(|| {
                headers
                    .get(reqwest::header::CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
            });
        let body = response
            .bytes_stream()
            .map(|item| item.map(|b| b.to_vec()).map_err(ApiError::from));
        Ok(AudioStream {
            total,
            served_from,
            content_type,
            body: Box::pin(body),
        })
    }
}
