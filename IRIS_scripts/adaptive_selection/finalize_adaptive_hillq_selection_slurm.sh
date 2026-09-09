#!/bin/bash
# Audit adaptive-Hill-q shards and, for a full run, reduce to final selections.

#SBATCH --nodes=1
#SBATCH --ntasks=1

set -euo pipefail

: "${IRIS_ADAPTIVE_RUN_ENV:?Submit through submit_adaptive_hillq_selection_slurm.sh}"

# shellcheck disable=SC1090
source "${IRIS_ADAPTIVE_RUN_ENV}"
read -r -a ALPHA_VALUE_ARGS <<< "${ALPHA_VALUES}"
read -r -a Q_VALUE_ARGS <<< "${Q_VALUES}"

mkdir -p "${RUN_ROOT}/${MODE}/status"
"${CLUSTER_BINARY}" status --plan "${PLAN}" --results "${RESULTS}" \
    > "${RUN_ROOT}/${MODE}/status/status.json"
python3 "${ADAPTIVE_HELPER}" aggregate \
    --usage-root "${RUN_ROOT}/${MODE}/resource_usage" \
    --output "${RUN_ROOT}/${MODE}/resource_report"

if [ "${MODE}" = full ]; then
    "${CLUSTER_BINARY}" audit \
        --source-root "${SOURCE_ROOT}" --bundle-root "${BUNDLE_ROOT}" \
        --plan "${PLAN}" --results "${RESULTS}" \
        > "${RUN_ROOT}/${MODE}/status/audit.json"
    "${SELECTOR_BINARY}" \
        --source-root "${SOURCE_ROOT}" \
        --bundle-root "${BUNDLE_ROOT}" \
        --alpha-values "${ALPHA_VALUE_ARGS[@]}" \
        --q-values "${Q_VALUE_ARGS[@]}" \
        --c-min "${C_MIN}" --c-max "${C_MAX}" --c-step "${C_STEP}" \
        --kappa-min "${KAPPA_MIN}" --kappa-max "${KAPPA_MAX}" \
        --kappa-points "${KAPPA_POINTS}" \
        --output "${FINAL_OUTPUT}" \
        --selection-replications "${SELECTION_REPLICATIONS}" \
        --cnap-plan "${PLAN}" \
        --cnap-results "${RESULTS}" \
        --threads "${THREADS}"
fi

echo "Finalized ${MODE} adaptive-Hill-q run at $(date -Iseconds)"
