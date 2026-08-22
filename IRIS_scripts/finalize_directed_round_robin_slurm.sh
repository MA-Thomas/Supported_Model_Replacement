#!/bin/bash
# Runs after the PR+ROC array succeeds. Full mode audits and reduces both tournaments;
# dual adaptive 70-system bundles also receive both policy-specific induced views.

#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=1

set -euo pipefail

: "${IRIS_RR_RUN_ENV:?Submit through submit_directed_round_robin_slurm.sh}"

# shellcheck disable=SC1090
source "${IRIS_RR_RUN_ENV}"

PDAC_ADAPTIVE_SYSTEMS=(
    full_hla__self_gated_hillq_pdac_selected
    focal_hla__self_gated_hillq_pdac_selected
    old_monoallelic__self_gated_hillq_pdac_selected
    mono_q_full_pn__self_gated_hillq_pdac_selected
    full_q_mono_pn__self_gated_hillq_pdac_selected
)
ALL_CONTEXT_ADAPTIVE_SYSTEMS=(
    full_hla__self_gated_hillq_all_contexts_selected
    focal_hla__self_gated_hillq_all_contexts_selected
    old_monoallelic__self_gated_hillq_all_contexts_selected
    mono_q_full_pn__self_gated_hillq_all_contexts_selected
    full_q_mono_pn__self_gated_hillq_all_contexts_selected
)

has_dual_adaptive_roster() {
    python3 -c '
import json, sys
systems = {row["system_id"] for row in json.load(open(sys.argv[1]))["systems"]}
expected = set(sys.argv[2:])
raise SystemExit(0 if len(systems) == 70 and expected <= systems else 1)
' "${1}/systems.json" "${PDAC_ADAPTIVE_SYSTEMS[@]}" "${ALL_CONTEXT_ADAPTIVE_SYSTEMS[@]}"
}

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
        if has_dual_adaptive_roster "${BUNDLE}"; then
            VIEW_ROOT="${RUN_ROOT}/full/induced_views/${METRIC}"
            mkdir -p "${VIEW_ROOT}"
            "${ORGANIZER}" induce-view \
                --bundle "${BUNDLE}" --plan "${PLAN}" --reduction "${REDUCTION}" \
                --exclude-system "${ALL_CONTEXT_ADAPTIVE_SYSTEMS[@]}" \
                --output "${VIEW_ROOT}/pdac_only"
            "${ORGANIZER}" induce-view \
                --bundle "${BUNDLE}" --plan "${PLAN}" --reduction "${REDUCTION}" \
                --exclude-system "${PDAC_ADAPTIVE_SYSTEMS[@]}" \
                --output "${VIEW_ROOT}/all_contexts_equal_weight"
        fi
    fi
done

echo "Finalized ${MODE} run at $(date -Iseconds)"
echo "Resource report: ${USAGE_OUTPUT}"
