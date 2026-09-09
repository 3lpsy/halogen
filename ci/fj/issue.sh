#!/usr/bin/env bash
# fj-help: issue <n> — title, body, and every comment (paginated)
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
fj_flags "$@"; set -- "${FJ_ARGS[@]+"${FJ_ARGS[@]}"}"

number="${1:-}"
fj_require_num "${number}" "issue number"

issue="$(fj_api GET "issues/${number}")"
if [ "${FJ_JSON}" = 1 ]; then
  comments="$(fj_api_paged "issues/${number}/comments?type=comment")"
  jq -n --argjson i "${issue}" --argjson c "${comments}" '{issue: $i, comments: $c}'
  exit 0
fi

jq -r '.title, "", .body' <<<"${issue}"
for page in $(seq 1 200); do
  comments="$(fj_api GET "issues/${number}/comments?limit=100&page=${page}")"
  count="$(jq -er 'if type == "array" then length else error("invalid comments") end' <<<"${comments}")"
  if [ "${count}" -gt 0 ]; then
    jq -r '.[] | "\n--- comment #\(.id) ---\n\(.body)"' <<<"${comments}"
  fi
  [ "${count}" -eq 100 ] || exit 0
done
echo "error: issue comments exceeded 200 pages" >&2
exit 1
