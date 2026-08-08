//! Axum extractors that eliminate boilerplate across handlers.
//!
//! - `Id` — parses `Path<String>` → `i32` with a single error shape
//! - `Body` — extracts the `data` field from `RequestData<T, P>`

use std::collections::HashMap;

use axum::{
    Json,
    body::Body as AxumBody,
    extract::{FromRequestParts, Path},
    http::request::Parts,
};
use halogen_utils::constants::{
    VALIDATION_DATA_FIELD, VALIDATION_INVALID_CODE, VALIDATION_PARSING_CODE,
    VALIDATION_REQUEST_FIELD,
};
use halogen_wire::RequestData;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_qs::axum::QsQuery;
use validator::Validate;

use crate::routers::errors::ApiError;

/// Extracts an `i32` ID from a path segment.
///
/// Replaces the repeated `Path<String>` → `i32::parse()` pattern found
/// in every get/delete/update handler.
///
/// # Example
/// ```text
/// .route("/users/{id}", route_get(user_get))
///
/// pub async fn get(Id(id): Id) -> Json<ResponseData<UserData>> { ... }
/// ```
#[derive(Debug, Clone)]
pub struct Id(pub i32);

impl<S> FromRequestParts<S> for Id
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Delegate to Path<String> which handles all the URL param extraction
        match Path::<String>::from_request_parts(parts, _state).await {
            Ok(Path(id_str)) => match id_str.parse::<i32>() {
                // Ids are 1-based DB primary keys. Reject `<= 0` here (and, via
                // `i32::parse`, anything above `i32::MAX`) so every id-bearing route
                // gets the same `min = 1, max = i32::MAX` guarantee for free.
                Ok(id) if id >= 1 => Ok(Id(id)),
                Ok(_) => Err(ApiError::invalid_id(format!(
                    "Invalid ID: must be a positive integer, got '{id_str}'"
                ))),
                Err(_) => Err(ApiError::unparsable_id(format!(
                    "Invalid ID format: expected integer, got '{id_str}'"
                ))),
            },
            Err(_rejection) => {
                // Path extraction failed — the id segment couldn't be read.
                Err(ApiError::invalid_id(
                    "Failed to extract path parameter".to_string(),
                ))
            }
        }
    }
}

/// The authenticated user's id (`i32`), taken from the JWT auth middleware.
///
/// The `JwtAuthLayer` validates the bearer token and inserts a [`JwtAuth`]
/// into the request extensions; this extractor reads it and parses the id.
/// Handlers that own per-user data (e.g. playbacks) use this instead of
/// trusting a `user_id` from the request body or query — that would let any
/// authenticated user act on another user's rows.
#[derive(Debug, Clone)]
pub struct AuthUserId(pub i32);

impl<S> FromRequestParts<S> for AuthUserId
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth = parts
            .extensions
            .get::<crate::routers::middleware::JwtAuth>()
            .ok_or_else(|| ApiError::unauthenticated("Missing authentication".to_string()))?;
        auth.user_id
            .parse::<i32>()
            .map(AuthUserId)
            .map_err(|_| ApiError::unauthenticated("Invalid user id in token".to_string()))
    }
}

/// The authenticated user's id, **required to be an admin**.
///
/// Reads the [`JwtAuth`] the middleware inserted: 401 if absent (no/invalid
/// token), 403 if the user isn't an admin. Used to gate operator-only control
/// endpoints (e.g. the polling control routes).
#[derive(Debug, Clone)]
pub struct AdminUser(pub i32);

impl<S> FromRequestParts<S> for AdminUser
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth = parts
            .extensions
            .get::<crate::routers::middleware::JwtAuth>()
            .ok_or_else(|| ApiError::unauthenticated("Missing authentication".to_string()))?;
        if !auth.is_admin {
            return Err(ApiError::unauthorized(
                "Admin privileges required".to_string(),
            ));
        }
        auth.user_id
            .parse::<i32>()
            .map(AdminUser)
            .map_err(|_| ApiError::unauthenticated("Invalid user id in token".to_string()))
    }
}

/// The authenticated actor: the user's id plus their admin flag, read from the
/// [`JwtAuth`] the middleware inserted (no extra DB hit — `is_admin` is already
/// populated). Routers that run ownership guards use this so the guard can apply
/// the admin bypass without a second lookup.
#[derive(Debug, Clone, Copy)]
pub struct Actor {
    pub id: i32,
    pub is_admin: bool,
}

impl<S> FromRequestParts<S> for Actor
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth = parts
            .extensions
            .get::<crate::routers::middleware::JwtAuth>()
            .ok_or_else(|| ApiError::unauthenticated("Missing authentication".to_string()))?;
        let id = auth
            .user_id
            .parse::<i32>()
            .map_err(|_| ApiError::unauthenticated("Invalid user id in token".to_string()))?;
        Ok(Actor {
            id,
            is_admin: auth.is_admin,
        })
    }
}

/// Extracts two ordered `i32` IDs from a two-segment nested path.
///
/// Used for routes like `/playlists/{id}/episodes/{episode_id}`. Replaces the
/// buggy pattern of using `Id` twice (which only consumes the first segment).
#[derive(Debug, Clone)]
pub struct Ids2(pub i32, pub i32);

impl<S> FromRequestParts<S> for Ids2
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        match Path::<(i32, i32)>::from_request_parts(parts, _state).await {
            // Both segments are 1-based primary keys (see `Id`): require `>= 1`.
            Ok(Path((a, b))) if a >= 1 && b >= 1 => Ok(Ids2(a, b)),
            Ok(_) => Err(ApiError::invalid_id(
                "Invalid path parameters: both IDs must be positive integers".to_string(),
            )),
            Err(_) => Err(ApiError::unparsable_id(
                "Invalid path parameters: expected two integers".to_string(),
            )),
        }
    }
}

/// Extracts and validates a query-string DTO.
///
/// The validating counterpart to [`Body`] for the read routes. Uses `serde_qs`
/// (not axum's `Query`) so bracketed array keys like `?includes[0]=podcast_config`
/// parse into a `Vec` — serde_urlencoded silently drops them. Validation runs
/// here, so a `Query<T>` route physically cannot forget the
/// `params.validate().map_err(ApiError)?` the list/get routers would otherwise repeat.
///
/// # Example
/// ```text
/// pub async fn list(Query(params): Query<DefaultListParams<PodcastInclude>>) -> ... { ... }
/// ```
#[derive(Debug, Clone)]
pub struct Query<T>(pub T);

impl<T, S> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let QsQuery(value) = QsQuery::<T>::from_request_parts(parts, state)
            .await
            .map_err(|_| {
                ApiError::new(
                    VALIDATION_REQUEST_FIELD,
                    VALIDATION_PARSING_CODE,
                    "Failed to parse query parameters".to_string(),
                )
            })?;
        value.validate().map_err(ApiError::from)?;
        Ok(Query(value))
    }
}

/// Extracts the `data` field from a `RequestData<T, P>` body.
///
/// Replaces the `.data.ok_or_else(|| ...)` pattern found in update/delete
/// handlers that receive a JSON body.
///
/// # Example
/// ```text
/// pub async fn update(Id(id): Id, Body(body): Body<EpisodeUpdateData>) -> Json<ResponseData<EpisodeData>> { ... }
/// ```
#[derive(Debug, Clone)]
pub struct Body<T>(pub T);

impl<T, S> axum::extract::FromRequest<S, AxumBody> for Body<T>
where
    T: DeserializeOwned + Serialize + Validate,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(
        req: axum::http::Request<AxumBody>,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let req_data: Json<RequestData<T, HashMap<String, String>>> =
            Json::from_request(req, _state).await.map_err(|_| {
                ApiError::new(
                    VALIDATION_DATA_FIELD,
                    VALIDATION_PARSING_CODE,
                    "Failed to parse request body".to_string(),
                )
            })?;

        let data = req_data.0.data.ok_or_else(|| {
            ApiError::new(
                VALIDATION_DATA_FIELD,
                VALIDATION_INVALID_CODE,
                "Request body is required".to_string(),
            )
        })?;

        // Type-enforced validation: every `Body<T>` endpoint validates its DTO
        // here, so a handler physically cannot forget to. The field-keyed error
        // envelope is identical to the handler-level `.validate()?` path, since
        // both go through `ApiError`. Media/art/download routes use `Id`, not
        // `Body`, so they are unaffected.
        data.validate().map_err(ApiError::from)?;

        Ok(Body(data))
    }
}

#[cfg(test)]
mod tests {
    use crate::routers::playbacks::tests::{build_test_router, generate_jwt_token, setup_test_db};
    use crate::tests::harness::{authed, authed_json, field_error, json_body};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    /// `Id`: a non-integer path segment fails the `i32::parse` and yields a
    /// 400 carrying the "Invalid ID format" message. We drive it through the
    /// real `GET /users/{id}` route with a valid JWT so the middleware passes
    /// and the `Id` extractor is the thing that rejects.
    #[tokio::test]
    async fn test_id_non_integer_path_is_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .oneshot(authed("GET", "/api/v1/users/not-an-int", &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        // The id-extractor rejection is keyed under "id" (the location), with a
        // `parsing` reason for an unparseable segment.
        let message = field_error(&json, "id");
        assert!(
            message.contains("Invalid ID format"),
            "expected 'Invalid ID format' message, got: {message}"
        );
    }

    /// `Ids2`: a non-integer in either segment of `/playlists/{id}/episodes/{episode_id}`
    /// fails `Path::<(i32, i32)>` and yields a 400 with the two-integers message.
    #[tokio::test]
    async fn test_ids2_non_integer_segment_is_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        // DELETE uses `Ids2`; the second segment is non-integer.
        let response = router
            .oneshot(authed(
                "DELETE",
                "/api/v1/playlists/1/episodes/not-an-int",
                &token,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        let message = field_error(&json, "id");
        assert!(
            message.contains("expected two integers"),
            "expected the two-integers message, got: {message}"
        );
    }

    /// `Body<T>`: a syntactically valid `RequestData` whose `data` is absent
    /// (`{}` body) is rejected by the extractor with the exact
    /// "Request body is required" message. Driven through `POST /podcasts`,
    /// which uses `Body<PodcastStoreData>`.
    #[tokio::test]
    async fn test_body_missing_data_is_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let body = serde_json::json!({});
        let response = router
            .oneshot(authed_json("POST", "/api/v1/podcasts", &token, &body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert_eq!(field_error(&json, "data"), "Request body is required");
    }

    /// `Body<T>`: an unparseable JSON body trips the earlier `Json::from_request`
    /// failure branch → "Failed to parse request body" (distinct from the
    /// missing-`data` message above).
    #[tokio::test]
    async fn test_body_unparseable_json_is_400() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/podcasts")
                    .header("Authorization", format!("Bearer {token}"))
                    .header("Content-Type", "application/json")
                    .body(Body::from("{ this is not json"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = json_body(response).await;
        assert_eq!(field_error(&json, "data"), "Failed to parse request body");
    }

    /// `AuthUserId`: a fully valid request reaches the handler, confirming the
    /// extractor read the id the middleware inserted (this is also the positive
    /// middleware control). `GET /playbacks` returns 200 with an empty list.
    #[tokio::test]
    async fn test_auth_user_id_valid_request_is_200() {
        let (_root, dbc, payload) = setup_test_db().await;
        let router = build_test_router(dbc);

        let user_id = payload.get("user_id").unwrap().as_str().unwrap();
        let token = generate_jwt_token(user_id);

        let response = router
            .oneshot(authed("GET", "/api/v1/playbacks", &token))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
