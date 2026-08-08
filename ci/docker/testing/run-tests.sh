#!/usr/bin/env bash
# Run every Halogen test tier in sequence. Fails on the first failing tier
# (set -e); prints a banner before each so the combined log stays navigable.
set -euo pipefail

banner() { printf '\n\033[1;36m=== %s ===\033[0m\n' "$1"; }

# When sccache is the rustc wrapper (CI), reset its counters up front so the
# end-of-run stats reflect just this run. No-op for a local `just test-all`.
sccache_on() { [ "${RUSTC_WRAPPER:-}" = "sccache" ] && command -v sccache >/dev/null 2>&1; }
if sccache_on; then sccache --zero-stats >/dev/null 2>&1 || true; fi

banner "Format check"
cargo fmt --all -- --check

banner "Compile checks (wasm frontend + native workspace)"
just check-all

# Run the tiers explicitly (not `just test-all`, which aggregates unit+integ+ui+e2e)
# so each gets its own banner and nothing runs twice.
banner "Unit tests (workspace, excl. e2e + ui + integration)"
just test-unit

banner "UI unit tests (halogen-ui, renderless/native)"
just test-ui

banner "Doctests (nextest skips these; no-op today, guards future ones)"
just test-doc

banner "Integration tests (Tier-1 HTTP: real axum + SQLite, mocked RSS)"
just test-integ

banner "Browser e2e (Tier-2 headless Chromium)"
just test-e2e

if sccache_on; then banner "sccache stats"; sccache --show-stats || true; fi

banner "All tiers passed ✔"
