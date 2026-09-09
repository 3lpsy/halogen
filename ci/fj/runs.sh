#!/usr/bin/env bash
# fj-help: runs <workflow.yml> [job] — recent runs: run/status/job/event/sha
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
fj_flags "$@"; set -- "${FJ_ARGS[@]+"${FJ_ARGS[@]}"}"

workflow="${1:-}"; job="${2:-}"
fj_require_workflow "${workflow}"

# WORKFLOW is the file name (e.g. test-e2e.yml), NOT the `name:` inside it.
rows="$(fj_api GET 'actions/tasks' | jq --arg w "${workflow}" --arg j "${job}" '
    .workflow_runs
    | map(select(.workflow_id == $w and ($j == "" or .name == $j)))
    | .[:20]')"
if [ "${FJ_JSON}" = 1 ]; then
  printf '%s\n' "${rows}"
else
  jq -r '.[] | [.run_number, .status, .name, .event, .head_sha[0:8]] | @tsv' <<<"${rows}"
fi
