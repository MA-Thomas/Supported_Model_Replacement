#!/bin/bash
# ============================================================================
# Dataset-agnostic Stage 2 Slurm worker
# ============================================================================
#
# This script is submitted by submit_stage2_new.sh.  It intentionally receives
# all dataset and mode configuration via exported environment variables so the
# Slurm array geometry is explicit at submission time.
#
# ============================================================================

set -euo pipefail

echo "============================================================"
echo "Stage 2 worker started: $(date)"
echo "Dataset: ${DATASET:-unknown}"
echo "HLA representation: ${HLA_ENVIRONMENT_REPRESENTATION:-unknown}"
echo "Node: $(hostname)"
echo "Job ID: ${SLURM_JOB_ID:-unknown}"
echo "Array Task ID: ${SLURM_ARRAY_TASK_ID:-unknown}"
echo "CPUs allocated: ${SLURM_CPUS_PER_TASK:-unknown}"
echo "============================================================"

: "${COMPUTE_PN:?COMPUTE_PN must be exported by submit_stage2_new.sh}"
: "${COMPUTE_Q:?COMPUTE_Q must be exported by submit_stage2_new.sh}"
: "${COMPUTE_PI:?COMPUTE_PI must be exported by submit_stage2_new.sh}"
: "${COMPUTE_EVAC:?COMPUTE_EVAC must be exported by submit_stage2_new.sh}"
: "${EXECUTION_MODE:?EXECUTION_MODE must be exported by submit_stage2_new.sh}"
: "${TOTAL_ENV_IDS:?TOTAL_ENV_IDS must be exported by submit_stage2_new.sh}"
: "${ENV_CHUNKS:?ENV_CHUNKS must be exported by submit_stage2_new.sh}"
: "${RUNNER:?RUNNER must be exported by submit_stage2_new.sh}"
: "${RUNNER_ROOT:?RUNNER_ROOT must be exported by submit_stage2_new.sh}"
: "${OUTPUTS_ROOT:?OUTPUTS_ROOT must be exported by submit_stage2_new.sh}"
: "${SLURM_ARRAY_TASK_ID:?SLURM_ARRAY_TASK_ID must be set by Slurm}"
: "${HLA_ENVIRONMENT_REPRESENTATION:?HLA_ENVIRONMENT_REPRESENTATION must be exported by submit_stage2_new.sh}"

case "${HLA_ENVIRONMENT_REPRESENTATION}" in
    full|mono) ;;
    *)
        echo "ERROR: unknown HLA environment representation '${HLA_ENVIRONMENT_REPRESENTATION}'." >&2
        exit 1
        ;;
esac
if [ "${HLA_ENVIRONMENT_REPRESENTATION}" = "mono" ] && [ "${PN_HLA_SCOPE:-all}" != "all" ]; then
    echo "ERROR: mono representation requires PN_HLA_SCOPE=all." >&2
    exit 1
fi

COMPUTE_IN_VITRO="${COMPUTE_IN_VITRO:-0}"
IN_VITRO_PEPTIDE_CONC="${IN_VITRO_PEPTIDE_CONC:-1000.0}"

if [ ! -x "${RUNNER}" ]; then
    echo "ERROR: Runner binary not found or not executable: ${RUNNER}" >&2
    echo "Submit through submit_stage2_new.sh so the binary is built before the array starts." >&2
    exit 1
fi

export RAYON_NUM_THREADS="${SLURM_CPUS_PER_TASK:-1}"
echo "RAYON_NUM_THREADS=${RAYON_NUM_THREADS}"

TASK_ID="${SLURM_ARRAY_TASK_ID}"
PN_SHARDED=0

if [ "${EXECUTION_MODE}" = "pn" ]; then
    : "${PARAM_CHUNKS:?PARAM_CHUNKS must be exported in pn mode}"
    : "${TOTAL_PARAMS:?TOTAL_PARAMS must be exported in pn mode}"
    : "${PN_HLA_SCOPE:?PN_HLA_SCOPE must be exported in pn mode}"
    : "${MAX_NUM_PS_VALUES_LOG2:?MAX_NUM_PS_VALUES_LOG2 must be exported in pn mode}"

    if [ -n "${PN_SHARD_PLAN:-}" ]; then
        PN_SHARDED=1
        IFS=$'\t' read -r _ PARENT_TASK_ID START_IDX STOP_IDX ENV_CHUNK_ID \
            ENV_START ENV_STOP QUERY_SHARD_INDEX QUERY_SHARD_COUNT EXPECTED_QUERY_ROWS < <(
                "${PYTHON_BIN}" "${CONTRACT_HELPER}" pn-shard-task \
                    --plan "${PN_SHARD_PLAN}" --task-id "${TASK_ID}"
            )
        PARAM_CHUNK_ID=$((PARENT_TASK_ID / ENV_CHUNKS))
    else
        PARAM_CHUNK_ID=$((TASK_ID / ENV_CHUNKS))
        ENV_CHUNK_ID=$((TASK_ID % ENV_CHUNKS))

        if [ "${PARAM_CHUNK_ID}" -ge "${PARAM_CHUNKS}" ]; then
            echo "ERROR: task ${TASK_ID} decodes to param_chunk ${PARAM_CHUNK_ID}, but PARAM_CHUNKS=${PARAM_CHUNKS}." >&2
            exit 1
        fi

        PARAMS_PER_CHUNK=$((TOTAL_PARAMS / PARAM_CHUNKS))
        START_IDX=$((PARAM_CHUNK_ID * PARAMS_PER_CHUNK))
        STOP_IDX=$((START_IDX + PARAMS_PER_CHUNK))

        if [ "${PARAM_CHUNK_ID}" -eq $((PARAM_CHUNKS - 1)) ]; then
            STOP_IDX=${TOTAL_PARAMS}
        fi
    fi

    echo "Execution mode: PN"
    echo "2D grid position: param_chunk=${PARAM_CHUNK_ID}, env_chunk=${ENV_CHUNK_ID}"
    if [ "${PN_SHARDED}" -eq 1 ]; then
        echo "Query shard: ${QUERY_SHARD_INDEX}/${QUERY_SHARD_COUNT} (${EXPECTED_QUERY_ROWS} observations)"
    fi
    echo "PN HLA scope: ${PN_HLA_SCOPE}"
    echo "Max p_s values: 2^${MAX_NUM_PS_VALUES_LOG2}"
elif [ "${EXECUTION_MODE}" = "qpi_only" ]; then
    ENV_CHUNK_ID="${TASK_ID}"
    START_IDX=0
    STOP_IDX=1

    echo "Execution mode: Q/Pi-only"
    echo "1D grid position: env_chunk=${ENV_CHUNK_ID}"
elif [ "${EXECUTION_MODE}" = "evac" ]; then
    ENV_CHUNK_ID=0
    START_IDX=0
    STOP_IDX=1

    echo "Execution mode: E-vac"
else
    echo "ERROR: unknown EXECUTION_MODE='${EXECUTION_MODE}'." >&2
    exit 1
fi

if [ "${EXECUTION_MODE}" != "evac" ] && [ "${ENV_CHUNK_ID}" -ge "${ENV_CHUNKS}" ]; then
    echo "ERROR: task ${TASK_ID} decodes to env_chunk ${ENV_CHUNK_ID}, but ENV_CHUNKS=${ENV_CHUNKS}." >&2
    exit 1
fi

if [ "${PN_SHARDED}" -eq 1 ]; then
    : # The immutable task plan supplied the exact environment range.
elif [ "${EXECUTION_MODE}" = "evac" ]; then
    ENV_START=0
    ENV_STOP=${TOTAL_ENV_IDS}
else
    ENV_IDS_PER_CHUNK=$((TOTAL_ENV_IDS / ENV_CHUNKS))
    ENV_START=$((ENV_CHUNK_ID * ENV_IDS_PER_CHUNK))
    ENV_STOP=$((ENV_START + ENV_IDS_PER_CHUNK))

    if [ "${ENV_CHUNK_ID}" -eq $((ENV_CHUNKS - 1)) ]; then
        ENV_STOP=${TOTAL_ENV_IDS}
    fi
fi

echo "Processing parameter sets: [${START_IDX}, ${STOP_IDX})"
echo "Processing environment IDs: [${ENV_START}, ${ENV_STOP})"

if [ "${COMPUTE_EVAC}" = "1" ]; then
    OUTPUT_DIR="${OUTPUTS_ROOT}/results_evac"
elif [ "${COMPUTE_PN}" = "1" ]; then
    OUTPUT_DIR="${OUTPUTS_ROOT}/results_pn_env_id_${ENV_START}_${ENV_STOP}"
else
    OUTPUT_DIR="${OUTPUTS_ROOT}/results_qpi_env_id_${ENV_START}_${ENV_STOP}"
fi

if [ "${EXECUTION_MODE}" = "evac" ]; then
    : "${RUN_ID:?RUN_ID must be exported for evac}"
    DONE_FILE="${OUTPUT_DIR}/TASK_${TASK_ID}.done"
    RUN_OUTPUTS_ROOT="${OUTPUTS_ROOT}"
    if [ -f "${DONE_FILE}" ]; then
        echo "E-vac done file exists -> skipping: ${DONE_FILE}"
        exit 0
    fi
else
    : "${STAGE2_MODE:?STAGE2_MODE must be exported for pn/qpi}"
    : "${RUN_ID:?RUN_ID must be exported for pn/qpi}"
    : "${MODE_FINGERPRINT:?MODE_FINGERPRINT must be exported for pn/qpi}"
    : "${QUERY_INPUT_FILE:?QUERY_INPUT_FILE must be exported for pn/qpi}"
    : "${CONTRACT_HELPER:?CONTRACT_HELPER must be exported for pn/qpi}"
    : "${PYTHON_BIN:?PYTHON_BIN must be exported for pn/qpi}"
    : "${PARAM_FILE:?PARAM_FILE must be exported for pn/qpi}"
    : "${MN_TUPLES_FILE:?MN_TUPLES_FILE must be exported for pn/qpi}"
    : "${PARAM_FILE_SHA256:?PARAM_FILE_SHA256 must be exported for pn/qpi}"
    : "${MN_TUPLES_FILE_SHA256:?MN_TUPLES_FILE_SHA256 must be exported for pn/qpi}"

    hash_file() {
        local path=$1
        if command -v sha256sum >/dev/null 2>&1; then
            sha256sum "${path}" | awk '{print $1}'
        elif command -v shasum >/dev/null 2>&1; then
            shasum -a 256 "${path}" | awk '{print $1}'
        else
            echo "ERROR: sha256sum or shasum is required." >&2
            return 1
        fi
    }
    if [ "$(hash_file "${PARAM_FILE}")" != "${PARAM_FILE_SHA256}" ]; then
        echo "ERROR: parameter file changed after submission: ${PARAM_FILE}" >&2
        exit 1
    fi
    if [ "$(hash_file "${MN_TUPLES_FILE}")" != "${MN_TUPLES_FILE_SHA256}" ]; then
        echo "ERROR: M/N file changed after submission: ${MN_TUPLES_FILE}" >&2
        exit 1
    fi

    if [ "${PN_SHARDED}" -eq 1 ]; then
        DONE_FILE="${OUTPUTS_ROOT}/.pn_shards/TASK_${PARENT_TASK_ID}/SHARD_${QUERY_SHARD_INDEX}_OF_${QUERY_SHARD_COUNT}/SHARD.done.json"
    else
        DONE_FILE="${OUTPUT_DIR}/TASK_${TASK_ID}.done.json"
    fi
    if "${PYTHON_BIN}" "${CONTRACT_HELPER}" task-complete \
        --manifest "${DONE_FILE}" \
        --fingerprint "${MODE_FINGERPRINT}" \
        --run-root "${OUTPUTS_ROOT}" >/dev/null 2>&1; then
        echo "Validated matching task completion manifest -> skipping: ${DONE_FILE}"
        exit 0
    fi

    STAGING_ATTEMPT="${SLURM_JOB_ID:-local}_${SLURM_RESTART_COUNT:-0}"
    RUN_OUTPUTS_ROOT="${OUTPUTS_ROOT}/.staging/${STAGE2_MODE}/TASK_${TASK_ID}/${STAGING_ATTEMPT}"
fi

if [ "${EXECUTION_MODE}" = "pn" ] && [ "${GRID_PROFILE:-}" = "survivor-exact" ]; then
    MN_TUPLES_FILE=$("${PYTHON_BIN}" "${SURVIVOR_GRID_HELPER}" task-mn \
        --mode-manifest "${MODE_MANIFEST}" --start "${START_IDX}" --stop "${STOP_IDX}")
fi

RUNNER_ARGS=(
    --root "${RUNNER_ROOT}"
    --outputs-root "${RUN_OUTPUTS_ROOT}"
    --start-idx "${START_IDX}"
    --stop-idx "${STOP_IDX}"
)

if [ "${EXECUTION_MODE}" != "evac" ]; then
    RUNNER_ARGS+=(--env-id-start "${ENV_START}")
    RUNNER_ARGS+=(--env-id-end "${ENV_STOP}")
fi

if [ "${PN_SHARDED}" -eq 1 ]; then
    RUNNER_ARGS+=(--query-shard-index "${QUERY_SHARD_INDEX}")
    RUNNER_ARGS+=(--query-shard-count "${QUERY_SHARD_COUNT}")
fi

if [ "${COMPUTE_Q}" = "1" ] || [ "${COMPUTE_PN}" = "1" ] || [ "${COMPUTE_PI}" = "1" ]; then
    : "${Q_MODEL_CONFIG:?Q_MODEL_CONFIG must be exported when Q/P/N/Pi is computed}"
    RUNNER_ARGS+=(--q-model-config "${Q_MODEL_CONFIG}")
    RUNNER_ARGS+=(--hla-environment-representation "${HLA_ENVIRONMENT_REPRESENTATION}")
fi

if [ "${COMPUTE_PN}" = "1" ]; then
    RUNNER_ARGS+=(--compute-pn)
    RUNNER_ARGS+=(--parameter-file "${PARAM_FILE}")
    RUNNER_ARGS+=(--mn-tuples-file "${MN_TUPLES_FILE}")
    RUNNER_ARGS+=(--pn-hla-scope "${PN_HLA_SCOPE}")
    RUNNER_ARGS+=(--max-num-ps-values-log2 "${MAX_NUM_PS_VALUES_LOG2}")
fi

if [ "${COMPUTE_Q}" = "1" ]; then
    RUNNER_ARGS+=(--compute-q)
fi

if [ "${COMPUTE_PI}" = "1" ]; then
    : "${PMHC_CONFIG:?PMHC_CONFIG must be exported when Pi is computed}"
    : "${PMHC_PARAMETERS_FILE:?PMHC_PARAMETERS_FILE must be exported when Pi is computed}"
    RUNNER_ARGS+=(--compute-pi)
    RUNNER_ARGS+=(--pmhc-config "${PMHC_CONFIG}")
    RUNNER_ARGS+=(--pmhc-parameters-file "${PMHC_PARAMETERS_FILE}")
fi

if [ "${COMPUTE_EVAC}" = "1" ]; then
    : "${VACCINE_CONFIG:?VACCINE_CONFIG must be exported when E_vac is computed}"
    RUNNER_ARGS+=(--compute-evac)
    RUNNER_ARGS+=(--vaccine-config "${VACCINE_CONFIG}")
fi

if [ "${COMPUTE_IN_VITRO}" = "1" ]; then
    RUNNER_ARGS+=(--in-vitro)
    RUNNER_ARGS+=(--in-vitro-peptide-conc "${IN_VITRO_PEPTIDE_CONC}")
fi

echo "============================================================"
echo "Final Runner command:"
printf '%s ' "${RUNNER}" "${RUNNER_ARGS[@]}"
echo
echo "Final output root: ${OUTPUTS_ROOT}"
echo "Task staging root: ${RUN_OUTPUTS_ROOT}"
echo "Completion file:   ${DONE_FILE}"
echo "============================================================"

"${RUNNER}" "${RUNNER_ARGS[@]}"

if [ "${EXECUTION_MODE}" = "evac" ]; then
    mkdir -p "${OUTPUT_DIR}"
    tmp_done="${DONE_FILE}.tmp"
    : > "${tmp_done}"
    mv "${tmp_done}" "${DONE_FILE}"
    echo "Wrote E-vac done file: ${DONE_FILE}"
elif [ "${PN_SHARDED}" -eq 1 ]; then
    "${PYTHON_BIN}" "${CONTRACT_HELPER}" validate-pn-shard \
        --plan "${PN_SHARD_PLAN}" \
        --task-id "${TASK_ID}" \
        --run-id "${RUN_ID}" \
        --fingerprint "${MODE_FINGERPRINT}" \
        --query "${QUERY_INPUT_FILE}" \
        --staging-root "${RUN_OUTPUTS_ROOT}" \
        --run-root "${OUTPUTS_ROOT}" \
        --param-file "${PARAM_FILE}" \
        --mn-tuples "${MN_TUPLES_FILE}"
    echo "Validated and retained P/N shard: ${DONE_FILE}"
else
    "${PYTHON_BIN}" "${CONTRACT_HELPER}" validate-publish \
        --mode "${STAGE2_MODE}" \
        --run-id "${RUN_ID}" \
        --fingerprint "${MODE_FINGERPRINT}" \
        --task-id "${TASK_ID}" \
        --query "${QUERY_INPUT_FILE}" \
        --staging-root "${RUN_OUTPUTS_ROOT}" \
        --final-root "${OUTPUTS_ROOT}" \
        --env-start "${ENV_START}" \
        --env-stop "${ENV_STOP}" \
        --parameter-start "${START_IDX}" \
        --parameter-stop "${STOP_IDX}" \
        --param-file "${PARAM_FILE}" \
        --mn-tuples "${MN_TUPLES_FILE}"
    echo "Validated outputs and wrote completion manifest: ${DONE_FILE}"
fi

echo "Stage 2 worker finished: $(date)"
echo "============================================================"
