#!/usr/bin/env bash
# fj-help: issue-comment <n> <body> — append a comment to an issue
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
fj_flags "$@"; set -- "${FJ_ARGS[@]+"${FJ_ARGS[@]}"}"

number="${1:-}"; body="${2:-}"
fj_require_num "${number}" "issue number"
[ -n "${body}" ] || fj_usage "comment body is required"

payload="$(jq -nc --arg b "${body}" '{body: $b}')"
out="$(fj_api POST "issues/${number}/comments" "${payload}")"
if [ "${FJ_DRY_RUN}" = 1 ]; then printf '%s\n' "${out}"; exit 0; fi
echo "commented on issue ${number} (comment #$(jq -er '.id' <<<"${out}"))"
