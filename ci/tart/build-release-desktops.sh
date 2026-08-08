#!/usr/bin/env bash
# macOS desktop release for a git tag (TART_TAG=vX overrides the Cargo.toml
# default) in an ephemeral halogen-builder clone; collected to
# ci/tart/artifacts/<TART_TAG>/release/ and signed unless TART_SKIP_SIGN=1.
set -euo pipefail
# shellcheck source=lib-release.sh
source "$(cd "$(dirname "$0")" && pwd)/lib-release.sh"

release_init desktop
release_boot

echo "==> building macOS desktop ${TART_TAG} in the VM"
release_run <<EOF
set -euo pipefail
export PATH="/usr/local/bin:\${PATH}"
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
echo "--- npm ci (tailwind resolves daisyui from node_modules)"
(cd crates/ui && npm ci)
echo "--- macOS desktop"
just ui-desktop-build --release
echo "--- sccache stats (this run)"
sccache --show-stats || true
echo "--- desktop build complete"
EOF

echo "==> collecting desktop artifacts -> ${TART_OUT_DIR}/release"
mkdir -p "${TART_OUT_DIR}"
# The dx output dir is the shared release/ root — keep an existing ios/
# subdir (from build-release-ios.sh) intact.
tmp_dx="${TART_OUT_DIR}/.dx-release.tmp"
rm -rf "${tmp_dx}"
release_scp "halogen/target/dx/halogen-ui/release" "${tmp_dx}"
mkdir -p "${TART_OUT_DIR}/release"
cp -R "${tmp_dx}/." "${TART_OUT_DIR}/release/"
rm -rf "${tmp_dx}"

if [ -z "${TART_SKIP_SIGN:-}" ]; then
  echo "==> signing"
  TART_TAG="${TART_TAG}" "$(dirname "$0")/sign-apple.sh" "${TART_OUT_DIR}/release"
else
  echo "==> TART_SKIP_SIGN set — artifacts left unsigned"
fi

echo
echo "done: desktop ${TART_TAG}"
find "${TART_OUT_DIR}" -maxdepth 2 -mindepth 1 | sed 's/^/  /'
