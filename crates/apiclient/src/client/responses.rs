use super::*;

impl ApiClient {
    pub(super) async fn handle_response<Res: DeserializeOwned + ResponsableData>(
        &self,
        response: reqwest::Response,
    ) -> Result<Page<Res>, ApiError> {
        let status = response.status();
        let bytes = response.bytes().await?;

        if !status.is_success() {
            if let Ok(api_err) = parse_error_body(status.as_u16(), &bytes) {
                return Err(api_err);
            }
            return Err(ApiError::Server {
                status: status.as_u16(),
                message: String::from_utf8_lossy(&bytes).to_string(),
            });
        }

        let resp: halogen_wire::ResponseData<Res> =
            serde_json::from_slice(&bytes).map_err(|e| ApiError::Decode(e.to_string()))?;

        let data = resp.data.ok_or(ApiError::Empty)?;
        Ok(Page {
            data,
            paginator: resp.paginator,
        })
    }

    pub(super) async fn handle_single_response<Res: DeserializeOwned + ResponsableData>(
        &self,
        response: reqwest::Response,
    ) -> Result<Res, ApiError> {
        let status = response.status();
        let bytes = response.bytes().await?;

        if !status.is_success() {
            if let Ok(api_err) = parse_error_body(status.as_u16(), &bytes) {
                return Err(api_err);
            }
            return Err(ApiError::Server {
                status: status.as_u16(),
                message: String::from_utf8_lossy(&bytes).to_string(),
            });
        }

        let resp: halogen_wire::ResponseData<Res> =
            serde_json::from_slice(&bytes).map_err(|e| ApiError::Decode(e.to_string()))?;

        resp.data.ok_or(ApiError::Empty)
    }

    pub(super) async fn handle_empty_response(
        &self,
        response: reqwest::Response,
    ) -> Result<(), ApiError> {
        let status = response.status();
        let bytes = response.bytes().await?;

        if !status.is_success() {
            if let Ok(api_err) = parse_error_body(status.as_u16(), &bytes) {
                return Err(api_err);
            }
            return Err(ApiError::Server {
                status: status.as_u16(),
                message: String::from_utf8_lossy(&bytes).to_string(),
            });
        }

        let _: halogen_wire::ResponseData<()> =
            serde_json::from_slice(&bytes).map_err(|e| ApiError::Decode(e.to_string()))?;
        Ok(())
    }
}
