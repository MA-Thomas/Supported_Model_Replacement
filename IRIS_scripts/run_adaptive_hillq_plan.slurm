#!/bin/bash
# Compute the immutable adaptive-Hill-q ranking plan on a Slurm compute node.

#SBATCH --nodes=1
#SBATCH --ntasks=1

set -euo pipefail

: "${IRIS_ADAPTIVE_PLAN_ENV:?Submit through submit_adaptive_hillq_selection_slurm.sh}"

# shellcheck disable=SC1090
source "${IRIS_ADAPTIVE_PLAN_ENV}"

if [ "${SLURM_CPUS_PER_TASK:-0}" -ne "${THREADS}" ]; then
    echo "ERROR: SLURM_CPUS_PER_TASK=${SLURM_CPUS_PER_TASK:-unset} but THREADS=${THREADS}" >&2
    exit 1
fi

export RAYON_NUM_THREADS="${THREADS}"
export OMP_NUM_THREADS=1
export MKL_NUM_THREADS=1
export OPENBLAS_NUM_THREADS=1

echo "Started plan:       $(date -Iseconds)"
echo "Node:               $(hostname)"
echo "Shared Rayon threads:${THREADS}"
"${CLUSTER_BINARY}" plan \
    --source-root "${SOURCE_ROOT}" --bundle-root "${BUNDLE_ROOT}" \
    --output "${PLAN}" --selection-replications "${SELECTION_REPLICATIONS}" \
    --matches-per-shard "${MATCHES_PER_SHARD}" --threads "${THREADS}"
echo "Finished plan:      $(date -Iseconds)"
