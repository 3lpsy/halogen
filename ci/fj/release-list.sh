#!/usr/bin/env bash
# fj-help: release-list — releases: tag/kind/name/published
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
fj_flags "$@"; set -- "${FJ_ARGS[@]+"${FJ_ARGS[@]}"}"

rows="$(fj_api_paged 'releases?draft=false')"
if [ "${FJ_JSON}" = 1 ]; then
  printf '%s\n' "${rows}"
else
  jq -r '.[] | [.tag_name, (if .prerelease then "prerelease" else "release" end),
                .name, .published_at] | @tsv' <<<"${rows}"
fi
