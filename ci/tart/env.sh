# shellcheck shell=bash
# Shared env loader for the ci/tart scripts — source, don't execute. Loads the
# repo-root .env without clobbering: shell env WINS over .env, script defaults
# apply last. Values take everything after the first '='; one quote pair stripped.

# Absolute path of ci/tart — resolved at source time so scripts can cd freely.
TART_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Repo root (where .env and the justfile live).
REPO_ROOT="$(cd "${TART_DIR}/../.." && pwd)"

if [ -f "${REPO_ROOT}/.env" ]; then
  while IFS= read -r _line || [ -n "${_line}" ]; do
    case "${_line}" in
      ''|\#*) continue ;;
    esac
    _key="${_line%%=*}"
    case "${_key}" in
      *[!A-Za-z0-9_]*|'') continue ;;   # skip anything that isn't a clean KEY=
    esac
    # Only adopt the .env value when the variable isn't already set.
    if [ -z "${!_key+x}" ]; then
      _val="${_line#*=}"
      # Strip one pair of matching surrounding quotes.
      case "${_val}" in
        \"*\") _val="${_val#\"}"; _val="${_val%\"}" ;;
        \'*\') _val="${_val#\'}"; _val="${_val%\'}" ;;
      esac
      # Expand a LEADING ~/ or $HOME/ — the one shell-ism people expect in
      # path values (everything else stays literal; this is not eval).
      case "${_val}" in
        '~/'*) _val="${HOME}/${_val#'~/'}" ;;
        '$HOME/'*) _val="${HOME}/${_val#'$HOME/'}" ;;
      esac
      export "${_key}=${_val}"
    fi
  done < "${REPO_ROOT}/.env"
fi
unset _line _key _val

# All tart state (images, VMs) lives on the external SSD — the internal disk
# is too small for the tens-of-GB Xcode images. Guard against the volume not
# being mounted: /Volumes is admin-writable, so a blind mkdir would silently
# recreate TART_HOME on the internal disk and fill it.
: "${TART_HOME:=/Volumes/DevboxExt/Tart}"
if [ ! -d "$(dirname "${TART_HOME}")" ]; then
  echo "error: $(dirname "${TART_HOME}") is not mounted — attach the external SSD (or override TART_HOME)" >&2
  return 1 2>/dev/null || exit 1
fi
mkdir -p "${TART_HOME}"
export TART_HOME

# Default release tag: v<version> from Cargo.toml [workspace.package] — the
# same parse the justfile's ci-tagged-release uses, so the two always agree.
# Callers: TART_TAG="${TART_TAG:-$(halogen_default_tag)}".
halogen_default_tag() {
  local v
  v="$(sed -n '/^\[workspace.package\]/,/^\[/p' "${TART_DIR}/../../Cargo.toml" \
    | grep -m1 '^version' | sed -E 's/.*"([^"]+)".*/\1/')"
  if [ -z "${v}" ]; then
    echo "error: could not parse [workspace.package] version from Cargo.toml" >&2
    return 1
  fi
  echo "v${v}"
}
