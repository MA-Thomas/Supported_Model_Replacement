#!/bin/bash
# Audits, graph-reduces, and applies Version 2 after all shards succeed.
#SBATCH --nodes=1
#SBATCH --ntasks=1

set -euo pipefail

: "${NCI_PARAMETER_RUN_ENV:?Submit through submit_nci_parameter_tournaments_slurm.sh}"
# shellcheck disable=SC1090
source "${NCI_PARAMETER_RUN_ENV}"

for MODEL in full_hla focal_hla old_monoallelic mono_q_full_pn full_q_mono_pn; do
    for METRIC in pr roc; do
        BRANCH_ROOT="${PACKAGE_ROOT}/${MODEL}/${METRIC}"
        BUNDLE="${BRANCH_ROOT}/bundle"
        PLAN="${RUN_ROOT}/plans/${MODEL}/${METRIC}"
        RESULTS="${RUN_ROOT}/results/${MODEL}/${METRIC}"
        REDUCTION="${RUN_ROOT}/reductions/${MODEL}/${METRIC}"
        VERSION2="${RUN_ROOT}/version2/${MODEL}/${METRIC}"

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
