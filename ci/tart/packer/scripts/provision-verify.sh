#!/bin/bash
# Toolchain inventory — the last provisioning step, so a missing tool fails the
# packer build here (with a readable log) instead of at first use.
set -euo pipefail
# shellcheck disable=SC1091
source "$HOME/.cargo/env"
# brew's /opt/homebrew/bin (node/npm) isn't on PATH in packer's plain-sh
# provisioner session — same reason provision-rust.sh wires it into ~/.zshenv.
eval "$(/opt/homebrew/bin/brew shellenv)"

echo "== xcode =="
xcodebuild -version
xcrun simctl list runtimes | head -5 || true

echo "== rust =="
rustc -V
cargo -V
rustup target list --installed

echo "== cargo tools =="
just --version
dx --version
cargo nextest --version
sccache --version

echo "== tailscale =="
/opt/homebrew/bin/tailscale version | head -1
sudo launchctl print system/com.tailscale.tailscaled >/dev/null && echo "tailscaled daemon loaded"

echo "== web tooling =="
/usr/local/bin/tailwindcss --help 2>&1 | head -1
node --version
npm --version

echo "provisioning verified"
