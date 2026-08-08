//! Small shared helpers for the `handle()` functions in each resource module.

use halogen_utils::constants::{VALIDATION_EXISTS_CODE, VALIDATION_ID_FIELD};
use halogen_utils::verrors;
use halogen_wire::{DbValidationErrors, ValidationErrors};

/// Map a SeaORM error to the standard validation-error envelope, logging the
/// real cause (with `context`) server-side. Use as
/// `.map_err(db_error("fetching podcast"))?`.
///
/// The mapping itself is delegated to [`DbValidationErrors`] — the single source
/// of truth in `halogen-wire`: a UNIQUE violation becomes a friendly
/// per-field 409, everything else a generic 500. The raw driver message is
/// logged but NEVER sent to the client.
pub fn db_error(context: &'static str) -> impl Fn(sea_orm::DbErr) -> ValidationErrors {
    move |e| {
        tracing::warn!("Database error {context}: {e}");
        DbValidationErrors::from(e).into()
    }
}

/// A 404 "<thing> not found" validation error, keyed `id`/`exists` — the same
/// shape the guards and `EntityHelpers::by_id_or_err` produce. Collapses the
/// `.ok_or_else(|| verrors(VALIDATION_ID_FIELD, VALIDATION_EXISTS_CODE,
/// msg.to_string()))` that the read/update/delete handlers repeat, while keeping
/// each handler's specific message (which several tests assert on verbatim).
pub fn not_found(message: impl Into<String>) -> ValidationErrors {
    verrors(VALIDATION_ID_FIELD, VALIDATION_EXISTS_CODE, message.into())
}

/// True when the requested `includes` list contains `variant`. Collapses the
/// `includes.as_ref().map(|i| i.contains(&X)).unwrap_or(false)` boilerplate the
/// read handlers repeat for each optional relation.
pub fn wants<T: PartialEq>(includes: Option<&Vec<T>>, variant: T) -> bool {
    includes.is_some_and(|i| i.contains(&variant))
}
