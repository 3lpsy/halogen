//! Resolve defaults < TOML < HALOGEN_* env < CLI < runtime overrides. Only allowlisted overrides are writable at
//! runtime; secrets, binding/identity, and override-file settings stay protected. Config endpoints replace the override
//! set. Save load/rejection diagnostics on Config for logging after startup.

mod config;
mod overrides;
#[cfg(test)]
mod tests;

pub use config::*;
pub use overrides::{read_overrides, write_overrides};
