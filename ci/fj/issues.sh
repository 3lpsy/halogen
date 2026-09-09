#!/usr/bin/env bash
# fj-help: issues [open|closed|all] — the CI issue inventory, title-prefix filtered
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
fj_flags "$@"; set -- "${FJ_ARGS[@]+"${FJ_ARGS[@]}"}"

state="${1:-open}"
case "${state}" in open|closed|all) ;; *) fj_usage "state must be open, closed or all" ;; esac

# Filtered HERE, not by the server: Forgejo ignores a `labels=` filter naming
# a label that does not exist and returns everything.
rows="$(fj_api_paged "issues?state=${state}&type=issues" \
  | jq '[.[] | select(.title | startswith("[ci] "))]')"
if [ "${FJ_JSON}" = 1 ]; then
  printf '%s\n' "${rows}"
else
  jq -r '.[] | [.number, .state, .title] | @tsv' <<<"${rows}"
fi
