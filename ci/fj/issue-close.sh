#!/usr/bin/env bash
# fj-help: issue-close <n> <comment> — comment why, then close the issue
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
fj_flags "$@"; set -- "${FJ_ARGS[@]+"${FJ_ARGS[@]}"}"

number="${1:-}"; comment="${2:-}"
fj_require_num "${number}" "issue number"
# The issue is the durable record, not the run: closing requires a reason.
[ -n "${comment}" ] || fj_usage "a closing comment is required"

payload="$(jq -nc --arg b "${comment}" '{body: $b}')"
if [ "${FJ_DRY_RUN}" = 1 ]; then
  fj_api POST "issues/${number}/comments" "${payload}"
  fj_api PATCH "issues/${number}" '{"state":"closed"}'
  exit 0
fi
fj_api POST "issues/${number}/comments" "${payload}" >/dev/null
fj_api PATCH "issues/${number}" '{"state":"closed"}' >/dev/null
echo "closed issue ${number}"
