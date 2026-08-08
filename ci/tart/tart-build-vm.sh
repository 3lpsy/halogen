#!/usr/bin/env bash
# Build the halogen-builder Tart VM image with Packer (one-time / toolchain
# bumps), on an Apple Silicon mac with tart + packer (brew). Override via env:
# TART_BASE_IMAGE (pin Xcode), TART_VM_CPU/MEMORY_GB/DISK_GB; config from .env.
# First run pulls a tens-of-GB base image (cached under ~/.tart).
set -euo pipefail
cd "$(dirname "$0")"
# shellcheck source=env.sh
source ./env.sh

TART_BASE_IMAGE="${TART_BASE_IMAGE:-ghcr.io/cirruslabs/macos-tahoe-xcode:latest}"
TART_VM_NAME="${TART_VM_NAME:-halogen-builder}"

# Packer scratch + plugin/image cache on the external SSD next to TART_HOME
# (env.sh) — the internal disk can't absorb multi-GB temp copies.
export TMPDIR="${TART_HOME}/tmp"
export PACKER_CACHE_DIR="${TART_HOME}/packer-cache"
mkdir -p "${TMPDIR}" "${PACKER_CACHE_DIR}"

# Pre-pull so download progress is visible (packer would pull quietly).
tart pull "${TART_BASE_IMAGE}"

cd packer
packer init .
# TART_FORGEJO_TOKEN (.env at repo root) enables the cache pre-warm; without it the
# image builds fine but ships cold caches. Sizing defaults match the release
# clones (8 cpu / 12 GB) — the prewarm compiles are the slow part of an image
# build, and the sizing persists into the image (clones inherit it).
packer build \
  -var "base_image=${TART_BASE_IMAGE}" \
  -var "vm_name=${TART_VM_NAME}" \
  -var "cpu_count=${TART_VM_CPU:-8}" \
  -var "memory_gb=${TART_VM_MEMORY_GB:-12}" \
  -var "disk_size_gb=${TART_VM_DISK_GB:-0}" \
  -var "forgejo_token=${TART_FORGEJO_TOKEN:-}" \
  -var "services_root_domain=${SERVICES_ROOT_DOMAIN:?SERVICES_ROOT_DOMAIN not set (.env at repo root)}" \
  -var "forgejo_host=${TART_FORGEJO_HOST:-git.${SERVICES_ROOT_DOMAIN}}" \
  -var "forgejo_owner=${TART_FORGEJO_OWNER:?TART_FORGEJO_OWNER not set (.env at repo root)}" \
  -var "forgejo_repo=${TART_FORGEJO_REPO:-halogen}" \
  halogen-builder.pkr.hcl

echo
echo "Built VM '${TART_VM_NAME}'."
echo "Smoke test:    tart run ${TART_VM_NAME} --no-graphics   (then: ssh admin@\$(tart ip ${TART_VM_NAME}), password 'admin')"
echo "Release build: ./build-releases.sh   (see docs/internal/TART.md)"
