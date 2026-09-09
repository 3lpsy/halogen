#!/usr/bin/env bash
# Forgejo API toolkit: one entrypoint that routes to ci/fj/<command>.sh.
# `ci/fj/fj.sh help` lists commands; host/repo/token resolve at runtime (lib.sh).
set -euo pipefail

FJ_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Leading global flags are also accepted before the subcommand.
while [ "${1:-}" = --json ] || [ "${1:-}" = --dry-run ]; do
  if [ "$1" = --json ]; then export FJ_JSON=1; else export FJ_DRY_RUN=1; fi
  shift
done
cmd="${1:-help}"
[ $# -gt 0 ] && shift

if [ "${cmd}" = help ] || [ "${cmd}" = -h ] || [ "${cmd}" = --help ]; then
  echo "usage: ci/fj/fj.sh <command> [--json] [--dry-run] [args]"
  echo
  for f in "${FJ_DIR}"/*.sh; do
    name="$(basename "${f}" .sh)"
    [[ "${name}" = lib || "${name}" = fj ]] && continue
    printf '  %-16s %s\n' "${name}" "$(sed -n 's/^# fj-help: //p' "${f}" | head -1)"
  done
  exit 0
fi

[[ "${cmd}" =~ ^[a-z][a-z-]*$ ]] || { echo "error: bad command name" >&2; exit 2; }
script="${FJ_DIR}/${cmd}.sh"
[ -f "${script}" ] || { echo "error: unknown command '${cmd}' — see ci/fj/fj.sh help" >&2; exit 2; }
exec bash "${script}" "$@"
