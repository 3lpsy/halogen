#!/usr/bin/env bash
# fj-help: tag <name> [ref] — create a tag server-side (default ref: master)
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
fj_flags "$@"; set -- "${FJ_ARGS[@]+"${FJ_ARGS[@]}"}"

name="${1:-}"; ref="${2:-master}"
fj_require_tag "${name}"
fj_require_ref "${ref}"

payload="$(jq -nc --arg t "${name}" --arg r "${ref}" '{tag_name: $t, target: $r}')"
if [ "${FJ_DRY_RUN}" = 1 ]; then
  fj_api POST 'tags' "${payload}"
  exit 0
fi
fj_api POST 'tags' "${payload}" >/dev/null
echo "created tag ${name} at ${ref}"
