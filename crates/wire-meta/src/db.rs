#[cfg(feature = "db")]
use halogen_utils::constants::{
    VALIDATION_DATABASE_FIELD, VALIDATION_PANIC_CODE, VALIDATION_REQUEST_FIELD,
    VALIDATION_UNIQUE_CODE,
};
use sea_orm::{DbErr, TransactionError};
use std::borrow::Cow;
use validator::{ValidationError, ValidationErrors};

use crate::field_ref;

pub struct DbValidationErrors(DbErr);
impl From<DbErr> for DbValidationErrors {
    fn from(err: DbErr) -> Self {
        DbValidationErrors(err)
    }
}
impl From<TransactionError<DbErr>> for DbValidationErrors {
    fn from(err: TransactionError<DbErr>) -> Self {
        match err {
            TransactionError::Connection(e) => DbValidationErrors(e),
            TransactionError::Transaction(e) => DbValidationErrors(e),
        }
    }
}
impl From<DbValidationErrors> for ValidationErrors {
    /// The single `DbErr -> ValidationErrors` mapping shared across the codebase. A UNIQUE violation parses the
    /// offending column into a per-field `unique` error (`field` = the column, → 409). Every other `DbErr`
    /// becomes a generic 500 keyed `database`/`panic`; the raw driver message is `warn!`-logged once here — the
    /// single mapping site every path converges on — and NEVER put in the response.
    fn from(wrapper: DbValidationErrors) -> Self {
        let db_msg = wrapper.0.to_string();

        if db_msg.contains("UNIQUE constraint failed:") {
            let field = db_msg
                .split("UNIQUE constraint failed:")
                .nth(1)
                .and_then(|s| s.trim().split('.').nth(1))
                .map(|s| s.to_string())
                .unwrap_or_else(|| VALIDATION_REQUEST_FIELD.to_string());

            let mut e = ValidationErrors::new();
            e.add(
                field_ref(&field),
                ValidationError::new(VALIDATION_UNIQUE_CODE)
                    .with_message(Cow::from(format!("Field {field} must be unique"))),
            );
            return e;
        }

        // Genuinely unexpected DB error. This is the single place every mapping
        // path (helpers, `common.rs`, pagination, server `db_error`) converges, so
        // log the raw cause here ONCE — the response stays generic and leak-free:
        // field `database` (location: inside the DB flow), code `panic` (→ 500).
        tracing::warn!("unexpected database error: {}", wrapper.0);
        let mut errs = ValidationErrors::new();
        errs.add(
            VALIDATION_DATABASE_FIELD,
            ValidationError::new(VALIDATION_PANIC_CODE).with_message(Cow::from("Database error")),
        );
        errs
    }
}
