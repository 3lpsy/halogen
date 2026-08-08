use chrono::{DateTime, Utc};

use serde::{Deserialize, Serialize};
use typeshare::typeshare;
use validator::Validate;

use super::{
    HasOrder, HasPagination, Order, Pagination, RequestData, RequestableParams, ResponsableData,
    ResponseData,
};

// Responses
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserData {
    pub id: i32,
    pub username: String,
    pub is_admin: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// impls

impl From<UserData> for ResponseData<UserData> {
    fn from(data: UserData) -> Self {
        ResponseData {
            data: Some(data),
            errors: None,
            paginator: None,
        }
    }
}

impl ResponsableData for UserData {}

// Requests
#[derive(Default, Clone, Debug, Validate, Serialize, Deserialize)]
pub struct UserShowParams {
    #[validate(range(min = 1, message = "ID must be a valid integer"))]
    pub id: Option<i32>,
    #[validate(length(max = 64, message = "Username must be at most 64 characters long"))]
    pub username: Option<String>,
}

#[derive(Default, Debug, Validate, Serialize, Deserialize)]
pub struct UserDeleteParams {
    #[validate(range(min = 1, message = "ID must be a valid integer"))]
    pub id: i32,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Default, Validate)]
pub struct UserStoreData {
    #[validate(length(
        min = 3,
        max = 64,
        message = "Username must be between 3 and 64 characters long"
    ))]
    pub username: String,
    #[validate(length(
        min = 8,
        max = 256,
        message = "Password must be between 8 and 256 characters long"
    ))]
    pub password: String,
    #[validate(length(
        min = 8,
        max = 256,
        message = "Password confirmation must be between 8 and 256 characters long"
    ))]
    #[validate(must_match(
        other = "password",
        message = "Password confirmation must match the password"
    ))]
    pub password_confirm: String,
    pub is_admin: Option<bool>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, Default, Validate)]
pub struct PasswordUpdateData {
    #[validate(length(
        min = 8,
        max = 256,
        message = "Password must be between 8 and 256 characters long"
    ))]
    pub password: String,
    #[validate(length(
        min = 8,
        max = 256,
        message = "Password confirmation must be between 8 and 256 characters long"
    ))]
    #[validate(must_match(
        other = "password",
        message = "Password confirmation must match the password"
    ))]
    pub password_confirm: String,
}

#[typeshare]
/// Request body for `POST /auth/password` — change the authenticated user's own
/// password. `current_password` is re-verified against the stored hash by the
/// handler; `new_password` reuses [`PasswordUpdateData`], so the 8-char minimum
/// and the confirmation-match check are enforced by validation.
#[derive(Debug, Validate, Serialize, Deserialize)]
pub struct PasswordChangeData {
    #[validate(length(min = 1, max = 256, message = "Current password is required"))]
    pub current_password: String,
    #[validate(nested)]
    pub new_password: PasswordUpdateData,
}

#[typeshare]
#[derive(Debug, Validate, Serialize, Deserialize, Default, Clone)]
pub struct UserUpdateData {
    #[validate(length(
        min = 3,
        max = 64,
        message = "Username must be between 3 and 64 characters long"
    ))]
    pub username: Option<String>,
    pub is_admin: Option<bool>,
}

impl<P: RequestableParams> From<UserStoreData> for RequestData<UserStoreData, P> {
    fn from(data: UserStoreData) -> Self {
        RequestData::from_data(data)
    }
}

#[derive(Default, Clone, Debug, Validate, Serialize, Deserialize)]
pub struct UserListParams {
    #[validate(nested)]
    pub pagination: Option<Pagination>,
    #[validate(nested)]
    pub order: Option<Order>,
}

impl HasPagination for UserListParams {
    fn pagination(&mut self) -> &mut Option<Pagination> {
        &mut self.pagination
    }
}

impl HasOrder for UserListParams {
    fn order(&mut self) -> &mut Option<Order> {
        &mut self.order
    }
}
