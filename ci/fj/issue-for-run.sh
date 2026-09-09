#!/usr/bin/env bash
# fj-help: issue-for-run <run> — the CI issue a run opened, by exact title match
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
fj_flags "$@"; set -- "${FJ_ARGS[@]+"${FJ_ARGS[@]}"}"

run="${1:-}"
fj_require_num "${run}" "run number"

# Exact " run N " title match — never fuzzy search, never label filters.
n="$(fj_api_paged 'issues?state=all&type=issues' \
  | jq -r --arg r " run ${run} " '
      [.[] | select(.title | startswith("[ci] ") and contains($r))]
      | first | if . == null then "" else .number|tostring end')"
[ -n "${n}" ] || fj_die "no CI issue for run ${run}"
printf '%s\n' "${n}"
