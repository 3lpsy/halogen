/// Shared HTTP-test support (router builder, JWT minting, entity factories,
/// request/response helpers) for the per-resource `#[cfg(test)] mod tests`.
/// `TestRoot` + `load_fixture` live in `halogen_fixture::test_support`.
#[cfg(test)]
pub mod harness;

/// Database / admin-seeding unit tests (crate-internal `cargo test` only).
#[cfg(test)]
mod database;

mod user_delete;

mod middleware;

mod extractors;
