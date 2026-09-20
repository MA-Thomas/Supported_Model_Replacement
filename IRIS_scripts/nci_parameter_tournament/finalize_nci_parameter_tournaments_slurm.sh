#!/bin/bash
# Audits, graph-reduces, and applies Version 2 after all shards succeed.
#SBATCH --nodes=1
#SBATCH --ntasks=1

set -euo pipefail

: "${NCI_PARAMETER_RUN_ENV:?Submit through submit_nci_parameter_tournaments_slurm.sh}"
EXECUTION_MODE=exhaustive
# shellcheck disable=SC1090
source "${NCI_PARAMETER_RUN_ENV}"
case "${EXECUTION_MODE}" in
    exhaustive|accelerated) ;;
    *) echo "ERROR: unknown execution mode" >&2; exit 1 ;;
esac
"${PIPELINE}" audit-prepared --package "${PACKAGE_ROOT}" --config "${CONFIG}"

for MODEL in full_hla focal_hla old_monoallelic mono_q_full_pn full_q_mono_pn; do
    for METRIC in pr roc; do
        BRANCH_ROOT="${PACKAGE_ROOT}/${MODEL}/${METRIC}"
        BUNDLE="${BRANCH_ROOT}/bundle"
        PLAN="${RUN_ROOT}/plans/${MODEL}/${METRIC}"
        RESULTS="${RUN_ROOT}/results/${MODEL}/${METRIC}"
        REDUCTION="${RUN_ROOT}/reductions/${MODEL}/${METRIC}"
        VERSION2="${RUN_ROOT}/version2/${MODEL}/${METRIC}"

        if [ "${EXECUTION_MODE}" = accelerated ]; then
            SELECTION="${RUN_ROOT}/selections/${MODEL}/${METRIC}"
            "${ORGANIZER}" accelerated audit --bundle "${BUNDLE}" --plan "${PLAN}" \
                --results "${RESULTS}" --selection "${SELECTION}"
            if [ -d "${VERSION2}" ]; then
                "${PIPELINE}" audit-version2 --output "${VERSION2}"
            else
                mkdir -p "$(dirname "${VERSION2}")"
                "${PIPELINE}" finalize-version2-accelerated \
                    --config "${CONFIG}" --bundle "${BUNDLE}" --plan "${PLAN}" \
                    --results "${RESULTS}" --selection "${SELECTION}" \
                    --output "${VERSION2}" --threads "${THREADS}"
            fi
            continue
        fi

        "${ORGANIZER}" audit \
            --bundle "${BUNDLE}" --plan "${PLAN}" --results "${RESULTS}" \
            --threads "${THREADS}"
        if [ -d "${REDUCTION}" ]; then
            "${ORGANIZER}" audit-reduction \
                --bundle "${BUNDLE}" --plan "${PLAN}" --reduction "${REDUCTION}" \
                --threads "${THREADS}"
        else
            mkdir -p "$(dirname "${REDUCTION}")"
            "${ORGANIZER}" reduce \
                --bundle "${BUNDLE}" --plan "${PLAN}" --results "${RESULTS}" \
                --output "${REDUCTION}" --threads "${THREADS}"
        fi
        if [ -d "${VERSION2}" ]; then
            "${PIPELINE}" audit-version2 --output "${VERSION2}"
        else
            mkdir -p "$(dirname "${VERSION2}")"
            "${PIPELINE}" finalize-version2 \
                --config "${CONFIG}" --bundle "${BUNDLE}" --plan "${PLAN}" \
                --results "${RESULTS}" --reduction "${REDUCTION}" \
                --output "${VERSION2}" --threads "${THREADS}"
        fi
    done
done

echo "All ten NCI parameter tournaments finalized: ${RUN_ROOT}/version2"
