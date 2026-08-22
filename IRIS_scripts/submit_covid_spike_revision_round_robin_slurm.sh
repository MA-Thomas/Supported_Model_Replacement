#!/bin/bash
# Submit the COVID-SPIKE-only 0.53 label revision without touching the base run.

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPOSITORY_ROOT=$(cd "${SCRIPT_DIR}/.." && pwd)
HELPER="${SCRIPT_DIR}/slurm_round_robin_helper.py"
WORKER="${SCRIPT_DIR}/run_directed_round_robin_array.slurm"
FINALIZER="${SCRIPT_DIR}/finalize_covid_spike_revision_round_robin_slurm.sh"

MODE=
REVISION_BUNDLE_ROOT=
BASE_RUN_ROOT=
RUN_ROOT=
REVISION_ID=covid_spike_higher_threshold_0p53
REPLACE_EVALUATION=covid_spike
COVID_SPIKE_LABEL_SET_ID=higher_threshold_0p53
ORGANIZER="${REPOSITORY_ROOT}/supported_ap_code/target/release/directed_round_robin_organizer"
THREADS=4
PILOT_MATCHES_PER_SHARD=1
FULL_MATCHES_PER_SHARD=50
MAX_CONCURRENT=16
PARTITION=componc_cpu
ACCOUNT=lukszam
MEMORY=32G
WALLTIME=
PREPARE_ONLY=false
DRY_RUN=false
EXPECTED_MATCHES=1770

usage() {
    cat <<'EOF'
Usage:
  bash submit_covid_spike_revision_round_robin_slurm.sh \
    --mode pilot|full \
    --revision-bundle-root /path/to/covid_spike_higher_threshold_0p53 \
    --base-run-root /path/to/completed/threshold_zero/run \
    --run-root /path/to/new/revision/run [options]

The revision root must contain PR and ROC COVID-SPIKE-only bundles implementing
cd8_IFNg_dmso_adj > 0.53. The base threshold-zero run is read-only and must
contain its completed bundles, full plans, and audited reductions. Full mode
reuses the base NONSPIKE and PDAC verdicts, substitutes only SPIKE, and
recomputes the three-context conjunction and graph.

Options:
  --revision-id ID                Default covid_spike_higher_threshold_0p53
  --organizer PATH                Release organizer binary
  --threads N                     CPUs and shared Rayon threads (default 4)
  --pilot-matches-per-shard N     Pilot size per metric (default 1)
  --full-matches-per-shard N      Full shard target (default 50)
  --max-concurrent N              Slurm array concurrency cap (default 16)
  --partition NAME                Slurm partition (default componc_cpu)
  --account NAME                  Slurm account (default lukszam; use '' to omit)
  --mem SIZE                      Memory per array task (default 32G)
  --time D-HH:MM:SS               Pilot default 04:00:00; full 2-00:00:00
  --prepare-only                  Install/validate bundles and create plans only
  --dry-run                       Print submission command without calling sbatch
EOF
}

positive_integer() {
    [[ "$2" =~ ^[1-9][0-9]*$ ]] || { echo "ERROR: $1 must be a positive integer" >&2; exit 2; }
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --mode) MODE=${2:?}; shift 2 ;;
        --revision-bundle-root) REVISION_BUNDLE_ROOT=${2:?}; shift 2 ;;
        --base-run-root) BASE_RUN_ROOT=${2:?}; shift 2 ;;
        --run-root) RUN_ROOT=${2:?}; shift 2 ;;
        --revision-id) REVISION_ID=${2:?}; shift 2 ;;
        --organizer) ORGANIZER=${2:?}; shift 2 ;;
        --threads) THREADS=${2:?}; shift 2 ;;
        --pilot-matches-per-shard) PILOT_MATCHES_PER_SHARD=${2:?}; shift 2 ;;
        --full-matches-per-shard) FULL_MATCHES_PER_SHARD=${2:?}; shift 2 ;;
        --max-concurrent) MAX_CONCURRENT=${2:?}; shift 2 ;;
        --partition) PARTITION=${2:?}; shift 2 ;;
        --account) ACCOUNT=${2-}; shift 2 ;;
        --mem) MEMORY=${2:?}; shift 2 ;;
        --time) WALLTIME=${2:?}; shift 2 ;;
        --prepare-only) PREPARE_ONLY=true; shift ;;
        --dry-run) DRY_RUN=true; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "ERROR: unknown option $1" >&2; usage >&2; exit 2 ;;
    esac
done

[ "${MODE}" = pilot ] || [ "${MODE}" = full ] || { echo "ERROR: --mode must be pilot or full" >&2; exit 2; }
[ -n "${REVISION_BUNDLE_ROOT}" ] || { echo "ERROR: --revision-bundle-root is required" >&2; exit 2; }
[ -n "${BASE_RUN_ROOT}" ] || { echo "ERROR: --base-run-root is required" >&2; exit 2; }
[ -n "${RUN_ROOT}" ] || { echo "ERROR: --run-root is required" >&2; exit 2; }
positive_integer --threads "${THREADS}"
positive_integer --pilot-matches-per-shard "${PILOT_MATCHES_PER_SHARD}"
positive_integer --full-matches-per-shard "${FULL_MATCHES_PER_SHARD}"
positive_integer --max-concurrent "${MAX_CONCURRENT}"

REVISION_BUNDLE_ROOT=$(cd "${REVISION_BUNDLE_ROOT}" && pwd)
BASE_RUN_ROOT=$(cd "${BASE_RUN_ROOT}" && pwd)
RUN_ROOT=$(mkdir -p "${RUN_ROOT}" && cd "${RUN_ROOT}" && pwd)
ORGANIZER=$(cd "$(dirname "${ORGANIZER}")" && pwd)/$(basename "${ORGANIZER}")
[ -x "${ORGANIZER}" ] || { echo "ERROR: organizer is not executable: ${ORGANIZER}" >&2; exit 1; }
for path in "${HELPER}" "${WORKER}" "${FINALIZER}"; do
    [ -f "${path}" ] || { echo "ERROR: missing wrapper component: ${path}" >&2; exit 1; }
done

mkdir -p "${RUN_ROOT}/bundles" "${RUN_ROOT}/${MODE}/plans" \
    "${RUN_ROOT}/${MODE}/results" "${RUN_ROOT}/${MODE}/resource_usage" \
    "${RUN_ROOT}/${MODE}/logs"

for METRIC in pr roc; do
    SOURCE_BUNDLE="${REVISION_BUNDLE_ROOT}/${METRIC}"
    BUNDLE="${RUN_ROOT}/bundles/${METRIC}"
    [ -d "${SOURCE_BUNDLE}" ] || { echo "ERROR: missing revision bundle: ${SOURCE_BUNDLE}" >&2; exit 1; }
    if [ -d "${BUNDLE}" ]; then
        "${ORGANIZER}" validate-bundle --bundle "${BUNDLE}" >/dev/null
    else
        python3 "${HELPER}" install-prebuilt-bundle --source "${SOURCE_BUNDLE}" --output "${BUNDLE}"
        "${ORGANIZER}" validate-bundle --bundle "${BUNDLE}" >/dev/null
    fi
    python3 "${HELPER}" check-bundle-label --bundle "${BUNDLE}" \
        --label-set "${COVID_SPIKE_LABEL_SET_ID}"
    python3 "${HELPER}" check-revision-bundle --bundle "${BUNDLE}" \
        --revision-id "${REVISION_ID}" --evaluation "${REPLACE_EVALUATION}" \
        --expected-systems 60 --expected-endpoints 1765 --expected-positive 667
    PLAN="${RUN_ROOT}/${MODE}/plans/${METRIC}"
    if [ "${MODE}" = pilot ]; then
        MATCHES_PER_SHARD=${PILOT_MATCHES_PER_SHARD}
    else
        MATCHES_PER_SHARD=${FULL_MATCHES_PER_SHARD}
    fi
    if [ ! -f "${PLAN}/plan.json" ]; then
        "${ORGANIZER}" plan --bundle "${BUNDLE}" --output "${PLAN}" \
            --matches-per-shard "${MATCHES_PER_SHARD}" >/dev/null
    fi
    read -r SHARDS MATCHES < <(
        python3 "${HELPER}" plan-info --plan "${PLAN}" \
            --expected-matches "${EXPECTED_MATCHES}" --matches-per-shard "${MATCHES_PER_SHARD}"
    )
    if [ "${METRIC}" = pr ]; then
        PR_SHARDS=${SHARDS}; PR_MATCHES=${MATCHES}
    else
        ROC_SHARDS=${SHARDS}; ROC_MATCHES=${MATCHES}
    fi

    BASE_BUNDLE="${BASE_RUN_ROOT}/bundles/${METRIC}"
    BASE_PLAN="${BASE_RUN_ROOT}/full/plans/${METRIC}"
    BASE_REDUCTION="${BASE_RUN_ROOT}/full/reductions/${METRIC}"
    [ -d "${BASE_BUNDLE}" ] || { echo "ERROR: missing base bundle: ${BASE_BUNDLE}" >&2; exit 1; }
    [ -f "${BASE_PLAN}/plan.json" ] || { echo "ERROR: missing base plan: ${BASE_PLAN}" >&2; exit 1; }
    if [ "${MODE}" = full ]; then
        [ -d "${BASE_REDUCTION}" ] || {
            echo "ERROR: missing audited base reduction: ${BASE_REDUCTION}" >&2
            exit 1
        }
        "${ORGANIZER}" audit-reduction --bundle "${BASE_BUNDLE}" \
            --plan "${BASE_PLAN}" --reduction "${BASE_REDUCTION}" \
            --threads 1
    fi
done

if [ "${MODE}" = pilot ]; then
    ARRAY_LAST=1
    WALLTIME=${WALLTIME:-04:00:00}
else
    ARRAY_LAST=$((PR_SHARDS + ROC_SHARDS - 1))
    WALLTIME=${WALLTIME:-2-00:00:00}
fi

RUN_ENV="${RUN_ROOT}/${MODE}/run.env"
TEMP_RUN_ENV="${RUN_ENV}.tmp.$$"
umask 077
{
    printf 'MODE=%q\n' "${MODE}"
    printf 'CONTEXT_REVISION_ID=%q\n' "${REVISION_ID}"
    printf 'COVID_SPIKE_LABEL_SET_ID=%q\n' "${COVID_SPIKE_LABEL_SET_ID}"
    printf 'REPLACE_EVALUATION=%q\n' "${REPLACE_EVALUATION}"
    printf 'ORGANIZER=%q\n' "${ORGANIZER}"
    printf 'HELPER=%q\n' "${HELPER}"
    printf 'RUN_ROOT=%q\n' "${RUN_ROOT}"
    printf 'BASE_RUN_ROOT=%q\n' "${BASE_RUN_ROOT}"
    printf 'PR_BUNDLE=%q\n' "${RUN_ROOT}/bundles/pr"
    printf 'ROC_BUNDLE=%q\n' "${RUN_ROOT}/bundles/roc"
    printf 'PR_PLAN=%q\n' "${RUN_ROOT}/${MODE}/plans/pr"
    printf 'ROC_PLAN=%q\n' "${RUN_ROOT}/${MODE}/plans/roc"
    printf 'PR_RESULTS=%q\n' "${RUN_ROOT}/${MODE}/results/pr"
    printf 'ROC_RESULTS=%q\n' "${RUN_ROOT}/${MODE}/results/roc"
    printf 'RESOURCE_ROOT=%q\n' "${RUN_ROOT}/${MODE}/resource_usage"
    printf 'THREADS=%q\n' "${THREADS}"
    printf 'PR_SHARDS=%q\n' "${PR_SHARDS}"
    printf 'ROC_SHARDS=%q\n' "${ROC_SHARDS}"
} > "${TEMP_RUN_ENV}"
if [ -e "${RUN_ENV}" ]; then
    if ! cmp -s "${TEMP_RUN_ENV}" "${RUN_ENV}"; then
        /bin/rm "${TEMP_RUN_ENV}"
        echo "ERROR: existing run environment differs: ${RUN_ENV}" >&2
        exit 1
    fi
    /bin/rm "${TEMP_RUN_ENV}"
else
    mv "${TEMP_RUN_ENV}" "${RUN_ENV}"
fi

echo "Prepared ${MODE} COVID SPIKE label revision"
echo "  Revision: ${REVISION_ID}"
echo "  Label set: ${COVID_SPIKE_LABEL_SET_ID}"
echo "  PR:       ${PR_MATCHES} matches in ${PR_SHARDS} shards"
echo "  ROC:      ${ROC_MATCHES} matches in ${ROC_SHARDS} shards"
echo "  Base run: ${BASE_RUN_ROOT} (read-only)"
echo "  New run:  ${RUN_ROOT}"
[ "${PREPARE_ONLY}" = false ] || exit 0

SBATCH_COMMON=(--partition "${PARTITION}" --nodes 1 --ntasks 1)
[ -z "${ACCOUNT}" ] || SBATCH_COMMON+=(--account "${ACCOUNT}")
ARRAY_COMMAND=(
    sbatch "${SBATCH_COMMON[@]}" --job-name "iris_spike53_rev_${MODE}"
    --array "0-${ARRAY_LAST}%${MAX_CONCURRENT}"
    --cpus-per-task "${THREADS}" --mem "${MEMORY}" --time "${WALLTIME}"
    --output "${RUN_ROOT}/${MODE}/logs/array_%A_%a.out"
    --error "${RUN_ROOT}/${MODE}/logs/array_%A_%a.err"
    --export "ALL,IRIS_RR_RUN_ENV=${RUN_ENV}" --parsable "${WORKER}"
)
if [ "${DRY_RUN}" = true ]; then
    printf 'DRY RUN: '; printf '%q ' "${ARRAY_COMMAND[@]}"; printf '\n'
    exit 0
fi
command -v sbatch >/dev/null || { echo "ERROR: sbatch is not available" >&2; exit 1; }
ARRAY_JOB_ID=$("${ARRAY_COMMAND[@]}")
ARRAY_JOB_ID=${ARRAY_JOB_ID%%;*}
FINAL_JOB_ID=$(sbatch "${SBATCH_COMMON[@]}" --job-name "iris_spike53_rev_${MODE}_finalize" \
    --cpus-per-task "${THREADS}" --mem 8G --time 12:00:00 --dependency "afterok:${ARRAY_JOB_ID}" \
    --output "${RUN_ROOT}/${MODE}/logs/finalize_%j.out" \
    --error "${RUN_ROOT}/${MODE}/logs/finalize_%j.err" \
    --export "ALL,IRIS_RR_RUN_ENV=${RUN_ENV}" --parsable "${FINALIZER}")
FINAL_JOB_ID=${FINAL_JOB_ID%%;*}
echo "Submitted array job: ${ARRAY_JOB_ID}"
echo "Submitted afterok finalizer: ${FINAL_JOB_ID}"
