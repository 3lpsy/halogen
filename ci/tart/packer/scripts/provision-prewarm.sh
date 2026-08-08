#!/usr/bin/env bash
# Pre-warm the image's build caches (~/.sccache, ~/.cargo/registry): clone,
# run the release builds, keep caches, DELETE the checkout. Trusted image-build
# provenance — no cross-run cache sharing. Skips cleanly without TART_FORGEJO_TOKEN.
set -euo pipefail
eval "$(/opt/homebrew/bin/brew shellenv)"
# shellcheck disable=SC1091
source "$HOME/.cargo/env"
export PATH="/usr/local/bin:${PATH}"

# Root domain of the in-cluster services — passed by packer (tart-build-vm.sh
# seeds it from .env). Every internal hostname derives from it.
SERVICES_ROOT_DOMAIN="${SERVICES_ROOT_DOMAIN:?SERVICES_ROOT_DOMAIN not passed by packer}"
TART_FORGEJO_HOST="${TART_FORGEJO_HOST:-git.${SERVICES_ROOT_DOMAIN}}"
TART_FORGEJO_OWNER="${TART_FORGEJO_OWNER:?TART_FORGEJO_OWNER not passed by packer}"
TART_FORGEJO_REPO="${TART_FORGEJO_REPO:-halogen}"

# Persist the deps-proxy env into ~/.zshenv (read by every zsh, incl. the
# non-interactive build sessions) — written even when prewarm is skipped, so a
# cold image still resolves the proxy.
DEPS_HOST="deps.${SERVICES_ROOT_DOMAIN}"
{
  echo "export SERVICES_ROOT_DOMAIN=${SERVICES_ROOT_DOMAIN}"
  echo "export CARGO_REGISTRIES_CHILLED_PROXY_INDEX=sparse+https://${DEPS_HOST}/crates/index/"
  echo "export npm_config_registry=https://${DEPS_HOST}/npm/"
  echo "export BUN_CONFIG_REGISTRY=https://${DEPS_HOST}/npm/"
  echo "export PIP_INDEX_URL=https://${DEPS_HOST}/pypi/simple/"
  echo "export UV_DEFAULT_INDEX=https://${DEPS_HOST}/pypi/simple/"
  echo "export CHILLED_PROXY_URL=https://${DEPS_HOST}"
} >> "$HOME/.zshenv"
export CARGO_REGISTRIES_CHILLED_PROXY_INDEX="sparse+https://${DEPS_HOST}/crates/index/"
export npm_config_registry="https://${DEPS_HOST}/npm/"
# cargo ignores [source] replacement from the environment (fails open), so the
# replacement must be a FILE — gates any cargo run outside a repo checkout.
mkdir -p "$HOME/.cargo"
printf '[source.crates-io]\nreplace-with = "chilled-proxy"\n' > "$HOME/.cargo/config.toml"
# Maven mirrors `central` and `google` (never `*` — that blackholes jitpack etc).
# NOTE: Gradle ignores these mirrors entirely and needs chilled-proxy's init
# script instead; this VM builds no Gradle projects, so none is installed. Vendor
# data/gradle/chilled-proxy.init.gradle into ~/.gradle/init.d if that changes.
mkdir -p "$HOME/.m2"
printf '%s\n' \
  '<settings xmlns="http://maven.apache.org/SETTINGS/1.0.0">' \
  '  <mirrors>' \
  '    <mirror><id>chilled-proxy-central</id>' \
  "      <url>https://${DEPS_HOST}/maven</url><mirrorOf>central</mirrorOf></mirror>" \
  '    <mirror><id>chilled-proxy-google</id>' \
  "      <url>https://${DEPS_HOST}/google-maven</url><mirrorOf>google</mirrorOf></mirror>" \
  '  </mirrors>' \
  '</settings>' \
  > "$HOME/.m2/settings.xml"

if [ -z "${TART_FORGEJO_TOKEN:-}" ]; then
  echo "==> prewarm: TART_FORGEJO_TOKEN not set — skipping (image ships with cold caches)"
  exit 0
fi

# Same cache the release builds use (build-releases.sh exports these too).
export SCCACHE_DIR="$HOME/.sccache"
# The dev profile's incremental=true bypasses sccache — force it off so the
# debug builds (e2e server, ios-core) warm the cache as well (CI does the same).
export CARGO_INCREMENTAL=0

echo "==> prewarm: cloning ${TART_FORGEJO_OWNER}/${TART_FORGEJO_REPO} @ master"
git clone --quiet --depth 1 \
  "https://${TART_FORGEJO_TOKEN}@${TART_FORGEJO_HOST}/${TART_FORGEJO_OWNER}/${TART_FORGEJO_REPO}.git" "$HOME/halogen"
cd "$HOME/halogen"

echo "==> prewarm: npm ci (also warms the npm cache)"
(cd crates/ui && npm ci)

echo "==> prewarm: iOS debug core (e2e path)"
just ios-core
echo "==> prewarm: server debug (e2e harness)"
cargo build -p halogen-server
echo "==> prewarm: iOS release core (device + sim)"
just ios-core-release
echo "==> prewarm: macOS desktop release"
just ui-desktop-build --release

echo "==> prewarm: first-boot the e2e simulator (bakes the one-time data migration into the image)"
xcrun simctl bootstatus "iPhone 17 Pro" -b || true
xcrun simctl shutdown all || true

echo "==> prewarm: stats"
sccache --show-stats || true  # no head: closing the pipe panics sccache (broken pipe)
du -sh "$HOME/.sccache" "$HOME/.cargo/registry" 2>/dev/null || true

# Token hygiene: the clone's git config holds the token — remove the whole
# checkout. The caches above are what the image keeps.
cd "$HOME"
rm -rf "$HOME/halogen"
echo "==> prewarm: done (checkout removed, caches kept)"
