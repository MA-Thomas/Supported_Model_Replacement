#!/bin/bash
# Plan and submit restartable adaptive-Hill-q paired-CNAP selection arrays.

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPOSITORY_ROOT=$(cd "${SCRIPT_DIR}/.." && pwd)

MODE=
SOURCE_ROOT=
BUNDLE_ROOT=
RUN_ROOT=
SELECTOR_BINARY="${REPOSITORY_ROOT}/supported_ap_code/target/release/select_adaptive_hillq"
CLUSTER_BINARY="${REPOSITORY_ROOT}/supported_ap_code/target/release/select_adaptive_hillq_cluster"
PLAN_WORKER="${SCRIPT_DIR}/run_adaptive_hillq_plan.slurm"
WORKER="${SCRIPT_DIR}/run_adaptive_hillq_selection_array.slurm"
FINALIZER="${SCRIPT_DIR}/finalize_adaptive_hillq_selection_slurm.sh"
MEASURE_HELPER="${SCRIPT_DIR}/slurm_round_robin_helper.py"
ADAPTIVE_HELPER="${SCRIPT_DIR}/adaptive_hillq_cluster_helper.py"
THREADS=12
SELECTION_REPLICATIONS=200
MATCHES_PER_SHARD=250
PILOT_SHARDS=2
ARRAY_CHUNK_SIZE=900
MAX_CONCURRENT=200
PARTITION=componc_cpu
ACCOUNT=lukszam
MEMORY=20G
WALLTIME=
PREPARE_ONLY=false
DRY_RUN=false

usage() {
    cat <<'EOF'
Usage:
  bash submit_adaptive_hillq_selection_slurm.sh \
    --mode pilot|full \
    --source-root /path/to/full_roster_transfers \
    --bundle-root /path/to/full_roster_fixed_l2 \
    --run-root /path/to/adaptive_selection_run \
    [options]

The parameter-selection challenge defaults to 200 replications. The validated
fixed-L2 bundle and its downstream 600-replication tournament are not modified.

Options:
  --selector PATH
  --cluster-binary PATH
  --threads N                    CPUs/Rayon threads per task (default 12)
  --selection-replications N     must equal the frozen production value 200
  --matches-per-shard N          CNAP comparisons per shard (default 250)
  --pilot-shards N               pilot array size (default 2)
  --array-chunk-size N           sequential Slurm arrays per batch (default 900)
  --max-concurrent N             task cap within each array (default 200)
  --partition NAME               default componc_cpu
  --account NAME                 default lukszam; use '' to omit
  --mem SIZE                     default 20G
  --time D-HH:MM:SS              pilot default 08:00:00; full 2-00:00:00
  --prepare-only                 create/reuse the plan in the current allocation
  --dry-run                      print sbatch commands without submitting
EOF
}

positive_integer() {
    [[ "$2" =~ ^[1-9][0-9]*$ ]] || { echo "ERROR: $1 must be positive" >&2; exit 2; }
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --mode) MODE=${2:?}; shift 2 ;;
        --source-root) SOURCE_ROOT=${2:?}; shift 2 ;;
        --bundle-root) BUNDLE_ROOT=${2:?}; shift 2 ;;
        --run-root) RUN_ROOT=${2:?}; shift 2 ;;
        --selector) SELECTOR_BINARY=${2:?}; shift 2 ;;
        --cluster-binary) CLUSTER_BINARY=${2:?}; shift 2 ;;
        --threads) THREADS=${2:?}; shift 2 ;;
        --selection-replications) SELECTION_REPLICATIONS=${2:?}; shift 2 ;;
        --matches-per-shard) MATCHES_PER_SHARD=${2:?}; shift 2 ;;
        --pilot-shards) PILOT_SHARDS=${2:?}; shift 2 ;;
        --array-chunk-size) ARRAY_CHUNK_SIZE=${2:?}; shift 2 ;;
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
[ -n "${SOURCE_ROOT}" ] || { echo "ERROR: --source-root is required" >&2; exit 2; }
[ -n "${BUNDLE_ROOT}" ] || { echo "ERROR: --bundle-root is required" >&2; exit 2; }
[ -n "${RUN_ROOT}" ] || { echo "ERROR: --run-root is required" >&2; exit 2; }
for item in --threads:${THREADS} --selection-replications:${SELECTION_REPLICATIONS} \
    --matches-per-shard:${MATCHES_PER_SHARD} --pilot-shards:${PILOT_SHARDS} \
    --array-chunk-size:${ARRAY_CHUNK_SIZE} --max-concurrent:${MAX_CONCURRENT}; do
    positive_integer "${item%%:*}" "${item#*:}"
done
[ "${SELECTION_REPLICATIONS}" -eq 200 ] || { echo "ERROR: production selection replications are frozen at 200" >&2; exit 2; }

SOURCE_ROOT=$(cd "${SOURCE_ROOT}" && pwd)
BUNDLE_ROOT=$(cd "${BUNDLE_ROOT}" && pwd)
RUN_ROOT=$(mkdir -p "${RUN_ROOT}" && cd "${RUN_ROOT}" && pwd)
SELECTOR_BINARY=$(cd "$(dirname "${SELECTOR_BINARY}")" && pwd)/$(basename "${SELECTOR_BINARY}")
CLUSTER_BINARY=$(cd "$(dirname "${CLUSTER_BINARY}")" && pwd)/$(basename "${CLUSTER_BINARY}")

for executable in "${SELECTOR_BINARY}" "${CLUSTER_BINARY}"; do
    [ -x "${executable}" ] || {
        echo "ERROR: missing release binary: ${executable}" >&2
        echo "Build with: cargo build --release --manifest-path ${REPOSITORY_ROOT}/supported_ap_code/Cargo.toml --package select_adaptive_hillq --bins" >&2
        exit 1
    }
done
for path in "${PLAN_WORKER}" "${WORKER}" "${FINALIZER}" "${MEASURE_HELPER}" "${ADAPTIVE_HELPER}"; do
    [ -f "${path}" ] || { echo "ERROR: missing workflow component: ${path}" >&2; exit 1; }
done

SBATCH_COMMON=(--partition "${PARTITION}" --nodes 1 --ntasks 1)
if [ -n "${ACCOUNT}" ]; then
    SBATCH_COMMON+=(--account "${ACCOUNT}")
fi

PLAN="${RUN_ROOT}/plan"
if [ ! -f "${PLAN}/plan.json" ]; then
    if [ "${PREPARE_ONLY}" = true ]; then
        echo "Generating immutable adaptive-CNAP plan in the current allocation..."
        "${CLUSTER_BINARY}" plan \
            --source-root "${SOURCE_ROOT}" --bundle-root "${BUNDLE_ROOT}" \
            --output "${PLAN}" --selection-replications "${SELECTION_REPLICATIONS}" \
            --matches-per-shard "${MATCHES_PER_SHARD}" --threads "${THREADS}"
    elif [ "${DRY_RUN}" = true ]; then
        echo "DRY RUN: an immutable plan must first be generated by ${PLAN_WORKER}"
        echo "Rerun after the planning job succeeds to preview the data-dependent arrays."
        exit 0
    else
        command -v sbatch >/dev/null || { echo "ERROR: sbatch is not available" >&2; exit 1; }
        mkdir -p "${RUN_ROOT}/planning_logs"
        PLAN_ENV="${RUN_ROOT}/plan.env"
        TEMP_PLAN_ENV="${PLAN_ENV}.tmp.$$"
        umask 077
        {
            printf 'SOURCE_ROOT=%q\n' "${SOURCE_ROOT}"
            printf 'BUNDLE_ROOT=%q\n' "${BUNDLE_ROOT}"
            printf 'PLAN=%q\n' "${PLAN}"
            printf 'CLUSTER_BINARY=%q\n' "${CLUSTER_BINARY}"
            printf 'THREADS=%q\n' "${THREADS}"
            printf 'SELECTION_REPLICATIONS=%q\n' "${SELECTION_REPLICATIONS}"
            printf 'MATCHES_PER_SHARD=%q\n' "${MATCHES_PER_SHARD}"
        } > "${TEMP_PLAN_ENV}"
        if [ -e "${PLAN_ENV}" ]; then
            cmp -s "${TEMP_PLAN_ENV}" "${PLAN_ENV}" || { rm "${TEMP_PLAN_ENV}"; echo "ERROR: existing plan.env differs" >&2; exit 1; }
            rm "${TEMP_PLAN_ENV}"
        else
            mv "${TEMP_PLAN_ENV}" "${PLAN_ENV}"
        fi
        echo "Submitting immutable planning job and waiting for completion..."
        PLAN_JOB_ID=$(sbatch "${SBATCH_COMMON[@]}" \
            --job-name iris_adaptive_plan --cpus-per-task "${THREADS}" \
            --mem "${MEMORY}" --time 02:00:00 --wait \
            --output "${RUN_ROOT}/planning_logs/plan_%j.out" \
            --error "${RUN_ROOT}/planning_logs/plan_%j.err" \
            --export "ALL,IRIS_ADAPTIVE_PLAN_ENV=${PLAN_ENV}" \
            --parsable "${PLAN_WORKER}")
        PLAN_JOB_ID=${PLAN_JOB_ID%%;*}
        echo "Completed planning job: ${PLAN_JOB_ID}"
    fi
fi
read -r SHARD_COUNT MATCH_COUNT PLAN_REPLICATIONS PLAN_MATCHES_PER_SHARD < <(
    python3 "${ADAPTIVE_HELPER}" plan-info --plan "${PLAN}"
)
[ "${PLAN_REPLICATIONS}" -eq "${SELECTION_REPLICATIONS}" ] || { echo "ERROR: existing plan has a different replication roster" >&2; exit 1; }
[ "${PLAN_MATCHES_PER_SHARD}" -eq "${MATCHES_PER_SHARD}" ] || { echo "ERROR: existing plan has a different shard size" >&2; exit 1; }

mkdir -p "${RUN_ROOT}/${MODE}/results" "${RUN_ROOT}/${MODE}/resource_usage" \
    "${RUN_ROOT}/${MODE}/logs" "${RUN_ROOT}/${MODE}/status"
RESULTS="${RUN_ROOT}/${MODE}/results"
FINAL_OUTPUT="${RUN_ROOT}/full/selection"
if [ "${MODE}" = pilot ]; then
    TOTAL_TASKS=${PILOT_SHARDS}
    [ "${TOTAL_TASKS}" -le "${SHARD_COUNT}" ] || TOTAL_TASKS=${SHARD_COUNT}
    WALLTIME=${WALLTIME:-08:00:00}
else
    TOTAL_TASKS=${SHARD_COUNT}
    WALLTIME=${WALLTIME:-2-00:00:00}
    [ ! -e "${FINAL_OUTPUT}" ] || { echo "ERROR: final output already exists: ${FINAL_OUTPUT}" >&2; exit 1; }
fi

RUN_ENV="${RUN_ROOT}/${MODE}/run.env"
TEMP_RUN_ENV="${RUN_ENV}.tmp.$$"
umask 077
{
    printf 'MODE=%q\n' "${MODE}"
    printf 'SOURCE_ROOT=%q\n' "${SOURCE_ROOT}"
    printf 'BUNDLE_ROOT=%q\n' "${BUNDLE_ROOT}"
    printf 'RUN_ROOT=%q\n' "${RUN_ROOT}"
    printf 'PLAN=%q\n' "${PLAN}"
    printf 'RESULTS=%q\n' "${RESULTS}"
    printf 'FINAL_OUTPUT=%q\n' "${FINAL_OUTPUT}"
    printf 'SELECTOR_BINARY=%q\n' "${SELECTOR_BINARY}"
    printf 'CLUSTER_BINARY=%q\n' "${CLUSTER_BINARY}"
    printf 'MEASURE_HELPER=%q\n' "${MEASURE_HELPER}"
    printf 'ADAPTIVE_HELPER=%q\n' "${ADAPTIVE_HELPER}"
    printf 'THREADS=%q\n' "${THREADS}"
    printf 'SELECTION_REPLICATIONS=%q\n' "${SELECTION_REPLICATIONS}"
} > "${TEMP_RUN_ENV}"
if [ -e "${RUN_ENV}" ]; then
    cmp -s "${TEMP_RUN_ENV}" "${RUN_ENV}" || { rm "${TEMP_RUN_ENV}"; echo "ERROR: existing run.env differs" >&2; exit 1; }
    rm "${TEMP_RUN_ENV}"
else
    mv "${TEMP_RUN_ENV}" "${RUN_ENV}"
fi

echo "Prepared adaptive-Hill-q ${MODE} run"
echo "  Unique comparisons: ${MATCH_COUNT}"
echo "  Plan shards:        ${SHARD_COUNT}"
echo "  Submitted tasks:    ${TOTAL_TASKS}"
echo "  Replications:       ${SELECTION_REPLICATIONS} (selection only)"
echo "  Threads/task:       ${THREADS}"
echo "  Run root:           ${RUN_ROOT}"

if [ "${PREPARE_ONLY}" = true ]; then
    exit 0
fi

OFFSET=0
PREVIOUS_JOB=
ARRAY_JOB_IDS=()
while [ "${OFFSET}" -lt "${TOTAL_TASKS}" ]; do
    REMAINING=$((TOTAL_TASKS - OFFSET))
    COUNT=${ARRAY_CHUNK_SIZE}
    [ "${COUNT}" -le "${REMAINING}" ] || COUNT=${REMAINING}
    ARRAY_LAST=$((COUNT - 1))
    COMMAND=(
        sbatch "${SBATCH_COMMON[@]}"
        --job-name "iris_adaptive_${MODE}"
        --array "0-${ARRAY_LAST}%${MAX_CONCURRENT}"
        --cpus-per-task "${THREADS}" --mem "${MEMORY}" --time "${WALLTIME}"
        --output "${RUN_ROOT}/${MODE}/logs/array_%A_%a.out"
        --error "${RUN_ROOT}/${MODE}/logs/array_%A_%a.err"
        --export "ALL,IRIS_ADAPTIVE_RUN_ENV=${RUN_ENV},IRIS_ADAPTIVE_SHARD_OFFSET=${OFFSET}"
    )
    if [ -n "${PREVIOUS_JOB}" ]; then
        COMMAND+=(--dependency "afterok:${PREVIOUS_JOB}")
    fi
    COMMAND+=(--parsable "${WORKER}")
    if [ "${DRY_RUN}" = true ]; then
        printf 'DRY RUN: '; printf '%q ' "${COMMAND[@]}"; printf '\n'
        PREVIOUS_JOB="<previous-array>"
    else
        command -v sbatch >/dev/null || { echo "ERROR: sbatch is not available" >&2; exit 1; }
        JOB_ID=$("${COMMAND[@]}")
        JOB_ID=${JOB_ID%%;*}
        ARRAY_JOB_IDS+=("${JOB_ID}")
        PREVIOUS_JOB=${JOB_ID}
    fi
    OFFSET=$((OFFSET + COUNT))
done

if [ "${DRY_RUN}" = true ]; then
    echo "DRY RUN: finalizer uses afterok:<last-array-job-id>"
    exit 0
fi

FINAL_JOB_ID=$(sbatch "${SBATCH_COMMON[@]}" \
    --job-name "iris_adaptive_${MODE}_finalize" \
    --cpus-per-task "${THREADS}" --mem 32G --time 08:00:00 \
    --dependency "afterok:${PREVIOUS_JOB}" \
    --output "${RUN_ROOT}/${MODE}/logs/finalize_%j.out" \
    --error "${RUN_ROOT}/${MODE}/logs/finalize_%j.err" \
    --export "ALL,IRIS_ADAPTIVE_RUN_ENV=${RUN_ENV}" \
    --parsable "${FINALIZER}")
FINAL_JOB_ID=${FINAL_JOB_ID%%;*}

echo "Submitted array jobs: ${ARRAY_JOB_IDS[*]}"
echo "Submitted afterok finalizer: ${FINAL_JOB_ID}"
