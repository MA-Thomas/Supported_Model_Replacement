#!/bin/bash
#SBATCH --nodes=1
#SBATCH --ntasks=1
set -euo pipefail
: "${NCI_CLUSTER_DRIVER:?Missing frozen driver}"
: "${NCI_CLUSTER_RUN:?Missing run directory}"
: "${NCI_CLUSTER_STAGE:?Missing stage}"
exec python3 "${NCI_CLUSTER_DRIVER}" stage --run-root "${NCI_CLUSTER_RUN}" \
    --stage "${NCI_CLUSTER_STAGE}" --task "${SLURM_ARRAY_TASK_ID:-0}"
