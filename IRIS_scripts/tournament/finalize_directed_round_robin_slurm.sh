#!/bin/bash
# Runs after the PR+ROC array succeeds. Full mode audits and reduces both
# immutable 65-system tournaments.

#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=1

set -euo pipefail

: "${IRIS_RR_RUN_ENV:?Submit through submit_directed_round_robin_slurm.sh}"

# shellcheck disable=SC1090
source "${IRIS_RR_RUN_ENV}"

USAGE_OUTPUT="${RUN_ROOT}/${MODE}/resource_report"
python3 "${HELPER}" aggregate-usage \
    --usage-root "${RESOURCE_ROOT}" \
    --output "${USAGE_OUTPUT}"

for METRIC in pr roc; do
    if [ "${METRIC}" = pr ]; then
        BUNDLE=${PR_BUNDLE}; PLAN=${PR_PLAN}; RESULTS=${PR_RESULTS}
    else
        BUNDLE=${ROC_BUNDLE}; PLAN=${ROC_PLAN}; RESULTS=${ROC_RESULTS}
    fi
    mkdir -p "${RUN_ROOT}/${MODE}/status"
    "${ORGANIZER}" status --plan "${PLAN}" --results "${RESULTS}" \
        > "${RUN_ROOT}/${MODE}/status/${METRIC}.json"

    if [ "${MODE}" = full ]; then
        REDUCTION="${RUN_ROOT}/full/reductions/${METRIC}"
        "${ORGANIZER}" audit \
            --bundle "${BUNDLE}" --plan "${PLAN}" --results "${RESULTS}"
        "${ORGANIZER}" reduce \
            --bundle "${BUNDLE}" --plan "${PLAN}" --results "${RESULTS}" \
            --output "${REDUCTION}"
        "${ORGANIZER}" audit \
            --bundle "${BUNDLE}" --plan "${PLAN}" --results "${RESULTS}" \
            --reduction "${REDUCTION}"
    fi
done

echo "Finalized ${MODE} run at $(date -Iseconds)"
echo "Resource report: ${USAGE_OUTPUT}"
