#!/bin/bash
# Reduce higher-threshold SPIKE results and compose with audited base contexts.

#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=8
#SBATCH --time=12:00:00

set -euo pipefail
: "${IRIS_RR_RUN_ENV:?Submit through submit_covid_spike_revision_round_robin_slurm.sh}"
# shellcheck disable=SC1090
source "${IRIS_RR_RUN_ENV}"

python3 "${HELPER}" aggregate-usage --usage-root "${RESOURCE_ROOT}" \
    --output "${RUN_ROOT}/${MODE}/resource_report"

for METRIC in pr roc; do
    if [ "${METRIC}" = pr ]; then
        BUNDLE=${PR_BUNDLE}; PLAN=${PR_PLAN}; RESULTS=${PR_RESULTS}
    else
        BUNDLE=${ROC_BUNDLE}; PLAN=${ROC_PLAN}; RESULTS=${ROC_RESULTS}
    fi
    mkdir -p "${RUN_ROOT}/${MODE}/status"
    if [ "${MODE}" != full ]; then
        "${ORGANIZER}" status --plan "${PLAN}" --results "${RESULTS}" \
            --threads "${THREADS}" > "${RUN_ROOT}/${MODE}/status/${METRIC}.json"
        continue
    fi

    REVISION_REDUCTION="${RUN_ROOT}/full/reductions/${METRIC}"
    if [ -d "${REVISION_REDUCTION}" ]; then
        "${ORGANIZER}" audit-reduction --bundle "${BUNDLE}" --plan "${PLAN}" \
            --reduction "${REVISION_REDUCTION}" --threads "${THREADS}"
    else
        "${ORGANIZER}" reduce --bundle "${BUNDLE}" --plan "${PLAN}" \
            --results "${RESULTS}" --output "${REVISION_REDUCTION}" \
            --threads "${THREADS}"
    fi
    cp "${REVISION_REDUCTION}/completeness_audit.json" \
        "${RUN_ROOT}/${MODE}/status/${METRIC}.json"

    BASE_BUNDLE="${BASE_RUN_ROOT}/bundles/${METRIC}"
    BASE_PLAN="${BASE_RUN_ROOT}/full/plans/${METRIC}"
    BASE_REDUCTION="${BASE_RUN_ROOT}/full/reductions/${METRIC}"
    COMPOSITION="${RUN_ROOT}/full/compositions/${METRIC}"
    [ -d "${BASE_REDUCTION}" ] || {
        echo "ERROR: missing audited base reduction: ${BASE_REDUCTION}" >&2
        exit 1
    }
    "${ORGANIZER}" audit-reduction --bundle "${BASE_BUNDLE}" \
        --plan "${BASE_PLAN}" --reduction "${BASE_REDUCTION}" \
        --threads "${THREADS}"
    if [ -d "${COMPOSITION}" ]; then
        "${ORGANIZER}" audit-revision --base-plan "${BASE_PLAN}" \
            --revision-plan "${PLAN}" --replace-evaluation "${REPLACE_EVALUATION}" \
            --revision-id "${CONTEXT_REVISION_ID}" --composition "${COMPOSITION}"
    else
        "${ORGANIZER}" compose-revision-from-reductions \
            --base-bundle "${BASE_BUNDLE}" --base-plan "${BASE_PLAN}" \
            --base-reduction "${BASE_REDUCTION}" --revision-bundle "${BUNDLE}" \
            --revision-plan "${PLAN}" --revision-reduction "${REVISION_REDUCTION}" \
            --replace-evaluation "${REPLACE_EVALUATION}" \
            --revision-id "${CONTEXT_REVISION_ID}" --output "${COMPOSITION}" \
            --threads "${THREADS}"
        "${ORGANIZER}" audit-revision --base-plan "${BASE_PLAN}" \
            --revision-plan "${PLAN}" --replace-evaluation "${REPLACE_EVALUATION}" \
            --revision-id "${CONTEXT_REVISION_ID}" --composition "${COMPOSITION}"
    fi
done

echo "Finalized ${MODE} revision ${CONTEXT_REVISION_ID} at $(date -Iseconds)"
