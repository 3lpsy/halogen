#!/usr/bin/env bash
# fj-help: issue-artifacts <n> [dest] — download an issue's attachments (default /tmp/ci-<n>)
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
fj_flags "$@"; set -- "${FJ_ARGS[@]+"${FJ_ARGS[@]}"}"

number="${1:-}"; dest="${2:-}"
fj_require_num "${number}" "issue number"
[ -n "${dest}" ] || dest="/tmp/ci-${number}"
mkdir -p "${dest}"

fj_api GET "issues/${number}/assets" \
  | jq -r '.[] | [.name, .browser_download_url] | @tsv' \
  | while IFS="$(printf '\t')" read -r name url; do
      [ -n "${url}" ] || continue
      # Asset names come from the API response: never let one escape DEST.
      case "${name}" in */*|*\\*|.|..|"") echo "skipping unsafe asset name: ${name}" >&2; continue ;; esac
      # Auth rides in a curl config from stdin so the token stays out of argv.
      curl -fsSL --max-time 300 -o "${dest}/${name}" --config - "${url}" <<EOF
header = "Authorization: token $(fj_token)"
EOF
      echo "${dest}/${name}"
    done
