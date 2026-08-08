#!/usr/bin/env bash
# All Apple release artifacts for a git tag (TART_TAG=vX overrides): iOS then
# macOS desktop, sequential ephemeral VM clones (macOS 2-running-VM limit).
# Each signs via sign-apple.sh unless TART_SKIP_SIGN=1; upload is separate.
set -euo pipefail
script_dir="$(cd "$(dirname "$0")" && pwd)"

echo "==> release: iOS"
"${script_dir}/build-release-ios.sh" "$@"
echo
echo "==> release: macOS desktop"
"${script_dir}/build-release-desktops.sh" "$@"
echo
echo "==> release: all platforms done"
