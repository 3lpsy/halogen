#!/usr/bin/env bash
# iOS release for a git tag (TART_TAG=vX overrides) in an ephemeral
# halogen-builder clone: e2e gate + screenshots, then the unsigned Release app
# → ci/tart/artifacts/<TART_TAG>/release/ios/, signed unless TART_SKIP_SIGN=1.
# Upload stays a separate, explicit ./upload-apple.sh.
set -euo pipefail
# shellcheck source=lib-release.sh
source "$(cd "$(dirname "$0")" && pwd)/lib-release.sh"

release_init ios
release_boot

echo "==> building iOS ${TART_TAG} in the VM"
# The ephemeral VM dies on failure — rescue the e2e diagnostics (xcresult,
# server log, video) first so a red gate is triageable from the host.
build_rc=0
release_run <<EOF || build_rc=$?
set -euo pipefail
export PATH="/usr/local/bin:\${PATH}"
# Hit the image's pre-warmed compile cache (provision-prewarm.sh); the repo
# default SCCACHE_DIR is repo-relative (empty in a fresh clone) and the dev
# profile's incremental bypasses sccache — override both, same as CI.
export SCCACHE_DIR="\$HOME/.sccache"
export CARGO_INCREMENTAL=0
# Per-run cache accounting (stats survive since the sccache server
# starts at first compile — zero them so the report below is THIS run).
sccache --zero-stats >/dev/null 2>&1 || true
echo "--- checkout ${TART_FORGEJO_OWNER}/${TART_FORGEJO_REPO} @ ${TART_TAG}"
if [ -d halogen/.git ]; then
  # Reused leftover VM: update its checkout — force-fetch the tag
  # since release tags get re-pointed during pipeline iteration.
  cd halogen
  git fetch --force --quiet origin "refs/tags/${TART_TAG}:refs/tags/${TART_TAG}"
  git -c advice.detachedHead=false checkout --force --quiet "refs/tags/${TART_TAG}"
  git reset --hard --quiet "refs/tags/${TART_TAG}"
else
  git -c advice.detachedHead=false clone --quiet --depth 1 --branch "${TART_TAG}" \
    "https://${TART_FORGEJO_TOKEN}@${TART_FORGEJO_HOST}/${TART_FORGEJO_OWNER}/${TART_FORGEJO_REPO}.git" halogen
  cd halogen
fi
# Images built before the native-iOS tooling lack these — self-heal (brew,
# bottled, seconds) until the image is rebuilt with them provisioned.
command -v typeshare >/dev/null 2>&1 || brew install typeshare
command -v xcodegen >/dev/null 2>&1 || brew install xcodegen
echo "--- iOS prereqs: e2e journeys + screenshots (release gate — a red suite aborts the release)"
# PROFILE=release: no dev-profile cargo builds in the release pipeline — the
# gate runs against the same release Rust cores the archive ships.
just ios-screenshots "iPhone 17 Pro" release
echo "--- iOS release build (unsigned device app)"
just ios-collect
echo "--- sccache stats (this run)"
sccache --show-stats || true
echo "--- iOS build complete"
EOF

if [ "${build_rc}" != "0" ]; then
  echo "==> BUILD FAILED (rc=${build_rc}) — rescuing e2e diagnostics -> ${TART_OUT_DIR}/ios-e2e-failure"
  mkdir -p "${TART_OUT_DIR}"
  rm -rf "${TART_OUT_DIR}/ios-e2e-failure"
  release_scp "halogen/data/ios/artifacts/e2e" "${TART_OUT_DIR}/ios-e2e-failure" || true
  echo "    triage: xcrun xcresulttool get test-results summary --path ${TART_OUT_DIR}/ios-e2e-failure/run.xcresult"
  exit "${build_rc}"
fi

echo "==> collecting iOS artifacts -> ${TART_OUT_DIR}"
mkdir -p "${TART_OUT_DIR}/release"
rm -rf "${TART_OUT_DIR}/release/ios" "${TART_OUT_DIR}/screenshots"
release_scp "halogen/artifacts/release/ios" "${TART_OUT_DIR}/release/"
# Screenshots moved to data/screenshots/ios/latest (549bf3f); the old
# halogen/screenshots path made this scp fail and abort BEFORE signing.
# Tolerant: missing screenshots must not block a release.
mkdir -p "${TART_OUT_DIR}/screenshots"
release_scp "halogen/data/screenshots/ios/latest/*" "${TART_OUT_DIR}/screenshots/" \
  || echo "    (no screenshots collected — continuing)"

if [ -z "${TART_SKIP_SIGN:-}" ]; then
  echo "==> signing"
  TART_TAG="${TART_TAG}" "$(dirname "$0")/sign-apple.sh" "${TART_OUT_DIR}/release"
else
  echo "==> TART_SKIP_SIGN set — artifacts left unsigned"
fi

echo
echo "done: iOS ${TART_TAG}"
find "${TART_OUT_DIR}" -maxdepth 2 -mindepth 1 | sed 's/^/  /'
