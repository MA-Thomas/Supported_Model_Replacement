#!/bin/bash
# Prepares ten NCI parameter tournaments; submits shards or one coordinator per branch.

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPOSITORY_ROOT=$(git -C "${SCRIPT_DIR}" rev-parse --show-toplevel)

CONFIG="${SCRIPT_DIR}/config.nci.v1.json"
RUN_ROOT=
PIPELINE="${REPOSITORY_ROOT}/supported_ap_code/target/release/iris_nci_parameter_tournament"
ORGANIZER="${REPOSITORY_ROOT}/supported_ap_code/target/release/directed_round_robin_organizer"
WORKER="${SCRIPT_DIR}/run_nci_parameter_tournament_array.slurm"
FINALIZER="${SCRIPT_DIR}/finalize_nci_parameter_tournaments_slurm.sh"
THREADS=12
MATCHES_PER_SHARD=25
EXECUTION_MODE=exhaustive
BATCH_SIZE=1
MAX_CONCURRENT=100
PARTITION=componc_cpu
ACCOUNT=lukszam
MEMORY=20G
WALLTIME=2-00:00:00
PREPARE_ONLY=false
DRY_RUN=false

usage() {
    cat <<'EOF'
Usage: bash submit_nci_parameter_tournaments_slurm.sh --run-root PATH [options]

Options:
  --config PATH              Frozen JSON configuration
  --pipeline PATH            Release iris_nci_parameter_tournament binary
  --organizer PATH           Release directed_round_robin_organizer binary
  --threads N                CPUs/Rayon threads per task (default 12)
  --matches-per-shard N      Organizer matches per shard (default 25)
  --execution-mode MODE      exhaustive (default) or accelerated
  --batch-size N             Accelerated opponent batch size (default 1)
  --max-concurrent N         Maximum simultaneous array tasks (default 100)
  --partition NAME           Slurm partition (default componc_cpu)
  --account NAME             Slurm account; use '' to omit (default lukszam)
  --mem SIZE                 Memory per task (default 20G)
  --time D-HH:MM:SS          Time per task (default 2-00:00:00)
  --prepare-only             Prepare/audit bundles and plans without sbatch
  --dry-run                  Print sbatch commands after preparation
EOF
}

positive_integer() {
    [[ "$2" =~ ^[1-9][0-9]*$ ]] || { echo "ERROR: $1 must be a positive integer" >&2; exit 2; }
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --config) CONFIG=${2:?}; shift 2 ;;
        --run-root) RUN_ROOT=${2:?}; shift 2 ;;
        --pipeline) PIPELINE=${2:?}; shift 2 ;;
        --organizer) ORGANIZER=${2:?}; shift 2 ;;
        --threads) THREADS=${2:?}; shift 2 ;;
        --matches-per-shard) MATCHES_PER_SHARD=${2:?}; shift 2 ;;
        --execution-mode) EXECUTION_MODE=${2:?}; shift 2 ;;
        --batch-size) BATCH_SIZE=${2:?}; shift 2 ;;
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

[ -n "${RUN_ROOT}" ] || { echo "ERROR: --run-root is required" >&2; exit 2; }
positive_integer --threads "${THREADS}"
positive_integer --matches-per-shard "${MATCHES_PER_SHARD}"
positive_integer --batch-size "${BATCH_SIZE}"
case "${EXECUTION_MODE}" in
    exhaustive|accelerated) ;;
    *) echo "ERROR: --execution-mode must be exhaustive or accelerated" >&2; exit 2 ;;
esac
positive_integer --max-concurrent "${MAX_CONCURRENT}"
for path in "${CONFIG}" "${WORKER}" "${FINALIZER}"; do
    [ -f "${path}" ] || { echo "ERROR: missing file ${path}" >&2; exit 1; }
done
for binary in "${PIPELINE}" "${ORGANIZER}"; do
    [ -x "${binary}" ] || { echo "ERROR: release binary is not executable: ${binary}" >&2; exit 1; }
done

CONFIG=$(cd "$(dirname "${CONFIG}")" && pwd)/$(basename "${CONFIG}")
RUN_ROOT=$(mkdir -p "${RUN_ROOT}" && cd "${RUN_ROOT}" && pwd)
PACKAGE_ROOT="${RUN_ROOT}/prepared"
if [ -d "${PACKAGE_ROOT}" ]; then
    "${PIPELINE}" audit-prepared --package "${PACKAGE_ROOT}" --config "${CONFIG}"
else
    "${PIPELINE}" prepare --config "${CONFIG}" --output "${PACKAGE_ROOT}" --threads "${THREADS}"
    "${PIPELINE}" audit-prepared --package "${PACKAGE_ROOT}" --config "${CONFIG}"
fi

TASK_MAP="${RUN_ROOT}/task_map.tsv"
TEMP_TASK_MAP="${TASK_MAP}.tmp.$$"
: > "${TEMP_TASK_MAP}"
TASK_ID=0
for MODEL in full_hla focal_hla old_monoallelic mono_q_full_pn full_q_mono_pn; do
    for METRIC in pr roc; do
        BUNDLE="${PACKAGE_ROOT}/${MODEL}/${METRIC}/bundle"
        PLAN="${RUN_ROOT}/plans/${MODEL}/${METRIC}"
        if [ "${EXECUTION_MODE}" = accelerated ]; then
            [ ! -f "${PLAN}/plan.json" ] || { echo "ERROR: exhaustive plan exists; choose a new run root" >&2; exit 1; }
            if [ ! -f "${PLAN}/accelerated_plan.json" ]; then
                mkdir -p "$(dirname "${PLAN}")"
                "${ORGANIZER}" accelerated plan --bundle "${BUNDLE}" --output "${PLAN}" --batch-size "${BATCH_SIZE}"
            fi
            python3 - "${PLAN}/accelerated_plan.json" "${BUNDLE}/bundle_manifest.json" "${BATCH_SIZE}" <<'PY'
import json, sys
with open(sys.argv[1]) as f: plan = json.load(f)
with open(sys.argv[2]) as f: bundle = json.load(f)
if (plan['bundle_content_hash'] != bundle['bundle_content_hash']
    or plan['options']['batch_size'] != int(sys.argv[3])
    or plan['options']['output_scope'] != 'survivor_set_and_operational_inputs'):
    raise SystemExit('ERROR: existing accelerated plan differs from requested bundle/batch/scope; choose a new run root')
PY
            printf '%s\t%s\t%s\t%s\n' "${TASK_ID}" "${MODEL}" "${METRIC}" "coordinator" >> "${TEMP_TASK_MAP}"
            TASK_ID=$((TASK_ID + 1))
            continue
        fi
        [ ! -f "${PLAN}/accelerated_plan.json" ] || { echo "ERROR: accelerated plan exists; choose a new run root" >&2; exit 1; }
        if [ ! -f "${PLAN}/plan.json" ]; then
            mkdir -p "$(dirname "${PLAN}")"
            "${ORGANIZER}" plan --bundle "${BUNDLE}" --output "${PLAN}" \
                --matches-per-shard "${MATCHES_PER_SHARD}"
        fi
        SHARDS=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["shard_count"])' "${PLAN}/plan.json")
        for ((SHARD_ID=0; SHARD_ID<SHARDS; SHARD_ID++)); do
            printf '%s\t%s\t%s\t%s\n' "${TASK_ID}" "${MODEL}" "${METRIC}" "${SHARD_ID}" >> "${TEMP_TASK_MAP}"
            TASK_ID=$((TASK_ID + 1))
        done
    done
done
if [ -f "${TASK_MAP}" ] && ! cmp -s "${TEMP_TASK_MAP}" "${TASK_MAP}"; then
    /bin/rm "${TEMP_TASK_MAP}"
    echo "ERROR: existing task map differs; choose a new run root" >&2
    exit 1
fi
[ -f "${TASK_MAP}" ] && /bin/rm "${TEMP_TASK_MAP}" || mv "${TEMP_TASK_MAP}" "${TASK_MAP}"

RUN_ENV="${RUN_ROOT}/run.env"
TEMP_RUN_ENV="${RUN_ENV}.tmp.$$"
umask 077
{
    printf 'CONFIG=%q\n' "${CONFIG}"
    printf 'RUN_ROOT=%q\n' "${RUN_ROOT}"
    printf 'PACKAGE_ROOT=%q\n' "${PACKAGE_ROOT}"
    printf 'TASK_MAP=%q\n' "${TASK_MAP}"
    printf 'PIPELINE=%q\n' "${PIPELINE}"
    printf 'ORGANIZER=%q\n' "${ORGANIZER}"
    printf 'THREADS=%q\n' "${THREADS}"
    # Keep the historical exhaustive run.env byte-compatible for resumption.
    if [ "${EXECUTION_MODE}" = accelerated ]; then
        printf 'EXECUTION_MODE=%q\n' "${EXECUTION_MODE}"
        printf 'BATCH_SIZE=%q\n' "${BATCH_SIZE}"
    fi
} > "${TEMP_RUN_ENV}"
if [ -f "${RUN_ENV}" ] && ! cmp -s "${TEMP_RUN_ENV}" "${RUN_ENV}"; then
    /bin/rm "${TEMP_RUN_ENV}"
    echo "ERROR: existing run environment differs; choose a new run root" >&2
    exit 1
fi
[ -f "${RUN_ENV}" ] && /bin/rm "${TEMP_RUN_ENV}" || mv "${TEMP_RUN_ENV}" "${RUN_ENV}"

ARRAY_LAST=$((TASK_ID - 1))
echo "Prepared 10 tournaments in ${EXECUTION_MODE} mode as ${TASK_ID} tasks; array 0-${ARRAY_LAST}%${MAX_CONCURRENT}"
[ "${PREPARE_ONLY}" = false ] || exit 0

SBATCH_COMMON=(--partition "${PARTITION}" --nodes 1 --ntasks 1)
[ -z "${ACCOUNT}" ] || SBATCH_COMMON+=(--account "${ACCOUNT}")
ARRAY_COMMAND=(sbatch "${SBATCH_COMMON[@]}" --job-name nci_parameter_rr \
    --array "0-${ARRAY_LAST}%${MAX_CONCURRENT}" --cpus-per-task "${THREADS}" \
    --mem "${MEMORY}" --time "${WALLTIME}" \
    --output "${RUN_ROOT}/array_%A_%a.out" --error "${RUN_ROOT}/array_%A_%a.err" \
    --export "ALL,NCI_PARAMETER_RUN_ENV=${RUN_ENV}" --parsable "${WORKER}")
if [ "${DRY_RUN}" = true ]; then
    printf 'DRY RUN: '; printf '%q ' "${ARRAY_COMMAND[@]}"; printf '\n'
    exit 0
fi

command -v sbatch >/dev/null || { echo "ERROR: sbatch is unavailable" >&2; exit 1; }
ARRAY_JOB_ID=$("${ARRAY_COMMAND[@]}")
ARRAY_JOB_ID=${ARRAY_JOB_ID%%;*}
FINAL_JOB_ID=$(sbatch "${SBATCH_COMMON[@]}" --job-name nci_parameter_finalize \
    --dependency "afterok:${ARRAY_JOB_ID}" --cpus-per-task "${THREADS}" \
    --mem "${MEMORY}" --time "${WALLTIME}" \
    --output "${RUN_ROOT}/finalize_%j.out" --error "${RUN_ROOT}/finalize_%j.err" \
    --export "ALL,NCI_PARAMETER_RUN_ENV=${RUN_ENV}" --parsable "${FINALIZER}")
echo "Submitted array ${ARRAY_JOB_ID}; finalizer ${FINAL_JOB_ID%%;*}"
