#!/usr/bin/env bash
# Delete leftover build-<platform>-<tag> VMs (kept by failed runs for retry).
# ONLY ephemeral build clones match — never the builder image, devboxvm, or
# OCI bases. -n = dry run.
set -euo pipefail
script_dir="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=env.sh
source "${script_dir}/env.sh"

dry_run=""
[ "${1:-}" = "-n" ] && dry_run=1

# Local VMs only (never OCI images), names starting with "build-".
victims="$(tart list --format json 2>/dev/null \
  | python3 -c 'import json,sys; [print(v["Name"]) for v in json.load(sys.stdin) if v.get("Source")=="local" and v["Name"].startswith("build-")]' \
  || tart list | awk '$1=="local" && $2 ~ /^build-/ {print $2}')"

if [ -z "${victims}" ]; then
  echo "tart-clean: no build-* VMs to delete"
  exit 0
fi

for vm in ${victims}; do
  if [ -n "${dry_run}" ]; then
    echo "tart-clean: would delete ${vm}"
  else
    echo "tart-clean: deleting ${vm}"
    tart stop "${vm}" >/dev/null 2>&1 || true
    tart delete "${vm}"
  fi
done
