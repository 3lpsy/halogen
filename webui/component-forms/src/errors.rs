//! Normalize local validator errors and server ApiErrors into shared field messages. Keep FormErrors renderer-free for
//! use by form hooks and field components.

use std::collections::HashMap;

use halogen_apiclient::ApiError;
use halogen_wire::{SerializableValidationErrors, ValidationErrors};

/// Normalized form errors: field key -> human-readable messages.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormErrors(HashMap<String, Vec<String>>);

impl FormErrors {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Messages for one field (empty when none).
    pub fn for_field(&self, field: &str) -> Vec<String> {
        self.0.get(field).cloned().unwrap_or_default()
    }

    /// The standard inline merge every form renders under an input: this (local `validator`) field error, only when
    /// `show_local` (the field is touched or the form was submitted, so an untouched-empty field isn't nagged),
    /// followed by any `server` field error. Call on the local error set, passing the last-submit server errors
    /// (cleared on edit, so they never linger past a fix).
    pub fn field_messages(
        &self,
        field: &str,
        show_local: bool,
        server: Option<&FormErrors>,
    ) -> Vec<String> {
        let mut v = Vec::new();
        if show_local {
            v.extend(self.for_field(field));
        }
        if let Some(s) = server {
            v.extend(s.for_field(field));
        }
        v
    }

    /// Every message whose key is NOT one of `known_fields` — the catch-all bucket
    /// (database/data/internal/not-found/transport/unknown), flattened in a stable
    /// key-sorted order so rendering is deterministic.
    pub fn catch_all(&self, known_fields: &[&str]) -> Vec<String> {
        let mut keys: Vec<&String> = self
            .0
            .keys()
            .filter(|k| !known_fields.contains(&k.as_str()))
            .collect();
        keys.sort();
        keys.into_iter().flat_map(|k| self.0[k].clone()).collect()
    }

    /// Local `validator` errors — built each render from the live form values to
    /// gate the submit button and show touched-field messages. Falls back to the
    /// rule's `code` when it carries no custom message.
    pub fn from_validation(errs: &ValidationErrors) -> Self {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for (field, items) in errs.field_errors() {
            let msgs = items
                .iter()
                .map(|e| {
                    e.message
                        .as_ref()
                        .map(|m| m.to_string())
                        .unwrap_or_else(|| e.code.to_string())
                })
                .collect();
            map.insert(field.to_string(), msgs);
        }
        Self(map)
    }

    /// Server validation envelope (`field -> [{code, message}]`).
    pub fn from_server(errs: &SerializableValidationErrors) -> Self {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for (field, items) in &errs.errors {
            let msgs = items
                .iter()
                .map(|e| e.message.clone().unwrap_or_else(|| e.code.clone()))
                .collect();
            map.insert(field.clone(), msgs);
        }
        Self(map)
    }

    /// Any [`ApiError`]. `Validation` keeps the server's field keys; every other
    /// variant becomes a single catch-all message under the `error` key.
    pub fn from_api_error(err: &ApiError) -> Self {
        match err {
            ApiError::Validation(errs) => Self::from_server(errs),
            ApiError::Server { status, message } => {
                Self::catch(format!("{message} (status {status})"))
            }
            ApiError::Transport(_) => Self::catch(
                "Couldn't reach the server. Check your connection and try again.".into(),
            ),
            ApiError::Decode(_) => {
                Self::catch("The server returned an unexpected response.".into())
            }
            ApiError::Empty => Self::catch("The server returned no data.".into()),
        }
    }

    fn catch(message: String) -> Self {
        let mut map = HashMap::new();
        map.insert("error".to_string(), vec![message]);
        Self(map)
    }

    /// Add a client-side message under `field` that isn't a `validator` rule (e.g.
    /// "must be a whole number" for a numeric input that didn't parse). Creates the
    /// field entry if absent. Lets a form merge parse errors into the same surface as
    /// `validator`/server errors.
    pub fn push_field(&mut self, field: &str, message: String) {
        self.0.entry(field.to_string()).or_default().push(message);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use halogen_apiclient::ApiError;
    use halogen_wire::{
        PlaylistStoreData, SerializableValidationErrors, Validate, ValidationErrorField,
    };

    use super::FormErrors;

    #[test]
    fn from_validation_maps_field_messages() {
        let err = PlaylistStoreData {
            name: String::new(),
            description: None,
            is_default: None,
            ..Default::default()
        }
        .validate()
        .unwrap_err();
        let fe = FormErrors::from_validation(&err);
        assert_eq!(fe.for_field("name"), vec!["Playlist name is required"]);
        assert!(fe.for_field("description").is_empty());
        // No catch-all when every key is a known field.
        assert!(fe.catch_all(&["name", "description"]).is_empty());
    }

    #[test]
    fn server_errors_split_known_fields_from_catch_all() {
        let mut map = HashMap::new();
        map.insert(
            "name".to_string(),
            vec![ValidationErrorField {
                code: "length".into(),
                message: Some("too short".into()),
            }],
        );
        map.insert(
            "database".to_string(),
            vec![ValidationErrorField {
                code: "panic".into(),
                message: Some("Database error".into()),
            }],
        );
        let fe = FormErrors::from_server(&SerializableValidationErrors { errors: map });
        assert_eq!(fe.for_field("name"), vec!["too short"]);
        assert_eq!(
            fe.catch_all(&["name", "description"]),
            vec!["Database error"]
        );
    }

    #[test]
    fn message_falls_back_to_code_when_absent() {
        let mut map = HashMap::new();
        map.insert(
            "name".to_string(),
            vec![ValidationErrorField {
                code: "required".into(),
                message: None,
            }],
        );
        let fe = FormErrors::from_server(&SerializableValidationErrors { errors: map });
        assert_eq!(fe.for_field("name"), vec!["required"]);
    }

    #[test]
    fn non_validation_api_error_is_catch_all() {
        let fe = FormErrors::from_api_error(&ApiError::Server {
            status: 500,
            message: "boom".into(),
        });
        assert!(fe.for_field("name").is_empty());
        assert_eq!(fe.catch_all(&["name"]).len(), 1);
    }
}
