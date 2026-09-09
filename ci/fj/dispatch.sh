#!/usr/bin/env bash
# fj-help: dispatch <workflow.yml> [ref] [json-inputs] — fire a workflow_dispatch run
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
fj_flags "$@"; set -- "${FJ_ARGS[@]+"${FJ_ARGS[@]}"}"

workflow="${1:-}"; ref="${2:-master}"; inputs="${3:-{\}}"
fj_require_workflow "${workflow}"
fj_require_ref "${ref}"
jq -e 'type == "object"' >/dev/null 2>&1 <<<"${inputs}" \
  || fj_usage "inputs must be a JSON object of string values"

body="$(jq -nc --arg r "${ref}" --argjson i "${inputs}" '{ref: $r, inputs: $i}')"
if [ "${FJ_DRY_RUN}" = 1 ]; then
  fj_api POST "actions/workflows/${workflow}/dispatches" "${body}"
  exit 0
fi
fj_api POST "actions/workflows/${workflow}/dispatches" "${body}" >/dev/null
echo "dispatched ${workflow} on ${ref}"
