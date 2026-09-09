#!/bin/bash
# One Slurm-array task runs one deterministic adaptive-Hill-q CNAP shard.

#SBATCH --nodes=1
#SBATCH --ntasks=1

set -euo pipefail

: "${IRIS_ADAPTIVE_RUN_ENV:?Submit through submit_adaptive_hillq_selection_slurm.sh}"
: "${SLURM_ARRAY_TASK_ID:?This worker must run as a Slurm array}"
: "${IRIS_ADAPTIVE_SHARD_OFFSET:=0}"

# shellcheck disable=SC1090
source "${IRIS_ADAPTIVE_RUN_ENV}"

if [ "${SLURM_CPUS_PER_TASK:-0}" -ne "${THREADS}" ]; then
    echo "ERROR: SLURM_CPUS_PER_TASK=${SLURM_CPUS_PER_TASK:-unset} but THREADS=${THREADS}" >&2
    exit 1
fi

SHARD_ID=$((IRIS_ADAPTIVE_SHARD_OFFSET + SLURM_ARRAY_TASK_ID))
USAGE_DIR="${RUN_ROOT}/${MODE}/resource_usage"
mkdir -p "${USAGE_DIR}" "${RESULTS}"
TIME_FILE="${USAGE_DIR}/shard_${SHARD_ID}.time.json"
RUN_SUMMARY="${USAGE_DIR}/shard_${SHARD_ID}.run_summary.json"
RESOURCE_SUMMARY="${USAGE_DIR}/shard_${SHARD_ID}.json"
TEMP_SUMMARY="${RUN_SUMMARY}.tmp.${SLURM_JOB_ID}.$$"

export RAYON_NUM_THREADS="${THREADS}"
export OMP_NUM_THREADS=1
export MKL_NUM_THREADS=1
export OPENBLAS_NUM_THREADS=1

echo "Started:             $(date -Iseconds)"
echo "Node:                $(hostname)"
echo "Job/array task:      ${SLURM_ARRAY_JOB_ID:-${SLURM_JOB_ID}}/${SLURM_ARRAY_TASK_ID}"
echo "Adaptive CNAP shard: ${SHARD_ID}"
echo "Selection challenges:${SELECTION_REPLICATIONS}"
echo "Shared Rayon threads:${THREADS}"

set +e
python3 "${MEASURE_HELPER}" run-measured \
    --time-file "${TIME_FILE}" \
    --stdout-file "${TEMP_SUMMARY}" \
    -- \
    "${CLUSTER_BINARY}" run-shard \
        --source-root "${SOURCE_ROOT}" \
        --bundle-root "${BUNDLE_ROOT}" \
        --plan "${PLAN}" \
        --results "${RESULTS}" \
        --shard-id "${SHARD_ID}" \
        --threads "${THREADS}"
RUN_EXIT=$?
set -e

if [ "${RUN_EXIT}" -eq 0 ] && python3 "${ADAPTIVE_HELPER}" check-summary \
    --path "${TEMP_SUMMARY}" --shard-id "${SHARD_ID}"; then
    mv "${TEMP_SUMMARY}" "${RUN_SUMMARY}"
else
    RUN_EXIT=1
fi

python3 "${ADAPTIVE_HELPER}" summarize \
    --mode "${MODE}" --shard-id "${SHARD_ID}" \
    --time-file "${TIME_FILE}" --run-summary "${RUN_SUMMARY}" \
    --results "${RESULTS}" --exit-code "${RUN_EXIT}" \
    --output "${RESOURCE_SUMMARY}"

echo "Finished:            $(date -Iseconds)"
echo "Resource summary:    ${RESOURCE_SUMMARY}"
exit "${RUN_EXIT}"
