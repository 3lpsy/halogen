/// Dev-environment data seeding. Debug-only by default — the production
/// server has no business populating fixture data. The `dev-seed` feature
/// compiles it into release builds for the ONE consumer that needs that:
/// the iOS e2e harness server, which runs release-profile in the release
/// pipeline (justfile `ios-e2e … release`).
#[cfg(any(debug_assertions, feature = "dev-seed"))]
pub mod dev;
pub mod playlist;
pub mod user;

/// Test-only scratch dirs + fixture loader, shared by the workspace's
/// `#[cfg(test)]` suites. Gated so it never ships in a normal build.
#[cfg(feature = "test-support")]
pub mod test_support;
