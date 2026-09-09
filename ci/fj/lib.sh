# shellcheck shell=bash
# Shared library for ci/fj/fj.sh subcommands: host/repo/token resolution, the
# fj_api curl wrapper, and input validators. Sourced, never executed.
# Nothing internal is hardcoded — host and repo derive from env or the remote.
{ set +x; } 2>/dev/null  # the token must never leak through xtrace

set -euo pipefail

FJ_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FJ_REMOTE="${HALOGEN_FJ_REMOTE:-internal}"
FJ_JSON="${FJ_JSON:-0}"
FJ_DRY_RUN="${FJ_DRY_RUN:-0}"

fj_die() { echo "error: $*" >&2; exit "${FJ_EXIT:-1}"; }
fj_usage() { FJ_EXIT=2 fj_die "$*"; }

# Consume leading --json/--dry-run flags; positional args land in FJ_ARGS.
fj_flags() {
  FJ_ARGS=()
  while [ $# -gt 0 ]; do
    case "$1" in
      --json) FJ_JSON=1 ;;
      --dry-run) FJ_DRY_RUN=1 ;;
      --) shift; FJ_ARGS+=("$@"); break ;;
      -*) fj_usage "unknown flag: $1" ;;
      *) FJ_ARGS+=("$1") ;;
    esac
    shift
  done
}

fj_require_tools() {
  command -v curl >/dev/null || fj_die "curl is required"
  command -v jq >/dev/null || fj_die "jq is required"
}

# host/owner/repo behind the remote; ssh://, scp-style and https URLs all
# reduce to the same slug.
fj_slug() {
  git -C "${FJ_ROOT}" remote get-url "${FJ_REMOTE}" \
    | sed -E 's#^[a-z+]+://##; s#^[^@/]*@##; s#:#/#; s#\.git$##'
}

fj_host() {
  if [ -n "${FORGEJO_HOST:-}" ]; then
    printf '%s\n' "${FORGEJO_HOST}" | sed -E 's#^[a-z+]+://##; s#/+$##'
  else
    fj_slug | cut -d/ -f1
  fi
}

fj_repo() {
  if [ -n "${FORGEJO_REPO_PATH:-}" ]; then
    printf '%s\n' "${FORGEJO_REPO_PATH#/}"
  else
    local slug; slug="$(fj_slug)"
    printf '%s\n' "${slug#*/}"
  fi
}

# Token: env, then .env at repo root (parsed, never sourced), then the fj
# CLI's own credential store. Never printed, never placed in argv.
fj_token() {
  if [ -n "${FORGEJO_TOKEN:-}" ]; then printf '%s\n' "${FORGEJO_TOKEN}"; return 0; fi
  local line
  if [ -f "${FJ_ROOT}/.env" ]; then
    line="$(grep -E '^(export +)?FORGEJO_TOKEN=' "${FJ_ROOT}/.env" | tail -n1 || true)"
    if [ -n "${line}" ]; then
      line="${line#*=}"; line="${line%\"}"; line="${line#\"}"; line="${line%\'}"; line="${line#\'}"
      [ -n "${line}" ] && { printf '%s\n' "${line}"; return 0; }
    fi
  fi
  local keys host
  keys="${FORGEJO_CLI_KEYS:-${XDG_DATA_HOME:-${HOME}/.local/share}/forgejo-cli/keys.json}"
  host="$(fj_host)"
  line="$(jq -er --arg h "${host}" '.hosts[$h].token // empty' "${keys}" 2>/dev/null || true)"
  [ -n "${line}" ] || fj_die "no token: set FORGEJO_TOKEN, put it in .env, or log in with fj (${keys})"
  printf '%s\n' "${line}"
}

# fj_api METHOD PATH [JSON_BODY] — repo-relative PATH, or absolute under
# /api/v1 when PATH starts with '/'. Prints the body; on HTTP >= 400 prints
# it to stderr and fails. Retries once on a 5xx. Honors FJ_DRY_RUN.
fj_api() {
  local method="$1" path="$2" body="${3:-}" url
  case "${path}" in
    /*) url="https://$(fj_host)/api/v1${path}" ;;
    *) url="https://$(fj_host)/api/v1/repos/$(fj_repo)/${path}" ;;
  esac
  # Dry-run stubs mutations only; GETs stay live so lookups still resolve.
  if [ "${FJ_DRY_RUN}" = 1 ] && [ "${method}" != GET ]; then
    echo "DRY ${method} ${url}"
    [ -n "${body}" ] && printf '%s\n' "${body}"
    return 0
  fi
  local args=( -sS --max-time 60 -X "${method}" -H 'Accept: application/json' )
  [ -n "${body}" ] && args=( "${args[@]}" -H 'Content-Type: application/json' -d "${body}" )
  local attempt out http
  for attempt in 1 2; do
    # The auth header rides in on a curl config from stdin, never in argv.
    out="$(curl "${args[@]}" -w $'\n%{http_code}' --config - "${url}" <<EOF
header = "Authorization: token $(fj_token)"
EOF
)" || fj_die "curl failed: ${method} ${path}"
    http="${out##*$'\n'}"; out="${out%$'\n'*}"
    case "${http}" in
      2*) printf '%s\n' "${out}"; return 0 ;;
      5*) [ "${attempt}" = 1 ] && { sleep 2; continue; } ;;
    esac
    { echo "error: HTTP ${http} from ${method} ${path}"; printf '%s\n' "${out}"; } >&2
    return 1
  done
}

# GET every page of an array endpoint and print the concatenated array.
fj_api_paged() {
  local path="$1" page=1 out pages='' sep='?'
  [[ "${path}" == *\?* ]] && sep='&'
  while :; do
    out="$(fj_api GET "${path}${sep}limit=50&page=${page}")"
    jq -e 'type == "array"' >/dev/null <<<"${out}" || fj_die "expected an array from ${path}"
    pages="${pages}${out}"$'\n'
    [ "$(jq length <<<"${out}")" -lt 50 ] && break
    page=$((page + 1))
    [ "${page}" -le 40 ] || fj_die "pagination exceeded 40 pages for ${path}"
  done
  jq -s 'add // []' <<<"${pages}"
}

fj_require_num() {  # positive integer
  [[ "${1:-}" =~ ^[1-9][0-9]*$ ]] || fj_usage "${2:-number} must be a positive integer"
}

fj_require_workflow() {  # a workflow file name, e.g. ios-task.yml
  [[ "${1:-}" =~ ^[A-Za-z0-9._-]+\.ya?ml$ ]] \
    || fj_usage "workflow must be a workflow file name like ios-task.yml"
}

fj_require_ref() {  # branch/tag/sha — safe charset, no traversal or flag shapes
  [[ "${1:-}" =~ ^[A-Za-z0-9][A-Za-z0-9._/-]*$ && "${1}" != *..* ]] \
    || fj_usage "ref '${1:-}' is not a valid git ref"
}

fj_require_tag() {  # single path segment: safe in URLs without encoding
  [[ "${1:-}" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ && "${1}" != *..* ]] \
    || fj_usage "tag '${1:-}' must be alphanumeric with . _ - only"
}

fj_require_tools
