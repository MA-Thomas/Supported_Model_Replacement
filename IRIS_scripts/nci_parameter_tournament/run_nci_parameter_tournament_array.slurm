#!/bin/bash
# One array task executes one immutable organizer shard.
#SBATCH --nodes=1
#SBATCH --ntasks=1

set -euo pipefail

: "${NCI_PARAMETER_RUN_ENV:?Submit through submit_nci_parameter_tournaments_slurm.sh}"
: "${SLURM_ARRAY_TASK_ID:?This worker must run as a Slurm array}"

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

echo "NCI parameter tournament: ${MODEL}/${METRIC}, shard ${SHARD_ID}"
"${ORGANIZER}" run-shard \
    --bundle "${BUNDLE}" \
    --plan "${PLAN}" \
    --shard-id "${SHARD_ID}" \
    --results "${RESULTS}" \
    --threads "${THREADS}"
