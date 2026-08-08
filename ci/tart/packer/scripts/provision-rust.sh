#!/bin/bash
# Rust toolchain + cargo tools for halogen's Apple-target builds (iOS device +
# simulator, macOS desktop). Runs as `admin` inside the VM during
# `packer build` (see ../halogen-builder.pkr.hcl).
set -euo pipefail

# brew is only on PATH via ~/.zprofile (login shells) — neither packer's sh
# nor `zsh -s` reads that. Wire it here AND into ~/.zshenv or node is missing.
eval "$(/opt/homebrew/bin/brew shellenv)"
export HOMEBREW_NO_AUTO_UPDATE=1
echo 'eval "$(/opt/homebrew/bin/brew shellenv)"' >> "$HOME/.zshenv"

echo "==> node (npm ci needs it; preinstalled via brew in the base image)"
command -v node >/dev/null 2>&1 || brew install node

echo "==> xcodegen (generates ios/Halogen.xcodeproj for the native iOS app)"
command -v xcodegen >/dev/null 2>&1 || brew install xcodegen

echo "==> typeshare (generates ios/Generated/WireTypes.swift from crates/wire*)"
command -v typeshare >/dev/null 2>&1 || brew install typeshare

echo "==> rustup (stable) + cross-compilation targets"
curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
# shellcheck disable=SC1091
source "$HOME/.cargo/env"

# The host triple (aarch64-apple-darwin) covers the mac desktop build; add the
# iOS device + simulator targets on top.
rustup target add aarch64-apple-ios aarch64-apple-ios-sim

echo "==> cargo-binstall + just / dx / nextest / sccache"
curl -fsSL -o /tmp/cargo-binstall.zip \
  https://github.com/cargo-bins/cargo-binstall/releases/latest/download/cargo-binstall-aarch64-apple-darwin.zip
unzip -o /tmp/cargo-binstall.zip -d "$HOME/.cargo/bin"
rm -f /tmp/cargo-binstall.zip
# dx must exactly match the workspace dioxus version (lockstep with
# crates/ui/Cargo.toml + the toolchain Dockerfile); sccache must be on PATH
# (the repo's .cargo/config.toml defaults rustc-wrapper to it).
cargo binstall -y just dioxus-cli@0.7.9 cargo-nextest sccache

echo "==> tailwindcss standalone CLI (the justfile 'tailwind' recipe expects it on PATH)"
sudo mkdir -p /usr/local/bin
sudo curl -fsSL -o /usr/local/bin/tailwindcss \
  https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-macos-arm64
sudo chmod +x /usr/local/bin/tailwindcss
# /usr/local/bin reaches login shells via path_helper (/etc/zprofile), but the
# `zsh -s` build sessions are non-login — put it on PATH for those too.
echo 'export PATH="/usr/local/bin:$PATH"' >> "$HOME/.zshenv"

# Make cargo visible to non-interactive SSH shells (`tart exec` / `ssh <cmd>`):
# zsh sources ~/.zshenv for every invocation, login or not.
echo 'source "$HOME/.cargo/env"' >> "$HOME/.zshenv"
