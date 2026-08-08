//! Response side: `ResponseData<T>` envelope (data + errors + paginator) and the
//! `ResponsableData` marker / serializable validation-error shapes.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use validator::ValidationErrors;

use super::pagination::Paginator;
use typeshare::typeshare;

/// Serializable form of `validator::ValidationErrors` for API responses.
/// Flattens nested struct/list errors to dotted keys (`field.inner`, `field.idx`).
/// The `#[typeshare]` shape is the FLATTENED serialization (a bare map), not the
/// struct — serde's `flatten` erases the wrapper on the wire.
#[typeshare(serialized_as = "HashMap<String, Vec<ValidationErrorField>>")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableValidationErrors {
    #[serde(flatten)]
    pub errors: HashMap<String, Vec<ValidationErrorField>>,
}

#[typeshare]
/// One validation error: its code plus optional human message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationErrorField {
    pub code: String,
    pub message: Option<String>,
}

impl From<ValidationErrors> for SerializableValidationErrors {
    fn from(errors: ValidationErrors) -> Self {
        let mut map = HashMap::new();
        for (field, fields) in errors.into_errors() {
            let field_errors = match fields {
                validator::ValidationErrorsKind::Field(e) => e
                    .into_iter()
                    .map(|e| ValidationErrorField {
                        code: e.code.to_string(),
                        message: e.message.map(|s| s.to_string()),
                    })
                    .collect(),
                validator::ValidationErrorsKind::Struct(e) => {
                    let nested = SerializableValidationErrors::from(*e);
                    for (k, v) in nested.errors {
                        map.entry(format!("{}.{}", field, k))
                            .or_insert_with(Vec::new)
                            .extend(v);
                    }
                    continue;
                }
                validator::ValidationErrorsKind::List(list) => {
                    for (idx, nested) in list.into_iter() {
                        let idx_str = format!("{}.{}", field, idx);
                        let nested_errors: Vec<_> = SerializableValidationErrors::from(*nested)
                            .errors
                            .into_values()
                            .flatten()
                            .map(|e| ValidationErrorField {
                                code: e.code,
                                message: e.message,
                            })
                            .collect();
                        map.entry(idx_str)
                            .or_insert_with(Vec::new)
                            .extend(nested_errors);
                    }
                    continue;
                }
            };
            map.insert(field.to_string(), field_errors);
        }
        Self { errors: map }
    }
}

/// Marker for types allowed inside `ResponseData<T>`. Opt-in (not blanket) so only
/// intended response bodies qualify.
pub trait ResponsableData: Serialize + for<'de> Deserialize<'de> {}

#[typeshare]
/// A successful paginated list response: the data payload plus optional
/// pagination metadata. The client-facing projection of [`ResponseData`] for list
/// endpoints (no `errors` — a failure surfaces as a typed error instead).
#[derive(Debug, Serialize, Deserialize)]
pub struct Page<T> {
    /// The data payload.
    pub data: T,
    /// Pagination metadata for list endpoints.
    pub paginator: Option<Paginator>,
}

impl<T: ResponsableData> ResponsableData for Page<T> {}

/// A list of responsable items is itself responsable — so list endpoints don't
/// each hand-write `impl ResponsableData for Vec<XData>` (which would also be an
/// orphan-rule violation now that the trait lives in this crate, not `wire`).
impl<T: ResponsableData> ResponsableData for Vec<T> {}

#[typeshare]
/// Response envelope: data, validation errors, and optional pagination.
///
/// # Type Parameters
/// - `T`: The type of the response data (typically a `*Data` struct)
///
/// # Usage Examples
///
/// ## Successful Response
/// ```rust
/// use halogen_wire_meta::response::{ResponsableData, ResponseData};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Serialize, Deserialize)]
/// struct PodcastData {
///     id: i32,
///     title: String,
/// }
/// impl ResponsableData for PodcastData {}
///
/// let podcast_data = PodcastData {
///     id: 1,
///     title: "My Podcast".to_string(),
/// };
///
/// let _response = ResponseData::from_data(podcast_data);
/// ```
///
/// ## Error Response
/// ```rust
/// use halogen_wire_meta::response::ResponseData;
///
/// // Create validation errors (this is simplified)
/// // let errors: ValidationErrors = todo!("validation errors");
/// // let response: ResponseData<PodcastData> = errors.into();
/// ```
#[derive(Debug, Serialize, Deserialize)]
#[serde(bound = "T: ResponsableData")]
pub struct ResponseData<T>
where
    T: ResponsableData,
{
    /// The actual data being returned in the response.
    pub data: Option<T>,
    /// Validation errors if the request failed validation.
    pub errors: Option<SerializableValidationErrors>,
    /// Pagination information for list responses.
    pub paginator: Option<Paginator>,
}

impl<T> ResponseData<T>
where
    T: ResponsableData,
{
    /// Create a new ResponseData with all fields specified.
    pub fn new(
        data: Option<T>,
        errors: Option<ValidationErrors>,
        paginator: Option<Paginator>,
    ) -> ResponseData<T> {
        Self {
            data,
            errors: errors.map(SerializableValidationErrors::from),
            paginator,
        }
    }

    /// Create a ResponseData from just the data (no errors or pagination).
    pub fn from_data(data: T) -> ResponseData<T> {
        ResponseData::new(Some(data), None, None)
    }

    /// Create a ResponseData with pagination information.
    pub fn from_paginator(data: T, paginator: Paginator) -> ResponseData<T> {
        ResponseData::new(Some(data), None, Some(paginator))
    }
}

impl ResponsableData for () {}

/// Convert validation errors into a response.
///
/// This implementation allows easy conversion of `ValidationErrors` directly
/// into `ResponseData` for error responses.
impl<T: ResponsableData> From<ValidationErrors> for ResponseData<T> {
    fn from(errors: ValidationErrors) -> Self {
        ResponseData::new(None, Some(errors), None)
    }
}
