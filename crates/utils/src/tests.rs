use super::*;

// --- extract_status_code -------------------------------------------------

#[test]
fn extract_status_code_reason_codes_map_to_status() {
    use crate::constants::{
        VALIDATION_CONFLICT_CODE, VALIDATION_EXISTS_CODE, VALIDATION_PANIC_CODE,
        VALIDATION_UNAUTHENTICATED_CODE, VALIDATION_UNAUTHORIZED_CODE,
        VALIDATION_UNIMPLEMENTED_CODE, VALIDATION_UNIQUE_CODE,
    };
    // Each genuinely non-400 reason maps to its HTTP status.
    let cases = [
        (VALIDATION_EXISTS_CODE, 404u16),
        (VALIDATION_UNAUTHENTICATED_CODE, 401),
        (VALIDATION_UNAUTHORIZED_CODE, 403),
        (VALIDATION_UNIQUE_CODE, 409),
        (VALIDATION_CONFLICT_CODE, 409),
        (VALIDATION_UNIMPLEMENTED_CODE, 501),
        (VALIDATION_PANIC_CODE, 500),
    ];
    for (code, want) in cases {
        let errs = verrors("id", code, "msg".to_string());
        assert_eq!(
            extract_status_code(&errs),
            want,
            "code {code:?} should map to {want}"
        );
    }
}

#[test]
fn extract_status_code_validation_reasons_are_400() {
    use crate::constants::{VALIDATION_INVALID_CODE, VALIDATION_PARSING_CODE};
    // Our own input reasons plus representative `validator` built-ins all fall
    // through to 400 — no explicit arm, no enumeration.
    for code in [
        VALIDATION_INVALID_CODE,
        VALIDATION_PARSING_CODE,
        "length",
        "range",
        "url",
        "must_match",
        "regex",
    ] {
        let errs = verrors("data", code, "msg".to_string());
        assert_eq!(
            extract_status_code(&errs),
            400,
            "code {code:?} should be 400"
        );
    }
}

#[test]
fn extract_status_code_numeric_code_is_not_a_status() {
    // The old numeric passthrough is gone: a code that happens to be digits is
    // just an unknown reason → 400, NOT that number.
    let errs = verrors("field", "404", "msg".to_string());
    assert_eq!(extract_status_code(&errs), 400);
}

#[test]
fn extract_status_code_unknown_defaults_to_400() {
    let errs = verrors("field", "some_unmapped_code", "msg".to_string());
    assert_eq!(extract_status_code(&errs), 400);
}

#[test]
fn extract_status_code_empty_defaults_to_400() {
    let errs = ValidationErrors::new();
    assert_eq!(extract_status_code(&errs), 400);
}

#[test]
fn extract_status_code_returns_first_recognized_reason() {
    use crate::constants::{VALIDATION_EXISTS_CODE, VALIDATION_INVALID_CODE};
    // A 400-default reason on one field and a recognized non-400 reason on
    // another: the non-400 reason wins regardless of field iteration order.
    let mut errs = verrors("data", VALIDATION_INVALID_CODE, "bad".to_string());
    errs.add(
        "id",
        ValidationError::new(VALIDATION_EXISTS_CODE).with_message("missing".into()),
    );
    assert_eq!(extract_status_code(&errs), 404);
}

// --- codify --------------------------------------------------------------

#[test]
fn codify_uppercases_and_replaces_spaces() {
    assert_eq!(codify("hello world"), "HELLO_WORLD");
    assert_eq!(codify("Already_Done"), "ALREADY_DONE");
    assert_eq!(codify("a b c"), "A_B_C");
    assert_eq!(codify(""), "");
    assert_eq!(codify("nospaces"), "NOSPACES");
}

// --- resolve_path --------------------------------------------------------

#[test]
fn resolve_path_absolute_unchanged() {
    let abs = PathBuf::from("/tmp/halogen/x.db");
    assert_eq!(resolve_path(&abs), abs);
}

#[test]
fn resolve_path_relative_joined_to_cwd() {
    let rel = PathBuf::from("sub/dir/x.db");
    let resolved = resolve_path(&rel);
    assert!(resolved.is_absolute());
    assert!(resolved.ends_with("sub/dir/x.db"));
    assert_eq!(resolved, cwd().join(&rel));
}

// --- ensure_parent_dir ---------------------------------------------------

#[test]
fn ensure_parent_dir_creates_missing_parents() {
    let base = env::temp_dir().join(format!("halogen_epd_{}", std::process::id()));
    // best-effort clean slate
    let _ = fs::remove_dir_all(&base);
    let file = base.join("a/b/c/file.txt");
    ensure_parent_dir(&file).expect("should create parents");
    assert!(file.parent().unwrap().is_dir());
    let _ = fs::remove_dir_all(&base);
}

#[test]
fn ensure_parent_dir_no_parent_is_ok() {
    // A bare relative filename has an empty parent — must not error.
    ensure_parent_dir(Path::new("just_a_name")).expect("empty parent is a no-op");
}

// --- verrors -------------------------------------------------------------

#[test]
fn verrors_builds_single_field_error() {
    let errs = verrors("username", "length", "too short".to_string());
    let map = errs.errors();
    assert!(map.contains_key("username"));
    let ValidationErrorsKind::Field(field_errs) = map.get("username").unwrap() else {
        unreachable!("expected a Field variant");
    };
    assert_eq!(field_errs.len(), 1);
    assert_eq!(field_errs[0].code.as_ref(), "length");
    assert_eq!(
        field_errs[0].message.as_ref().map(|m| m.to_string()),
        Some("too short".to_string())
    );
}
