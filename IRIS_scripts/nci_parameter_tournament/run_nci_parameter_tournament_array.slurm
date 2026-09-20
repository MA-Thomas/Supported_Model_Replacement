#!/bin/bash
# One array task executes one immutable shard or one accelerated coordinator.
#SBATCH --nodes=1
#SBATCH --ntasks=1

set -euo pipefail

: "${NCI_PARAMETER_RUN_ENV:?Submit through submit_nci_parameter_tournaments_slurm.sh}"
: "${SLURM_ARRAY_TASK_ID:?This worker must run as a Slurm array}"

EXECUTION_MODE=exhaustive
# shellcheck disable=SC1090
source "${NCI_PARAMETER_RUN_ENV}"

IFS=$'\t' read -r MODEL METRIC SHARD_ID < <(
    awk -F '\t' -v task="${SLURM_ARRAY_TASK_ID}" '$1 == task { print $2 "\t" $3 "\t" $4 }' "${TASK_MAP}"
)
[ -n "${SHARD_ID:-}" ] || { echo "ERROR: no work mapped to task ${SLURM_ARRAY_TASK_ID}" >&2; exit 1; }

BRANCH_ROOT="${PACKAGE_ROOT}/${MODEL}/${METRIC}"
BUNDLE="${BRANCH_ROOT}/bundle"
PLAN="${RUN_ROOT}/plans/${MODEL}/${METRIC}"
RESULTS="${RUN_ROOT}/results/${MODEL}/${METRIC}"
mkdir -p "${RESULTS}"

export RAYON_NUM_THREADS="${THREADS}"
export OMP_NUM_THREADS=1
export MKL_NUM_THREADS=1
export OPENBLAS_NUM_THREADS=1

if [ "${EXECUTION_MODE}" = accelerated ]; then
    [ "${SHARD_ID}" = coordinator ] || { echo "ERROR: accelerated task must be a coordinator" >&2; exit 1; }
    SELECTION="${RUN_ROOT}/selections/${MODEL}/${METRIC}"
    echo "NCI parameter tournament: ${MODEL}/${METRIC}, accelerated coordinator"
    if [ -d "${SELECTION}" ]; then
        "${ORGANIZER}" accelerated audit --bundle "${BUNDLE}" --plan "${PLAN}" \
            --results "${RESULTS}" --selection "${SELECTION}"
    else
        mkdir -p "$(dirname "${SELECTION}")"
        "${ORGANIZER}" accelerated run --bundle "${BUNDLE}" --plan "${PLAN}" \
            --results "${RESULTS}" --output "${SELECTION}" --threads "${THREADS}"
    fi
    exit 0
fi
[ "${EXECUTION_MODE}" = exhaustive ] || { echo "ERROR: unknown execution mode" >&2; exit 1; }

echo "NCI parameter tournament: ${MODEL}/${METRIC}, shard ${SHARD_ID}"
"${ORGANIZER}" run-shard \
    --bundle "${BUNDLE}" \
    --plan "${PLAN}" \
    --shard-id "${SHARD_ID}" \
    --results "${RESULTS}" \
    --threads "${THREADS}"
