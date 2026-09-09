use super::*;

impl ApiClient {
    /// GET /admin/users — list users with pagination. **Admin only.**
    pub async fn list_users(
        &self,
        params: halogen_wire::DefaultListParams<halogen_wire::NoInclude>,
    ) -> Result<Page<Vec<UserData>>, ApiError> {
        let qs = serde_qs::to_string(&params).map_err(|e| ApiError::Decode(e.to_string()))?;
        let url = build_url(&self.base, "/admin/users", &qs);
        let response = self.authed_get(url).await?;
        self.handle_response(response).await
    }
}
