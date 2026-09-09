#!/usr/bin/env bash
# Run the justfile gates, stopping on failure: run-tests.sh for all, or run-tests.sh test-crates for one tier.
set -euo pipefail

banner() { printf '\n\033[1;36m=== %s ===\033[0m\n' "$1"; }
sccache_on() { [ "${RUSTC_WRAPPER:-}" = sccache ] && command -v sccache >/dev/null 2>&1; }

requested="${1:-check-all}"
metrics="${HALOGEN_METRICS_FILE:-}"

case "${requested}" in
  check-all)
    gate_output="$(just --dump --dump-format json | jq -er '
      .recipes["check-all"] as $recipe
      | if ($recipe.body | length) != 0
          or ($recipe.dependencies | length) == 0
          or any($recipe.dependencies[]; (.arguments | length) != 0 or .star != null)
        then error("check-all must contain only unparameterized dependencies")
        else $recipe.dependencies[].recipe
        end
    ')"
    mapfile -t recipes <<<"${gate_output}"
    [ "${#recipes[@]}" -gt 0 ] && [ -n "${recipes[0]:-}" ] \
      || { echo "error: check-all has no constituent recipes" >&2; exit 1; }
    ;;
  test-crates|test-webui|test-e2e|clippy) recipes=("${requested}") ;;
  # `check` alone would only be `cargo check`; the tag also runs the cheap
  # structural and Apple release-helper gates, never simulator work.
  check) recipes=(check check-wasm check-tests-run check-webui-packages test-apple-release) ;;
  *) echo "error: unknown tier '${requested}'" >&2; exit 1 ;;
esac

# The browser tier skips silently when chromedriver or dist/ is missing, which
# reads exactly like a pass. In CI that is a failure (TEST-09).
export HALOGEN_E2E_REQUIRED=1

finish_metrics() {
  [ -n "${metrics}" ] || return 0
  node ci/internal/report/ci-metrics.mjs finish "${metrics}" >/dev/null 2>&1 || true
}
trap finish_metrics EXIT

if [ -n "${metrics}" ]; then
  node ci/internal/report/ci-metrics.mjs init "${metrics}"
fi

if sccache_on; then sccache --zero-stats >/dev/null 2>&1 || true; fi
for recipe in "${recipes[@]}"; do
  banner "$recipe"
  if [ -n "${metrics}" ]; then
    node ci/internal/report/ci-metrics.mjs phase "${metrics}" "$recipe" -- just "$recipe"
  else
    just "$recipe"
  fi
done
if sccache_on; then banner "sccache stats"; sccache --show-stats || true; fi
if [ -n "${metrics}" ]; then
  finish_metrics
  trap - EXIT
  node ci/internal/report/ci-metrics.mjs summary "${metrics}"
fi
banner "Passed: ${requested}"
