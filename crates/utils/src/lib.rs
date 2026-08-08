use anyhow::Result;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use tracing::info;
use validator::{ValidationError, ValidationErrors, ValidationErrorsKind};
pub mod constants;
pub mod environment;
pub mod opml;
pub mod patterns;

use crate::constants::{
    VALIDATION_CONFLICT_CODE, VALIDATION_EXISTS_CODE, VALIDATION_PANIC_CODE,
    VALIDATION_UNAUTHENTICATED_CODE, VALIDATION_UNAUTHORIZED_CODE, VALIDATION_UNIMPLEMENTED_CODE,
    VALIDATION_UNIQUE_CODE,
};

pub fn codify(value: &str) -> String {
    value.to_owned().replace(" ", "_").to_ascii_uppercase()
}

pub fn dt_human(timestamp: DateTime<Utc>, timezone: &Tz) -> String {
    let dt_local = timestamp.with_timezone(timezone);
    dt_local.format("%B %d, %Y at %I:%M %p").to_string()
}

pub fn verrors(field: &'static str, code: &'static str, message: String) -> ValidationErrors {
    let mut errs = ValidationErrors::new();
    errs.add(
        field,
        ValidationError::new(code).with_message(message.into()),
    );
    errs
}

/// Derive the HTTP status from the *reason* (`code`) of the first error that
/// carries a non-400 reason. `code` is a reason, never a status name or number —
/// only the genuinely non-400 reasons get an arm; everything else (our `invalid`/
/// `parsing` and every `validator` built-in: `length`/`range`/`url`/`must_match`/
/// `regex`) is bad input and falls through to 400.
pub fn extract_status_code(errs: &ValidationErrors) -> u16 {
    for kind in errs.errors().values() {
        if let ValidationErrorsKind::Field(validations) = kind {
            for validation in validations {
                let status = match validation.code.as_ref() {
                    VALIDATION_EXISTS_CODE => 404,
                    VALIDATION_UNAUTHENTICATED_CODE => 401,
                    VALIDATION_UNAUTHORIZED_CODE => 403,
                    VALIDATION_UNIQUE_CODE | VALIDATION_CONFLICT_CODE => 409,
                    VALIDATION_UNIMPLEMENTED_CODE => 501,
                    VALIDATION_PANIC_CODE => 500,
                    // invalid / parsing / validator built-ins / anything unknown
                    // → bad input. Keep scanning in case a later field carries a
                    // recognized non-400 reason.
                    _ => continue,
                };
                return status;
            }
        }
    }
    400
}

pub fn touch(path: &PathBuf) -> Result<()> {
    if !path.exists() {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(path)?;
    }
    Ok(())
}

pub fn load_dotenvs(dotenvs: Vec<PathBuf>) -> Result<()> {
    for dotenv in dotenvs {
        if dotenv.exists() {
            // `from_path` doesn't override vars already set in the real environment,
            // so the process env wins over the .env file
            info!("Loading environment file: {}", &dotenv.to_string_lossy());
            dotenvy::from_path(&dotenv)?;
        }
    }
    Ok(())
}

pub fn resolve_path<P: AsRef<Path>>(path: &P) -> PathBuf {
    let path = path.as_ref();

    if path.is_absolute() {
        path.to_path_buf()
    } else {
        let base_path = cwd();
        // dev runners may launch from a different cwd; resolve against it
        base_path.join(path)
    }
}

// should only be used for dev, prod paths should be known or configured
pub fn cwd() -> PathBuf {
    env::current_dir().unwrap()
}

pub fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && parent != Path::new("")
    {
        fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("Failed to create directory {:?}: {}", parent, e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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

    // --- dt_human ------------------------------------------------------------

    #[test]
    fn dt_human_formats_fixed_timestamp_in_utc() {
        // Fixed instant: 2021-03-04 09:05:00 UTC.
        let ts = Utc.with_ymd_and_hms(2021, 3, 4, 9, 5, 0).unwrap();
        let out = dt_human(ts, &chrono_tz::UTC);
        assert_eq!(out, "March 04, 2021 at 09:05 AM");
    }

    #[test]
    fn dt_human_respects_timezone_offset() {
        // 2021-03-04 09:05:00 UTC is 04:05 AM in New York (EST, -5).
        let ts = Utc.with_ymd_and_hms(2021, 3, 4, 9, 5, 0).unwrap();
        let out = dt_human(ts, &chrono_tz::America::New_York);
        assert_eq!(out, "March 04, 2021 at 04:05 AM");
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
}
