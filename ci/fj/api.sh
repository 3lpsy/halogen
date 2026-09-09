#!/usr/bin/env bash
# fj-help: api <method> <path> [json-body] — raw API call; path is repo-relative, or /absolute under /api/v1
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
fj_flags "$@"; set -- "${FJ_ARGS[@]+"${FJ_ARGS[@]}"}"

method="${1:-}"; path="${2:-}"; body="${3:-}"
case "${method}" in GET|POST|PUT|PATCH|DELETE) ;; *) fj_usage "method must be GET/POST/PUT/PATCH/DELETE" ;; esac
[[ "${path}" =~ ^[[:graph:]]+$ ]] || fj_usage "path is required and must not contain whitespace"
if [ -n "${body}" ]; then
  jq -e . >/dev/null 2>&1 <<<"${body}" || fj_usage "body must be valid JSON"
fi

fj_api "${method}" "${path}" "${body}"
