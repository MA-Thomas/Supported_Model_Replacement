#!/bin/bash
# Merge validated complete-F P/N peptide shards into the canonical task output.

set -euo pipefail

: "${SLURM_ARRAY_TASK_ID:?SLURM_ARRAY_TASK_ID must be set by Slurm}"
: "${PN_SHARD_PLAN:?PN_SHARD_PLAN must be exported by submit_stage2_new.sh}"
: "${OUTPUTS_ROOT:?OUTPUTS_ROOT must be exported by submit_stage2_new.sh}"
: "${MODE_FINGERPRINT:?MODE_FINGERPRINT must be exported by submit_stage2_new.sh}"
: "${RUN_ID:?RUN_ID must be exported by submit_stage2_new.sh}"
: "${QUERY_INPUT_FILE:?QUERY_INPUT_FILE must be exported by submit_stage2_new.sh}"
: "${PARAM_FILE:?PARAM_FILE must be exported by submit_stage2_new.sh}"
: "${MN_TUPLES_FILE:?MN_TUPLES_FILE must be exported by submit_stage2_new.sh}"
: "${CONTRACT_HELPER:?CONTRACT_HELPER must be exported by submit_stage2_new.sh}"
: "${PYTHON_BIN:?PYTHON_BIN must be exported by submit_stage2_new.sh}"

PARENT_TASK_ID="${SLURM_ARRAY_TASK_ID}"

IFS=$'\t' read -r _ _ _ _ ENV_START ENV_STOP _ < <(
    "${PYTHON_BIN}" "${CONTRACT_HELPER}" pn-shard-parent \
        --plan "${PN_SHARD_PLAN}" \
        --parent-task-id "${PARENT_TASK_ID}"
)

DONE_FILE="${OUTPUTS_ROOT}/results_pn_env_id_${ENV_START}_${ENV_STOP}/TASK_${PARENT_TASK_ID}.done.json"
if "${PYTHON_BIN}" "${CONTRACT_HELPER}" task-complete \
    --manifest "${DONE_FILE}" \
    --fingerprint "${MODE_FINGERPRINT}" \
    --run-root "${OUTPUTS_ROOT}" >/dev/null 2>&1; then
    echo "Validated matching parent completion manifest -> skipping: ${DONE_FILE}"
    exit 0
fi

"${PYTHON_BIN}" "${CONTRACT_HELPER}" merge-pn-shards \
    --plan "${PN_SHARD_PLAN}" \
    --parent-task-id "${PARENT_TASK_ID}" \
    --run-id "${RUN_ID}" \
    --fingerprint "${MODE_FINGERPRINT}" \
    --query "${QUERY_INPUT_FILE}" \
    --run-root "${OUTPUTS_ROOT}" \
    --param-file "${PARAM_FILE}" \
    --mn-tuples "${MN_TUPLES_FILE}"

echo "Validated, merged, and published parent P/N task: ${DONE_FILE}"
