#!/usr/bin/env bash
# fj-help: issue-create <title> <body> [labels] — open an issue; labels are comma-separated names
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
fj_flags "$@"; set -- "${FJ_ARGS[@]+"${FJ_ARGS[@]}"}"

title="${1:-}"; body="${2:-}"; labels="${3:-}"
[ -n "${title}" ] || fj_usage "title is required"
[ -n "${body}" ] || fj_usage "body is required"

# The API wants label IDs; resolve the given names and fail on unknown ones.
ids='[]'
if [ -n "${labels}" ]; then
  repo_labels="$(fj_api_paged 'labels')"
  ids="$(jq -c --arg want "${labels}" '
      . as $all | ($want | split(",") | map(gsub("^ +| +$"; ""))) as $names
      | $names | map(
          . as $n | ($all | map(select(.name == $n)) | first)
          | if . == null then error("no such label: \($n)") else .id end)' \
    <<<"${repo_labels}")" || fj_die "label lookup failed (unknown label in '${labels}'?)"
fi

payload="$(jq -nc --arg t "${title}" --arg b "${body}" --argjson l "${ids}" \
  '{title: $t, body: $b} + (if ($l | length) > 0 then {labels: $l} else {} end)')"
out="$(fj_api POST 'issues' "${payload}")"
if [ "${FJ_DRY_RUN}" = 1 ]; then printf '%s\n' "${out}"; exit 0; fi
jq -er '.number' <<<"${out}"
