#!/usr/bin/env bash
# fj-help: run-wait <workflow.yml> <job> <watermark> [poll] [tries] — block until a run above the watermark finishes; prints "<run> <status>"
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
fj_flags "$@"; set -- "${FJ_ARGS[@]+"${FJ_ARGS[@]}"}"

workflow="${1:-}"; job="${2:-}"; since="${3:-}"; poll="${4:-30}"; tries="${5:-240}"
fj_require_workflow "${workflow}"
[ -n "${job}" ] || fj_usage "job name is required (workflow AND job pin the run)"
# SINCE feeds jq --argjson; a non-number would error every poll, not once.
[[ "${since}" =~ ^[0-9]+$ && "${poll}" =~ ^[0-9]+$ && "${tries}" =~ ^[0-9]+$ ]] \
  || fj_usage "watermark, poll and tries must be integers"

# Matching on workflow AND job is the point: a job name alone can also match
# another workflow's run and hand back someone else's verdict.
for _ in $(seq 1 "${tries}"); do
  row="$(fj_api GET 'actions/tasks' | jq -r \
    --arg w "${workflow}" --arg j "${job}" --argjson m "${since}" '
      [.workflow_runs[]
       | select(.workflow_id == $w and .name == $j and .run_number > $m)]
      | sort_by(.run_number) | last
      | if . == null then "" else [.run_number, .status] | @tsv end')"
  case "${row}" in
    *success|*failure|*cancelled|*skipped) printf '%s\n' "${row}"; exit 0 ;;
  esac
  [ -n "${row}" ] && echo "waiting: ${row}" >&2 || echo "waiting: no run above ${since} yet" >&2
  sleep "${poll}"
done
echo "error: ${workflow}/${job} did not finish in time" >&2
exit 1
