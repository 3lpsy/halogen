pub mod handlers;
pub mod logging;
pub mod restart;
pub mod routers;

// The `tests` module holds the crate's own unit-test infrastructure: `harness`
// (HTTP test support) and `database` (seeding unit tests). Crate-internal `cargo
// test` only; shared `TestRoot` + `load_fixture` live in
// `halogen_fixture::test_support`. The real-server HTTP harness lives in the
// `halogen-integ` crate (which the browser `halogen-e2e` tier also reuses).
#[cfg(test)]
mod tests;
