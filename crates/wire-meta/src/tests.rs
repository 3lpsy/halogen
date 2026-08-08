mod response_tests {
    use crate::response::*;
    use std::borrow::Cow;
    use validator::{ValidationError, ValidationErrors, ValidationErrorsKind};

    fn field_error(code: &'static str) -> ValidationErrors {
        let mut e = ValidationErrors::new();
        e.add(code, ValidationError::new(code));
        e
    }

    #[test]
    fn flattens_field_struct_and_list_errors() {
        // Top-level plain field error.
        let mut errors = ValidationErrors::new();
        errors.add(
            "username",
            ValidationError::new("length").with_message(Cow::from("too short")),
        );

        // Nested struct error: `password` -> inner field `confirm`.
        let mut inner = ValidationErrors::new();
        inner.add("confirm", ValidationError::new("must_match"));
        errors.errors_mut().insert(
            "password".into(),
            ValidationErrorsKind::Struct(Box::new(inner)),
        );

        // List error: `items[0]` and `items[2]` each have an inner field error.
        let mut list = std::collections::BTreeMap::new();
        list.insert(0usize, Box::new(field_error("required")));
        list.insert(2usize, Box::new(field_error("range")));
        errors
            .errors_mut()
            .insert("items".into(), ValidationErrorsKind::List(list));

        let serializable = SerializableValidationErrors::from(errors);
        let map = &serializable.errors;

        // Plain field retained under its own key.
        assert_eq!(map["username"].len(), 1);
        assert_eq!(map["username"][0].code, "length");
        assert_eq!(map["username"][0].message.as_deref(), Some("too short"));

        // Struct flattened to `field.inner`.
        assert!(
            map.contains_key("password.confirm"),
            "keys: {:?}",
            map.keys()
        );
        assert_eq!(map["password.confirm"][0].code, "must_match");

        // List flattened to `field.idx`.
        assert!(map.contains_key("items.0"), "keys: {:?}", map.keys());
        assert!(map.contains_key("items.2"), "keys: {:?}", map.keys());
        assert_eq!(map["items.0"][0].code, "required");
        assert_eq!(map["items.2"][0].code, "range");
    }
}

#[cfg(feature = "db")]
mod db_tests {
    use crate::db::*;
    use halogen_utils::constants::{
        VALIDATION_DATABASE_FIELD, VALIDATION_PANIC_CODE, VALIDATION_REQUEST_FIELD,
    };
    use sea_orm::DbErr;
    use sea_orm::RuntimeErr;
    use validator::{ValidationErrors, ValidationErrorsKind};

    /// Build a synthetic `DbErr` whose `Display` carries the SQLite-style
    /// `UNIQUE constraint failed: <table>.<column>` message the converter parses.
    fn unique_violation(table_dot_column: &str) -> DbErr {
        DbErr::Query(RuntimeErr::Internal(format!(
            "UNIQUE constraint failed: {table_dot_column}"
        )))
    }

    fn field_codes(errs: &ValidationErrors, field: &str) -> Vec<String> {
        match errs.errors().get(field) {
            Some(ValidationErrorsKind::Field(v)) => v.iter().map(|e| e.code.to_string()).collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn unique_violation_extracts_column_as_field_ref() {
        // `feed_url` is not in the static FIELD_NAMES allow-list, so `field_ref`
        // maps it back to the generic "request" key — but the *message* still
        // names the extracted column.
        let errs: ValidationErrors =
            DbValidationErrors::from(unique_violation("podcast.feed_url")).into();
        assert_eq!(field_codes(&errs, VALIDATION_REQUEST_FIELD), vec!["unique"]);

        let ValidationErrorsKind::Field(v) = errs.errors().get(VALIDATION_REQUEST_FIELD).unwrap()
        else {
            unreachable!("expected Field variant");
        };
        assert_eq!(
            v[0].message.as_ref().map(|m| m.to_string()),
            Some("Field feed_url must be unique".to_string())
        );
    }

    #[test]
    fn unique_violation_known_field_keeps_its_name() {
        // `name` IS in FIELD_NAMES, so the error is keyed by the column itself.
        let errs: ValidationErrors =
            DbValidationErrors::from(unique_violation("config.name")).into();
        assert_eq!(field_codes(&errs, "name"), vec!["unique"]);
        assert!(errs.errors().get(VALIDATION_REQUEST_FIELD).is_none());
    }

    #[test]
    fn non_unique_error_is_generic_and_does_not_leak() {
        // Any other DbErr becomes a generic 500 — the raw driver text ("boom")
        // must NOT appear in the response envelope. Keyed `database`/`panic`:
        // location is "inside the DB flow", reason is "unexpected".
        let errs: ValidationErrors =
            DbValidationErrors::from(DbErr::Custom("boom".to_string())).into();
        assert_eq!(
            field_codes(&errs, VALIDATION_DATABASE_FIELD),
            vec![VALIDATION_PANIC_CODE.to_string()]
        );
        let ValidationErrorsKind::Field(v) = errs.errors().get(VALIDATION_DATABASE_FIELD).unwrap()
        else {
            unreachable!("expected Field variant");
        };
        assert_eq!(
            v[0].message.as_ref().map(|m| m.to_string()),
            Some("Database error".to_string()),
            "must not leak the raw DbErr message"
        );
    }
}
