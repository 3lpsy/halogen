//! Config resolution: [`Cli`] (clap) + [`ConfigFile`] (TOML) → resolved
//! [`Config`].
//!
//! Five layers, highest priority last: defaults < TOML < env (`HALOGEN_*`) < CLI
//! < the runtime overrides file. The overrides file is the only writable-at-
//! runtime layer; it honours an allowlist (`apply_overrides`) and never touches
//! secrets, binding/identity fields, or its own `config_overrides_*` knobs. The
//! `read_overrides` / `write_overrides` helpers back the `/config-overrides`
//! endpoints (which replace the override set wholesale). Logging isn't up during
//! `resolve`, so
//! anything noteworthy (what the overrides file changed, rejected keys, load
//! errors) is stashed on `Config` and logged from `main`.

mod config;
mod overrides;
#[cfg(test)]
mod tests;

pub use config::*;
pub use overrides::{read_overrides, write_overrides};
