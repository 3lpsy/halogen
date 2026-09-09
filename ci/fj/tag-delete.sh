#!/usr/bin/env bash
# fj-help: tag-delete <name> — delete a tag server-side
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
fj_flags "$@"; set -- "${FJ_ARGS[@]+"${FJ_ARGS[@]}"}"

name="${1:-}"
fj_require_tag "${name}"

if [ "${FJ_DRY_RUN}" = 1 ]; then
  fj_api DELETE "tags/${name}"
  exit 0
fi
fj_api DELETE "tags/${name}" >/dev/null
echo "deleted tag ${name}"
