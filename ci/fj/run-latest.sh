#!/usr/bin/env bash
# fj-help: run-latest <workflow.yml> — highest run number (the pre-dispatch watermark)
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
fj_flags "$@"; set -- "${FJ_ARGS[@]+"${FJ_ARGS[@]}"}"

workflow="${1:-}"
fj_require_workflow "${workflow}"

fj_api GET 'actions/tasks' | jq -r --arg w "${workflow}" '
    [.workflow_runs[] | select(.workflow_id == $w) | .run_number] | max // 0'
