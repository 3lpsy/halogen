use anyhow::Result;
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

pub use halogen_format::dt_human;

pub fn verrors(field: &'static str, code: &'static str, message: String) -> ValidationErrors {
    let mut errs = ValidationErrors::new();
    errs.add(
        field,
        ValidationError::new(code).with_message(message.into()),
    );
    errs
}

/// Derive the HTTP status from the *reason* (`code`) of the first error that carries a non-400 reason. `code`
/// is a reason, never a status name or number — only the genuinely non-400 reasons get an arm; everything else
/// (our `invalid`/ `parsing` and every `validator` built-in: `length`/`range`/`url`/`must_match`/ `regex`) is
/// bad input and falls through to 400.
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
mod tests;
