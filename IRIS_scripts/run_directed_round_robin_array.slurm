#!/bin/bash
# One Slurm-array task runs one deterministic organizer shard.

#SBATCH --nodes=1
#SBATCH --ntasks=1

set -euo pipefail

: "${IRIS_RR_RUN_ENV:?Submit through submit_directed_round_robin_slurm.sh}"
: "${SLURM_ARRAY_TASK_ID:?This worker must run as a Slurm array}"

# shellcheck disable=SC1090
source "${IRIS_RR_RUN_ENV}"

if [ "${SLURM_CPUS_PER_TASK:-0}" -ne "${THREADS}" ]; then
    echo "ERROR: SLURM_CPUS_PER_TASK=${SLURM_CPUS_PER_TASK:-unset} but THREADS=${THREADS}" >&2
    exit 1
fi

case "${MODE}" in
    pilot)
        case "${SLURM_ARRAY_TASK_ID}" in
            0) METRIC=pr; SHARD_ID=0 ;;
            1) METRIC=roc; SHARD_ID=0 ;;
            *) echo "ERROR: pilot array task must be 0 or 1" >&2; exit 1 ;;
        esac
        ;;
    full)
        if [ "${SLURM_ARRAY_TASK_ID}" -lt "${PR_SHARDS}" ]; then
            METRIC=pr
            SHARD_ID=${SLURM_ARRAY_TASK_ID}
        else
            METRIC=roc
            SHARD_ID=$((SLURM_ARRAY_TASK_ID - PR_SHARDS))
            if [ "${SHARD_ID}" -ge "${ROC_SHARDS}" ]; then
                echo "ERROR: array task ${SLURM_ARRAY_TASK_ID} exceeds the PR+ROC shard count" >&2
                exit 1
            fi
        fi
        ;;
    *) echo "ERROR: unknown MODE=${MODE}" >&2; exit 1 ;;
esac

if [ "${METRIC}" = pr ]; then
    BUNDLE=${PR_BUNDLE}
    PLAN=${PR_PLAN}
    RESULTS=${PR_RESULTS}
else
    BUNDLE=${ROC_BUNDLE}
    PLAN=${ROC_PLAN}
    RESULTS=${ROC_RESULTS}
fi

USAGE_DIR="${RESOURCE_ROOT}/${METRIC}"
mkdir -p "${USAGE_DIR}" "${RESULTS}"
TIME_FILE="${USAGE_DIR}/shard_${SHARD_ID}.time.txt"
RUN_SUMMARY="${USAGE_DIR}/shard_${SHARD_ID}.run_summary.json"
RESOURCE_SUMMARY="${USAGE_DIR}/shard_${SHARD_ID}.json"
TEMP_SUMMARY="${RUN_SUMMARY}.tmp.${SLURM_JOB_ID}"

# Match-level parallelism is owned by the organizer's std::thread workers.
# Prevent numerical libraries or incidental Rayon users from nesting pools.
export RAYON_NUM_THREADS=1
export OMP_NUM_THREADS=1
export MKL_NUM_THREADS=1
export OPENBLAS_NUM_THREADS=1

echo "Started:            $(date -Iseconds)"
echo "Node:               $(hostname)"
echo "Job/array task:     ${SLURM_ARRAY_JOB_ID:-${SLURM_JOB_ID}}/${SLURM_ARRAY_TASK_ID}"
echo "Mode:               ${MODE}"
echo "SPIKE label set:    ${COVID_SPIKE_LABEL_SET_ID}"
echo "Metric/shard:       ${METRIC}/${SHARD_ID}"
echo "Organizer threads:  ${THREADS}"
echo "Bundle:             ${BUNDLE}"
echo "Plan:               ${PLAN}"
echo "Results:            ${RESULTS}"

set +e
if /usr/bin/time --version 2>&1 | grep -q GNU; then
    TIME_STYLE=(-v)
else
    TIME_STYLE=(-l)
fi
/usr/bin/time "${TIME_STYLE[@]}" -o "${TIME_FILE}" \
    "${ORGANIZER}" run-shard \
        --bundle "${BUNDLE}" \
        --plan "${PLAN}" \
        --shard-id "${SHARD_ID}" \
        --results "${RESULTS}" \
        --threads "${THREADS}" \
        > "${TEMP_SUMMARY}"
TIME_EXIT=$?
set -e

if python3 "${HELPER}" check-run-summary --path "${TEMP_SUMMARY}"; then
    RUN_EXIT=0
    mv "${TEMP_SUMMARY}" "${RUN_SUMMARY}"
else
    RUN_EXIT=${TIME_EXIT}
fi

python3 "${HELPER}" summarize-shard \
    --mode "${MODE}" \
    --metric "${METRIC}" \
    --shard-id "${SHARD_ID}" \
    --plan "${PLAN}" \
    --results "${RESULTS}" \
    --time-file "${TIME_FILE}" \
    --run-summary "${RUN_SUMMARY}" \
    --exit-code "${RUN_EXIT}" \
    --output "${RESOURCE_SUMMARY}"

echo "Finished:           $(date -Iseconds)"
echo "Resource summary:   ${RESOURCE_SUMMARY}"
exit "${RUN_EXIT}"
