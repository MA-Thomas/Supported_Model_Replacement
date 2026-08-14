#!/bin/bash
# Prepare immutable PR/ROC bundles and submit pilot or full Slurm arrays.

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPOSITORY_ROOT=$(cd "${SCRIPT_DIR}/.." && pwd)
PROVIDER="${SCRIPT_DIR}/iris_score_provider.py"
HELPER="${SCRIPT_DIR}/slurm_round_robin_helper.py"
WORKER="${SCRIPT_DIR}/run_directed_round_robin_array.slurm"
FINALIZER="${SCRIPT_DIR}/finalize_directed_round_robin_slurm.sh"

MODE=
CONFIG=
RUN_ROOT=
COVID_SPIKE_LABEL_SET_ID=
ORGANIZER="${REPOSITORY_ROOT}/supported_ap_code/target/release/directed_round_robin_organizer"
THREADS=4
PILOT_MATCHES_PER_SHARD=3
FULL_MATCHES_PER_SHARD=50
MAX_CONCURRENT=16
PARTITION=componc_cpu
ACCOUNT=lukszam
MEMORY=32G
WALLTIME=
PREPARE_ONLY=false
DRY_RUN=false
EXPECTED_MATCHES=5310

usage() {
    cat <<'EOF'
Usage:
  bash submit_directed_round_robin_slurm.sh \
    --mode pilot|full \
    --config /path/to/config.json \
    --run-root /path/to/output_root \
    --covid-spike-label-set LABEL_SET_ID \
    [options]

Required scientific choice:
  --covid-spike-label-set must exactly match config.json's
  covid_spike_label_specification.id. It selects the SPIKE definition only.
  COVID NONSPIKE remains fixed at cd8_TNFa_IFNg_dmso_adj > 0.

Modes:
  pilot  Runs shard 0 for PR and shard 0 for ROC. Each contains
         --pilot-matches-per-shard matches (default 3: one per evaluation).
         Records elapsed time, maximum RSS, and exact match-artifact bytes.
  full   Runs every PR and ROC shard and requires exactly 5,310 matches in
         each tournament. After success, audits and reduces both tournaments.

Options:
  --organizer PATH                 Release organizer binary
  --threads N                      Match workers and Slurm CPUs per task (default 4)
  --pilot-matches-per-shard N      Pilot shard size (default 3)
  --full-matches-per-shard N       Full shard target (default 50)
  --max-concurrent N               Slurm array concurrency cap (default 16)
  --partition NAME                 Slurm partition (default componc_cpu)
  --account NAME                   Slurm account (default lukszam; use '' to omit)
  --mem SIZE                       Memory per array task (default 32G)
  --time D-HH:MM:SS                Wall time (pilot default 04:00:00; full 2-00:00:00)
  --prepare-only                   Build/validate bundles and plans without sbatch
  --dry-run                        Print sbatch commands after preparing inputs
  -h, --help                       Show this help

Examples:
  bash submit_directed_round_robin_slurm.sh --mode pilot \
    --config config.threshold_zero.json --run-root /data1/.../round_robin \
    --covid-spike-label-set threshold_zero

  bash submit_directed_round_robin_slurm.sh --mode full \
    --config config.higher_threshold.json --run-root /data1/.../round_robin_high \
    --covid-spike-label-set higher_threshold_0p53 --threads 8
EOF
}

positive_integer() {
    [[ "$2" =~ ^[1-9][0-9]*$ ]] || { echo "ERROR: $1 must be a positive integer" >&2; exit 2; }
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --mode) MODE=${2:?}; shift 2 ;;
        --config) CONFIG=${2:?}; shift 2 ;;
        --run-root) RUN_ROOT=${2:?}; shift 2 ;;
        --covid-spike-label-set) COVID_SPIKE_LABEL_SET_ID=${2:?}; shift 2 ;;
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
[ -n "${CONFIG}" ] || { echo "ERROR: --config is required" >&2; exit 2; }
[ -n "${RUN_ROOT}" ] || { echo "ERROR: --run-root is required" >&2; exit 2; }
[ -n "${COVID_SPIKE_LABEL_SET_ID}" ] || { echo "ERROR: --covid-spike-label-set is required" >&2; exit 2; }
positive_integer --threads "${THREADS}"
positive_integer --pilot-matches-per-shard "${PILOT_MATCHES_PER_SHARD}"
positive_integer --full-matches-per-shard "${FULL_MATCHES_PER_SHARD}"
positive_integer --max-concurrent "${MAX_CONCURRENT}"

CONFIG=$(cd "$(dirname "${CONFIG}")" && pwd)/$(basename "${CONFIG}")
RUN_ROOT=$(mkdir -p "${RUN_ROOT}" && cd "${RUN_ROOT}" && pwd)
ORGANIZER=$(cd "$(dirname "${ORGANIZER}")" && pwd)/$(basename "${ORGANIZER}")

[ -f "${CONFIG}" ] || { echo "ERROR: config not found: ${CONFIG}" >&2; exit 1; }
[ -x "${ORGANIZER}" ] || {
    echo "ERROR: release organizer not executable: ${ORGANIZER}" >&2
    echo "Build it with: cargo build --release --manifest-path ${REPOSITORY_ROOT}/supported_ap_code/Cargo.toml --package directed_round_robin_organizer --bin directed_round_robin_organizer" >&2
    exit 1
}
for path in "${PROVIDER}" "${HELPER}" "${WORKER}" "${FINALIZER}"; do
    [ -f "${path}" ] || { echo "ERROR: missing wrapper component: ${path}" >&2; exit 1; }
done

EFFECTIVE_CONFIG="${RUN_ROOT}/effective_config.${COVID_SPIKE_LABEL_SET_ID}.json"
python3 "${HELPER}" prepare-config \
    --source "${CONFIG}" --output "${EFFECTIVE_CONFIG}" \
    --organizer "${ORGANIZER}" --label-set "${COVID_SPIKE_LABEL_SET_ID}"

mkdir -p "${RUN_ROOT}/bundles" "${RUN_ROOT}/${MODE}/plans" \
    "${RUN_ROOT}/${MODE}/results" "${RUN_ROOT}/${MODE}/resource_usage" \
    "${RUN_ROOT}/${MODE}/logs"

for METRIC in pr roc; do
    BUNDLE="${RUN_ROOT}/bundles/${METRIC}"
    if [ -d "${BUNDLE}" ]; then
        "${ORGANIZER}" validate-bundle --bundle "${BUNDLE}" >/dev/null
        python3 "${HELPER}" check-bundle-label \
            --bundle "${BUNDLE}" --label-set "${COVID_SPIKE_LABEL_SET_ID}"
    else
        python3 "${PROVIDER}" build \
            --config "${EFFECTIVE_CONFIG}" --metric "${METRIC}" --output "${BUNDLE}"
    fi

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
            --expected-matches "${EXPECTED_MATCHES}" \
            --matches-per-shard "${MATCHES_PER_SHARD}"
    )
    if [ "${METRIC}" = pr ]; then
        PR_SHARDS=${SHARDS}; PR_MATCHES=${MATCHES}
    else
        ROC_SHARDS=${SHARDS}; ROC_MATCHES=${MATCHES}
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
    printf 'COVID_SPIKE_LABEL_SET_ID=%q\n' "${COVID_SPIKE_LABEL_SET_ID}"
    printf 'ORGANIZER=%q\n' "${ORGANIZER}"
    printf 'HELPER=%q\n' "${HELPER}"
    printf 'RUN_ROOT=%q\n' "${RUN_ROOT}"
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
        echo "Choose a new --run-root/mode or reuse its original settings." >&2
        exit 1
    fi
    /bin/rm "${TEMP_RUN_ENV}"
else
    mv "${TEMP_RUN_ENV}" "${RUN_ENV}"
fi

echo "Prepared ${MODE} run"
echo "  SPIKE label set: ${COVID_SPIKE_LABEL_SET_ID}"
echo "  NONSPIKE label:  cd8_TNFa_IFNg_dmso_adj > 0 (fixed)"
echo "  PR:              ${PR_MATCHES} matches in ${PR_SHARDS} shards"
echo "  ROC:             ${ROC_MATCHES} matches in ${ROC_SHARDS} shards"
echo "  Array:           0-${ARRAY_LAST}%${MAX_CONCURRENT}"
echo "  Threads/task:    ${THREADS}"
echo "  Run root:        ${RUN_ROOT}"

if [ "${PREPARE_ONLY}" = true ]; then
    exit 0
fi

SBATCH_COMMON=(--partition "${PARTITION}" --nodes 1 --ntasks 1)
if [ -n "${ACCOUNT}" ]; then
    SBATCH_COMMON+=(--account "${ACCOUNT}")
fi
ARRAY_COMMAND=(
    sbatch "${SBATCH_COMMON[@]}"
    --job-name "iris_rr_${MODE}"
    --array "0-${ARRAY_LAST}%${MAX_CONCURRENT}"
    --cpus-per-task "${THREADS}" --mem "${MEMORY}" --time "${WALLTIME}"
    --output "${RUN_ROOT}/${MODE}/logs/array_%A_%a.out"
    --error "${RUN_ROOT}/${MODE}/logs/array_%A_%a.err"
    --export "ALL,IRIS_RR_RUN_ENV=${RUN_ENV}"
    --parsable "${WORKER}"
)

if [ "${DRY_RUN}" = true ]; then
    printf 'DRY RUN: '; printf '%q ' "${ARRAY_COMMAND[@]}"; printf '\n'
    echo "The finalizer is submitted with afterok:<array_job_id>."
    exit 0
fi

command -v sbatch >/dev/null || { echo "ERROR: sbatch is not available" >&2; exit 1; }
ARRAY_JOB_ID=$("${ARRAY_COMMAND[@]}")
ARRAY_JOB_ID=${ARRAY_JOB_ID%%;*}
FINAL_JOB_ID=$(sbatch "${SBATCH_COMMON[@]}" \
    --job-name "iris_rr_${MODE}_finalize" \
    --cpus-per-task 1 --mem 8G --time 04:00:00 \
    --dependency "afterok:${ARRAY_JOB_ID}" \
    --output "${RUN_ROOT}/${MODE}/logs/finalize_%j.out" \
    --error "${RUN_ROOT}/${MODE}/logs/finalize_%j.err" \
    --export "ALL,IRIS_RR_RUN_ENV=${RUN_ENV}" \
    --parsable "${FINALIZER}")
FINAL_JOB_ID=${FINAL_JOB_ID%%;*}

echo "Submitted array job: ${ARRAY_JOB_ID}"
echo "Submitted afterok finalizer: ${FINAL_JOB_ID}"
