#!/bin/bash
# ============================================================================
# Dataset-aware Stage 2 submission launcher
# ============================================================================
#
# This script is the user-facing entry point for Stage 2 submissions.  It owns
# dataset/mode configuration, validates env-id geometry, builds Runner once, and
# submits run_stage_2_new.sh with explicit Slurm options.
#
# Examples:
#   bash submit_stage2_new.sh --dataset COVID_SPIKE --mode pn --run-id 20260715_full
#   bash submit_stage2_new.sh --dataset COVID_NONSPIKE --mode pn --run-id 20260715_full --target-env-ids 13-15
#   bash submit_stage2_new.sh --dataset PDAC --mode qpi --run-id 20260715_full --missing-only
#   bash submit_stage2_new.sh --dataset PDAC --mode evac --run-id 20260715_full
#
# ============================================================================

set -euo pipefail

WORKSPACE="${WORKSPACE:-/data1/lukszam/Marcus/New_Approaches}"
MONO_WORKSPACE="${MONO_WORKSPACE:-/data1/lukszam/Marcus/New_Approaches_monoalleleic}"
RUNNER_ROOT="${RUNNER_ROOT:-/data1/lukszam/Marcus}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN_SCRIPT="${RUN_SCRIPT:-${SCRIPT_DIR}/run_stage_2_new.sh}"
FINALIZE_SHARDS_SCRIPT="${FINALIZE_SHARDS_SCRIPT:-${SCRIPT_DIR}/finalize_stage2_pn_shards.sh}"
OUT_ERR_DIR="${OUT_ERR_DIR:-/home/thomm15/New_Approaches_Scripts/Out_and_Err}"
CONTRACT_HELPER="${CONTRACT_HELPER:-${SCRIPT_DIR}/stage2_run_contract.py}"
PYTHON_BIN="${PYTHON_BIN:-python3}"

DATASET=""
MODE=""
RUN_ID=""
GRID_PROFILE="${GRID_PROFILE:-full-production}"
PARAM_FILE_OVERRIDE=""
MN_TUPLES_FILE_OVERRIDE=""
REGIME_MANIFEST=""
SURVIVOR_GRID_HELPER="${SCRIPT_DIR}/survivor_grid.py"
HLA_ENVIRONMENT_REPRESENTATION="${HLA_ENVIRONMENT_REPRESENTATION:-full}"
REPRESENTATION_CROSSWALK=""
TOTAL_ENV_IDS_OVERRIDE=""
ENV_CHUNKS_OVERRIDE=""
MISSING_ONLY=0
TARGET_ENV_IDS=""
ARRAY_CONCURRENCY=""
PARTITION="${PARTITION:-componc_cpu}"
CPUS_PER_TASK="${CPUS_PER_TASK:-8}"
MEM_OVERRIDE=""
TIME_OVERRIDE=""
DRY_RUN=0
COMPUTE_IN_VITRO_OVERRIDE=""
IN_VITRO_PEPTIDE_CONC="${IN_VITRO_PEPTIDE_CONC:-1000.0}"
Q_MODEL_CONFIG_OVERRIDE=""
EXTERNAL_VALIDATION_INPUT_ROOT="${EXTERNAL_VALIDATION_INPUT_ROOT:-}"
VACCINE_CONFIG_OVERRIDE=""
PN_HLA_SCOPE="${PN_HLA_SCOPE:-all}"
MAX_NUM_PS_VALUES_LOG2="${MAX_NUM_PS_VALUES_LOG2:-14}"
NCI_PN_LOW_MEM="${NCI_PN_LOW_MEM:-90G}"
NCI_PN_HIGH_MEM="${NCI_PN_HIGH_MEM:-120G}"
PN_SHARD_MERGE_MEM="${PN_SHARD_MERGE_MEM:-16G}"
PN_SHARD_MERGE_TIME="${PN_SHARD_MERGE_TIME:-0-02:00:00}"
PN_PEPTIDES_PER_SHARD=0
PN_SHARD_PLAN=""
PN_SHARDED=0

usage() {
    cat <<'EOF'
Usage:
  bash submit_stage2_new.sh --dataset DATASET --mode MODE [options]

Datasets:
  NCI
  PDAC
  COVID_SPIKE
  COVID_NONSPIKE

Modes:
  pn       Compute P/N over parameter chunks x environment chunks
  qpi      Compute Q and Pi over environment chunks only
  evac     Compute vaccine-induced expansion E_vac

Options:
  --target-env-ids LIST    Submit only env IDs/chunks in LIST, e.g. 13-15 or 0,2,4-7.
                           For datasets where env chunks contain multiple env IDs,
                           the containing env chunks are submitted.
  --run-id ID              Required for all modes. Use the same ID for Q/Pi,
                           PN, and E_vac; outputs are isolated under
                           OUTPUTS_ROOT/runs/ID.
  --missing-only           Submit only tasks without a valid matching JSON completion
                           manifest and intact recorded outputs.
  --array-concurrency N    Slurm array concurrency cap. Defaults: pn=100, qpi=100.
  --mem VALUE              Slurm memory request. Defaults: sharded complete-F
                           pn=60G, other pn=82G, qpi=60G.
                           An explicit value disables automatic NCI PN tiers.
  --time VALUE             Slurm time limit. Defaults: pn=3-00:00:00, qpi=0-00:50:00.
  --partition NAME         Slurm partition. Default: componc_cpu.
  --cpus-per-task N        Slurm CPUs per task. Default: 8.
  --in-vitro 0|1           Override Q/Pi in-vitro mode. Default: qpi=1, pn=0.
  --peptide-conc VALUE     In-vitro peptide concentration in nM. Default: 1000.0.
  --pn-hla-scope SCOPE     HLA recognition scope for thymic P/N: all or focal.
                           Default: all. mono representation requires all.
  --hla-environment-representation full|mono
                           Physical HLA environment used for query and self Q.
                           Default: full.
  --representation-crosswalk PATH
                           Maps mono observations/environments back to their full
                           biological identities. Required for PDAC/COVID mono.
  --grid-profile PROFILE   full-production (1,008 x 25; default) or
                           survivor-exact (certified sparse regimes; external cohorts).
  --regime-manifest PATH  Required survivor grid manifest for external cohorts.
  --param-file PATH        Parameter CSV. Required explicitly for the external profile.
  --mn-tuples-file PATH    M/N CSV. Required explicitly for the external profile.
  --pn-peptides-per-shard N
                           Shard complete-F P/N query observations to at most N
                           rows per worker. Not supported for NCI or Q/Pi.
  --total-env-ids N        Override representation-specific environment count.
  --env-chunks N           Override representation-specific environment chunks.
  --max-num-ps-values-log2 N
                           Base-2 exponent for the P/N top-k cap. Default: 14.
  --q-model-config PATH    Override dataset Q model TOML.
  --external-validation-input-root PATH
                           Directory containing the installed complete-F query
                           and representation files referenced by generated TOMLs.
  --vaccine-config PATH    Override vaccine expansion TOML.
  --dry-run                Print the resolved submission without calling sbatch.
  -h, --help               Show this help.

Environment override:
  PMHC_PARAMETERS_FILE     Absolute estimated beta/lambda CSV used by qpi.
  NCI_PN_LOW_MEM           NCI PN low-memory tier. Default: 90G.
  NCI_PN_HIGH_MEM          NCI PN high-memory tier. Default: 120G.
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --dataset)
            DATASET="${2:-}"
            shift 2
            ;;
        --mode)
            MODE="${2:-}"
            shift 2
            ;;
        --target-env-ids)
            TARGET_ENV_IDS="${2:-}"
            shift 2
            ;;
        --missing-only)
            MISSING_ONLY=1
            shift
            ;;
        --run-id)
            RUN_ID="${2:-}"
            shift 2
            ;;
        --regime-manifest)
            REGIME_MANIFEST="${2:-}"
            shift 2
            ;;
        --grid-profile)
            GRID_PROFILE="${2:-}"
            shift 2
            ;;
        --param-file)
            PARAM_FILE_OVERRIDE="${2:-}"
            shift 2
            ;;
        --mn-tuples-file)
            MN_TUPLES_FILE_OVERRIDE="${2:-}"
            shift 2
            ;;
        --pn-peptides-per-shard)
            PN_PEPTIDES_PER_SHARD="${2:-}"
            shift 2
            ;;
        --hla-environment-representation)
            HLA_ENVIRONMENT_REPRESENTATION="${2:-}"
            shift 2
            ;;
        --representation-crosswalk)
            REPRESENTATION_CROSSWALK="${2:-}"
            shift 2
            ;;
        --total-env-ids)
            TOTAL_ENV_IDS_OVERRIDE="${2:-}"
            shift 2
            ;;
        --env-chunks)
            ENV_CHUNKS_OVERRIDE="${2:-}"
            shift 2
            ;;
        --array-concurrency)
            ARRAY_CONCURRENCY="${2:-}"
            shift 2
            ;;
        --mem)
            MEM_OVERRIDE="${2:-}"
            shift 2
            ;;
        --time)
            TIME_OVERRIDE="${2:-}"
            shift 2
            ;;
        --partition)
            PARTITION="${2:-}"
            shift 2
            ;;
        --cpus-per-task)
            CPUS_PER_TASK="${2:-}"
            shift 2
            ;;
        --in-vitro)
            COMPUTE_IN_VITRO_OVERRIDE="${2:-}"
            shift 2
            ;;
        --peptide-conc)
            IN_VITRO_PEPTIDE_CONC="${2:-}"
            shift 2
            ;;
        --pn-hla-scope)
            PN_HLA_SCOPE="${2:-}"
            shift 2
            ;;
        --max-num-ps-values-log2)
            MAX_NUM_PS_VALUES_LOG2="${2:-}"
            shift 2
            ;;
        --q-model-config)
            Q_MODEL_CONFIG_OVERRIDE="${2:-}"
            shift 2
            ;;
        --external-validation-input-root)
            EXTERNAL_VALIDATION_INPUT_ROOT="${2:-}"
            shift 2
            ;;
        --vaccine-config)
            VACCINE_CONFIG_OVERRIDE="${2:-}"
            shift 2
            ;;
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "ERROR: unknown argument: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

if [ -z "${DATASET}" ] || [ -z "${MODE}" ]; then
    echo "ERROR: --dataset and --mode are required." >&2
    usage >&2
    exit 1
fi

DATASET="$(printf '%s' "${DATASET}" | tr '[:lower:]' '[:upper:]')"
MODE="$(printf '%s' "${MODE}" | tr '[:upper:]' '[:lower:]')"
PN_HLA_SCOPE="$(printf '%s' "${PN_HLA_SCOPE}" | tr '[:upper:]' '[:lower:]')"
if [ -n "${EXTERNAL_VALIDATION_INPUT_ROOT}" ]; then
    EXTERNAL_VALIDATION_INPUT_ROOT="$(cd "${EXTERNAL_VALIDATION_INPUT_ROOT}" && pwd)"
    export EXTERNAL_VALIDATION_INPUT_ROOT
fi
HLA_ENVIRONMENT_REPRESENTATION="$(printf '%s' "${HLA_ENVIRONMENT_REPRESENTATION}" | tr '[:upper:]' '[:lower:]')"
GRID_PROFILE="$(printf '%s' "${GRID_PROFILE}" | tr '[:upper:]_' '[:lower:]-')"

case "${PN_HLA_SCOPE}" in
    all|focal)
        ;;
    *)
        echo "ERROR: unknown PN HLA scope '${PN_HLA_SCOPE}' (use all or focal)." >&2
        exit 1
        ;;
esac

case "${HLA_ENVIRONMENT_REPRESENTATION}" in
    full|mono) ;;
    *)
        echo "ERROR: unknown HLA environment representation '${HLA_ENVIRONMENT_REPRESENTATION}' (use full or mono)." >&2
        exit 1
        ;;
esac
if [ "${HLA_ENVIRONMENT_REPRESENTATION}" = "mono" ] && [ "${PN_HLA_SCOPE}" != "all" ]; then
    echo "ERROR: mono representation requires --pn-hla-scope all." >&2
    exit 1
fi

if ! printf '%s' "${MAX_NUM_PS_VALUES_LOG2}" | grep -Eq '^[0-9]+$'; then
    echo "ERROR: --max-num-ps-values-log2 must be a non-negative integer." >&2
    exit 1
fi
if [ "${MAX_NUM_PS_VALUES_LOG2}" -ge 64 ]; then
    echo "ERROR: --max-num-ps-values-log2 must be less than 64." >&2
    exit 1
fi

case "${MODE}" in
    pn)
        EXECUTION_MODE="pn"
        COMPUTE_PN=1
        COMPUTE_Q=0
        COMPUTE_PI=0
        COMPUTE_EVAC=0
        DEFAULT_MEM="82G"
        DEFAULT_TIME="3-00:00:00"
        DEFAULT_CONCURRENCY=100
        DEFAULT_COMPUTE_IN_VITRO=0
        ;;
    qpi|qpi_only)
        MODE="qpi"
        EXECUTION_MODE="qpi_only"
        COMPUTE_PN=0
        COMPUTE_Q=1
        COMPUTE_PI=1
        COMPUTE_EVAC=0
        DEFAULT_MEM="60G"
        DEFAULT_TIME="0-00:50:00"
        DEFAULT_CONCURRENCY=100
        DEFAULT_COMPUTE_IN_VITRO=1
        ;;
    evac)
        EXECUTION_MODE="evac"
        COMPUTE_PN=0
        COMPUTE_Q=0
        COMPUTE_PI=0
        COMPUTE_EVAC=1
        DEFAULT_MEM="100G"
        DEFAULT_TIME="0-02:00:00"
        DEFAULT_CONCURRENCY=1
        DEFAULT_COMPUTE_IN_VITRO=0
        ;;
    *)
        echo "ERROR: unknown mode '${MODE}' (use pn or qpi)." >&2
        exit 1
        ;;
esac

if [ "${MODE}" = "evac" ] && { [ "${PN_HLA_SCOPE}" != "all" ] || [ "${HLA_ENVIRONMENT_REPRESENTATION}" != "full" ]; }; then
    echo "ERROR: evac currently supports only full representation with PN scope all." >&2
    exit 1
fi

case "${DATASET}" in
    NCI)
        OUTPUTS_ROOT="${OUTPUTS_ROOT:-${WORKSPACE}/Runner/Outputs_NCI}"
        if [ "${HLA_ENVIRONMENT_REPRESENTATION}" = "mono" ]; then
            TOTAL_ENV_IDS=196
            ENV_CHUNKS=49
            ENV_DICT="${ENV_DICT:-/data1/lukszam/Marcus/Foreign_Epitopes/New_Approaches_Data/EricDiego_Peptides/hla_env_dict_SingleAlleleModel.csv}"
            Q_MODEL_CONFIG="${Q_MODEL_CONFIG_OVERRIDE:-${MONO_WORKSPACE}/MHC_competition_solver/src/binding_sim_updated_NCI_SingleAllele.toml}"
        else
            TOTAL_ENV_IDS=130
            ENV_CHUNKS=43
            ENV_DICT="${ENV_DICT:-/data1/lukszam/Marcus/Foreign_Epitopes/New_Approaches_Data/EricDiego_Peptides/hla_env_dict.csv}"
            Q_MODEL_CONFIG="${Q_MODEL_CONFIG_OVERRIDE:-${WORKSPACE}/MHC_competition_solver/src/binding_sim_updated_NCI.toml}"
        fi
        VACCINE_CONFIG="${VACCINE_CONFIG_OVERRIDE:-}"
        ;;
    PDAC)
        TOTAL_ENV_IDS=22
        ENV_CHUNKS=22
        OUTPUTS_ROOT="${OUTPUTS_ROOT:-${WORKSPACE}/Runner/Outputs_PDAC}"
        ENV_DICT="${ENV_DICT:-/data1/lukszam/Marcus/PDAC_vax_trial_epitopes/hla_env_dict.csv}"
        Q_MODEL_CONFIG="${Q_MODEL_CONFIG_OVERRIDE:-${WORKSPACE}/MHC_competition_solver/src/binding_sim_updated_PDAC.toml}"
        VACCINE_CONFIG="${VACCINE_CONFIG_OVERRIDE:-${WORKSPACE}/Vaccine_expansion/vaccine_expansion.toml}"
        ;;
    COVID_SPIKE)
        TOTAL_ENV_IDS=13
        ENV_CHUNKS=13
        OUTPUTS_ROOT="${OUTPUTS_ROOT:-${WORKSPACE}/Runner/Outputs_Cansu_Covid_Spike}"
        ENV_DICT="${ENV_DICT:-/data1/lukszam/Marcus/Cansu_Covid_Spike/Marcus_preprocessing/covid_spike_hla_env_dict.csv}"
        Q_MODEL_CONFIG="${Q_MODEL_CONFIG_OVERRIDE:-${WORKSPACE}/MHC_competition_solver/src/binding_sim_updated_COVID_Spike.toml}"
        VACCINE_CONFIG="${VACCINE_CONFIG_OVERRIDE:-}"
        ;;
    COVID_NONSPIKE)
        TOTAL_ENV_IDS=16
        ENV_CHUNKS=16
        OUTPUTS_ROOT="${OUTPUTS_ROOT:-${WORKSPACE}/Runner/Outputs_Cansu_Covid_Nonspike}"
        ENV_DICT="${ENV_DICT:-/data1/lukszam/Marcus/Cansu_Covid_Nonspike/Marcus_preprocessing/atlas_hla_env_dict.csv}"
        Q_MODEL_CONFIG="${Q_MODEL_CONFIG_OVERRIDE:-${WORKSPACE}/MHC_competition_solver/src/binding_sim_updated_COVID_Nonspike.toml}"
        VACCINE_CONFIG="${VACCINE_CONFIG_OVERRIDE:-}"
        ;;
    *)
        echo "ERROR: unknown dataset '${DATASET}'." >&2
        exit 1
        ;;
esac

if [ "${HLA_ENVIRONMENT_REPRESENTATION}" = "mono" ] && [ "${DATASET}" != "NCI" ]; then
    if [ -z "${Q_MODEL_CONFIG_OVERRIDE}" ] || [ -z "${TOTAL_ENV_IDS_OVERRIDE}" ] || \
       [ -z "${ENV_CHUNKS_OVERRIDE}" ] || [ -z "${REPRESENTATION_CROSSWALK}" ]; then
        echo "ERROR: ${DATASET} mono representation is Gate-B controlled and requires" >&2
        echo "       --q-model-config, --total-env-ids, --env-chunks, and --representation-crosswalk." >&2
        exit 1
    fi
    Q_MODEL_CONFIG="${Q_MODEL_CONFIG_OVERRIDE}"
fi

if [ -n "${TOTAL_ENV_IDS_OVERRIDE}" ]; then
    TOTAL_ENV_IDS="${TOTAL_ENV_IDS_OVERRIDE}"
fi
if [ -n "${ENV_CHUNKS_OVERRIDE}" ]; then
    ENV_CHUNKS="${ENV_CHUNKS_OVERRIDE}"
fi
for geometry_value in "${TOTAL_ENV_IDS}" "${ENV_CHUNKS}"; do
    if ! printf '%s' "${geometry_value}" | grep -Eq '^[1-9][0-9]*$'; then
        echo "ERROR: environment counts/chunks must be positive integers; saw '${geometry_value}'." >&2
        exit 1
    fi
done

OUTPUTS_BASE_ROOT="${OUTPUTS_ROOT}"
case "${GRID_PROFILE}" in
    full-production)
        TOTAL_PARAMS=1008
        PARAM_CHUNKS=48
        PARAM_FILE="${PARAM_FILE_OVERRIDE:-${PARAM_FILE:-${WORKSPACE}/TCR_availability/param_sets.csv}}"
        MN_TUPLES_FILE="${MN_TUPLES_FILE_OVERRIDE:-${MN_TUPLES_FILE:-${WORKSPACE}/TCR_availability/mn_tuples.csv}}"
        ;;
    survivor-exact)
        if [ -z "${REGIME_MANIFEST}" ] || [ -n "${PARAM_FILE_OVERRIDE}" ] || [ -n "${MN_TUPLES_FILE_OVERRIDE}" ]; then
            echo "ERROR: survivor-exact requires --regime-manifest and derives both CSV paths from it." >&2
            exit 1
        fi
        GRID_DESCRIPTION=$("${PYTHON_BIN}" "${SURVIVOR_GRID_HELPER}" describe \
            --manifest "${REGIME_MANIFEST}" --representation "${HLA_ENVIRONMENT_REPRESENTATION}" --scope "${PN_HLA_SCOPE}")
        IFS=$'\t' read -r TOTAL_PARAMS PARAM_CHUNKS PARAM_FILE MN_TUPLES_FILE <<< "${GRID_DESCRIPTION}"
        ;;
    *)
        echo "ERROR: unknown grid profile '${GRID_PROFILE}' (use full-production for NCI or survivor-exact for external cohorts)." >&2
        exit 1
        ;;
esac

if [ "${DATASET}" != "NCI" ] && [ "${MODE}" != "evac" ] && [ "${GRID_PROFILE}" != "survivor-exact" ]; then
    echo "ERROR: external-cohort PN/QPi require --grid-profile survivor-exact." >&2
    exit 1
fi

if ! printf '%s' "${PN_PEPTIDES_PER_SHARD}" | grep -Eq '^[0-9]+$'; then
    echo "ERROR: --pn-peptides-per-shard must be a non-negative integer." >&2
    exit 1
fi
if [ "${PN_PEPTIDES_PER_SHARD}" -gt 0 ]; then
    if [ "${MODE}" != "pn" ]; then
        echo "ERROR: --pn-peptides-per-shard is valid only for P/N." >&2
        exit 1
    fi
    case "${DATASET}" in
        PDAC|COVID_SPIKE|COVID_NONSPIKE) ;;
        *)
            echo "ERROR: P/N peptide sharding is not supported for ${DATASET}; NCI retains its legacy task geometry." >&2
            exit 1
            ;;
    esac
    if [ "${GRID_PROFILE}" != "survivor-exact" ]; then
        echo "ERROR: P/N peptide sharding requires --grid-profile survivor-exact." >&2
        exit 1
    fi
    if [ ! -f "${FINALIZE_SHARDS_SCRIPT}" ]; then
        echo "ERROR: P/N shard finalizer not found: ${FINALIZE_SHARDS_SCRIPT}" >&2
        exit 1
    fi
    PN_SHARDED=1
    DEFAULT_MEM="60G"
fi
PMHC_CONFIG="${PMHC_CONFIG:-${WORKSPACE}/pMHC_TCR_interaction_strength/src/pMHC_TCR_interaction_config.toml}"
PMHC_PARAMETERS_FILE="${PMHC_PARAMETERS_FILE:-${WORKSPACE}/pMHC_TCR_interaction_strength/estimated_hla_specific_betas_and_global_lambda_params.csv}"

if [ -z "${RUN_ID}" ]; then
    if [ "${DRY_RUN}" -eq 1 ]; then
        RUN_ID="DRY_RUN"
    else
        echo "ERROR: --run-id is required for ${MODE} submissions." >&2
        echo "Use the same run ID for the Q/Pi, PN, and E_vac phases of one rerun." >&2
        exit 1
    fi
fi
if ! printf '%s' "${RUN_ID}" | grep -Eq '^[A-Za-z0-9][A-Za-z0-9._-]*$'; then
    echo "ERROR: --run-id may contain only letters, digits, dot, underscore, and hyphen." >&2
    exit 1
fi
OUTPUTS_ROOT="${OUTPUTS_BASE_ROOT}/runs/${RUN_ID}"

if ! command -v "${PYTHON_BIN}" >/dev/null 2>&1; then
    echo "ERROR: Python interpreter not found: ${PYTHON_BIN}" >&2
    exit 1
fi
if [ ! -f "${PARAM_FILE}" ]; then
    echo "ERROR: parameter file not found: ${PARAM_FILE}" >&2
    exit 1
fi
if [ ! -f "${MN_TUPLES_FILE}" ]; then
    echo "ERROR: M/N file not found: ${MN_TUPLES_FILE}" >&2
    exit 1
fi
if [ -n "${REPRESENTATION_CROSSWALK}" ] && [ ! -f "${REPRESENTATION_CROSSWALK}" ]; then
    echo "ERROR: representation crosswalk not found: ${REPRESENTATION_CROSSWALK}" >&2
    exit 1
fi

absolute_file_path() {
    local path="$1"
    local directory
    directory=$(cd "$(dirname "${path}")" && pwd)
    printf '%s/%s\n' "${directory}" "$(basename "${path}")"
}

# Slurm workers may start in a different directory from the submit shell. Export
# immutable, absolute paths so the files that were validated and fingerprinted
# here are the exact files consumed by every array task.
PARAM_FILE=$(absolute_file_path "${PARAM_FILE}")
MN_TUPLES_FILE=$(absolute_file_path "${MN_TUPLES_FILE}")
if [ -n "${REGIME_MANIFEST}" ]; then
    REGIME_MANIFEST=$(absolute_file_path "${REGIME_MANIFEST}")
fi
if [ -n "${REPRESENTATION_CROSSWALK}" ]; then
    REPRESENTATION_CROSSWALK=$(absolute_file_path "${REPRESENTATION_CROSSWALK}")
fi

validate_grid_profile() {
    if [ "${GRID_PROFILE}" = "survivor-exact" ]; then
        "${PYTHON_BIN}" "${SURVIVOR_GRID_HELPER}" describe --manifest "${REGIME_MANIFEST}" \
            --representation "${HLA_ENVIRONMENT_REPRESENTATION}" --scope "${PN_HLA_SCOPE}" >/dev/null
        return
    fi
    "${PYTHON_BIN}" - "${GRID_PROFILE}" "${PARAM_FILE}" "${MN_TUPLES_FILE}" <<'PY'
import csv
import sys
from decimal import Decimal, InvalidOperation
from pathlib import Path

profile, param_name, mn_name = sys.argv[1:]
param_path = Path(param_name)
mn_path = Path(mn_name)
param_header = ["d_pos", "d_neg", "steepness_pos", "steepness_neg", "tau_thymus"]
mn_header = ["at_least_M", "at_most_N"]

def fail(message):
    raise SystemExit(f"ERROR: {message}")

with param_path.open(newline="", encoding="utf-8-sig") as handle:
    reader = csv.DictReader(handle)
    if reader.fieldnames != param_header:
        fail(f"parameter header must be {param_header}; saw {reader.fieldnames}")
    raw_params = list(reader)
params = []
for row_number, row in enumerate(raw_params, start=2):
    try:
        params.append((
            *(Decimal(row[column].strip()) for column in param_header[:4]),
            int(row["tau_thymus"].strip()),
        ))
    except (InvalidOperation, TypeError, ValueError, AttributeError) as exc:
        fail(f"invalid parameter CSV row {row_number}: {exc}")
if len(params) != len(set(params)):
    fail("parameter file contains duplicate rows")

with mn_path.open(newline="", encoding="utf-8-sig") as handle:
    reader = csv.DictReader(handle)
    if reader.fieldnames != mn_header:
        fail(f"M/N header must be {mn_header}; saw {reader.fieldnames}")
    raw_mn = list(reader)
try:
    mn = [(int(row["at_least_M"]), int(row["at_most_N"])) for row in raw_mn]
except (TypeError, ValueError) as exc:
    fail(f"invalid M/N row: {exc}")
if len(mn) != len(set(mn)):
    fail("M/N file contains duplicate rows")

taus = [1000, 50052, 91691, 120604, 156295, 200000]
if profile == "full-production":
    if len(params) != 1008:
        fail(f"full-production requires 1008 parameter rows; saw {len(params)}")
    if sorted({row[4] for row in params}) != taus:
        fail("full-production tau axis does not match the required six values")
    expected_mn = {(m, n) for m in (1, 2, 3, 6, 20) for n in (1, 2, 3, 8, 15)}
    if set(mn) != expected_mn:
        fail("full-production M/N file does not match the required 25-pair product")

else:
    fail(f"unknown grid profile {profile!r}")

print(f"Validated {profile}: {len(params)} parameter rows and {len(mn)} M/N rows.")
PY
}

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

validate_grid_profile
PARAM_FILE_SHA256=$(hash_file "${PARAM_FILE}")
MN_TUPLES_FILE_SHA256=$(hash_file "${MN_TUPLES_FILE}")

if [ "${MODE}" = "qpi" ]; then
    if [ ! -f "${PMHC_CONFIG}" ]; then
        echo "ERROR: pMHC config file not found: ${PMHC_CONFIG}" >&2
        exit 1
    fi
    if [ ! -f "${PMHC_PARAMETERS_FILE}" ]; then
        echo "ERROR: pMHC parameter file not found: ${PMHC_PARAMETERS_FILE}" >&2
        exit 1
    fi
fi

if [ "${MODE}" = "evac" ] && [ "${DATASET}" != "PDAC" ] && [ -z "${VACCINE_CONFIG_OVERRIDE}" ]; then
    echo "ERROR: evac mode currently requires DATASET=PDAC unless --vaccine-config is provided." >&2
    exit 1
fi

if [ "${MODE}" = "evac" ] && [ -z "${VACCINE_CONFIG}" ]; then
    echo "ERROR: evac mode requires VACCINE_CONFIG." >&2
    exit 1
fi

if [ -n "${ENV_DICT}" ] && [ ! -f "${ENV_DICT}" ]; then
    case "${DATASET}" in
        COVID_SPIKE)
            LOCAL_ENV_DICT="/Users/thomm15/Work_Data/Cansu_Covid_Spike/Marcus_preprocessing/covid_spike_hla_env_dict.csv"
            ;;
        COVID_NONSPIKE)
            LOCAL_ENV_DICT="/Users/thomm15/Work_Data/Cansu_Covid_Nonspike/Marcus_preprocessing/atlas_hla_env_dict.csv"
            ;;
        *)
            LOCAL_ENV_DICT=""
            ;;
    esac

    if [ -n "${LOCAL_ENV_DICT}" ] && [ -f "${LOCAL_ENV_DICT}" ]; then
        ENV_DICT="${LOCAL_ENV_DICT}"
    fi
fi

if [ ! -f "${RUN_SCRIPT}" ]; then
    echo "ERROR: run script not found: ${RUN_SCRIPT}" >&2
    exit 1
fi

if [ "${TOTAL_ENV_IDS}" -lt "${ENV_CHUNKS}" ]; then
    echo "ERROR: TOTAL_ENV_IDS (${TOTAL_ENV_IDS}) must be >= ENV_CHUNKS (${ENV_CHUNKS})." >&2
    exit 1
fi

if [ "${MODE}" = "pn" ]; then
    TOTAL_TASKS=$((PARAM_CHUNKS * ENV_CHUNKS))
elif [ "${MODE}" = "evac" ]; then
    TOTAL_TASKS=1
else
    TOTAL_TASKS=${ENV_CHUNKS}
fi
ARRAY_CONCURRENCY="${ARRAY_CONCURRENCY:-${DEFAULT_CONCURRENCY}}"
MEM="${MEM_OVERRIDE:-${DEFAULT_MEM}}"
TIME_LIMIT="${TIME_OVERRIDE:-${DEFAULT_TIME}}"
COMPUTE_IN_VITRO="${COMPUTE_IN_VITRO_OVERRIDE:-${DEFAULT_COMPUTE_IN_VITRO}}"

validate_env_dict() {
    local path=$1
    local expected_count=$2

    if [ -z "${path}" ]; then
        return 0
    fi

    if [ ! -f "${path}" ]; then
        echo "WARNING: env dict not found; skipping dict validation: ${path}" >&2
        return 0
    fi

    local count
    count=$(awk -F, 'NR > 1 && $1 != "" {count++} END {print count + 0}' "${path}")
    if [ "${count}" -ne "${expected_count}" ]; then
        echo "ERROR: env dict row count ${count} != TOTAL_ENV_IDS ${expected_count}: ${path}" >&2
        exit 1
    fi

    local expected=0
    while IFS= read -r env_id; do
        if [ "${env_id}" -ne "${expected}" ]; then
            echo "ERROR: env dict IDs are not contiguous from 0 at ${path}; expected ${expected}, saw ${env_id}." >&2
            exit 1
        fi
        expected=$((expected + 1))
    done < <(awk -F, 'NR > 1 && $1 != "" {print $1}' "${path}" | sort -n)
}

env_range_for_chunk() {
    local chunk_id=$1
    local ids_per_chunk=$((TOTAL_ENV_IDS / ENV_CHUNKS))
    local start=$((chunk_id * ids_per_chunk))
    local stop=$((start + ids_per_chunk))

    if [ "${chunk_id}" -eq $((ENV_CHUNKS - 1)) ]; then
        stop=${TOTAL_ENV_IDS}
    fi

    printf '%s %s\n' "${start}" "${stop}"
}

chunk_for_env_id() {
    local env_id=$1
    local chunk_id

    for ((chunk_id=0; chunk_id<ENV_CHUNKS; chunk_id++)); do
        local env_start env_stop
        IFS=' ' read -r env_start env_stop < <(env_range_for_chunk "${chunk_id}")
        if [ "${env_id}" -ge "${env_start}" ] && [ "${env_id}" -lt "${env_stop}" ]; then
            echo "${chunk_id}"
            return 0
        fi
    done

    echo "ERROR: env_id ${env_id} is outside configured range [0, ${TOTAL_ENV_IDS})." >&2
    exit 1
}

expand_env_id_list_to_chunks() {
    local spec=$1
    local tmp
    tmp=$(mktemp)

    local old_ifs=${IFS}
    IFS=,
    for part in ${spec}; do
        if printf '%s' "${part}" | grep -Eq '^[0-9]+-[0-9]+$'; then
            local start=${part%-*}
            local stop=${part#*-}
            if [ "${start}" -gt "${stop}" ]; then
                echo "ERROR: invalid env-id range '${part}'." >&2
                rm -f "${tmp}"
                exit 1
            fi
            local env_id
            for ((env_id=start; env_id<=stop; env_id++)); do
                chunk_for_env_id "${env_id}" >> "${tmp}"
            done
        elif printf '%s' "${part}" | grep -Eq '^[0-9]+$'; then
            chunk_for_env_id "${part}" >> "${tmp}"
        else
            echo "ERROR: invalid --target-env-ids component '${part}'." >&2
            rm -f "${tmp}"
            exit 1
        fi
    done
    IFS=${old_ifs}

    sort -n "${tmp}" | uniq
    rm -f "${tmp}"
}

compress_to_ranges() {
    if [ "$#" -eq 0 ]; then
        return 0
    fi

    local ranges=()
    local start=$1
    local prev=$1
    shift

    local curr
    for curr in "$@"; do
        if [ "${curr}" -eq $((prev + 1)) ]; then
            prev=${curr}
        else
            if [ "${start}" -eq "${prev}" ]; then
                ranges+=("${start}")
            else
                ranges+=("${start}-${prev}")
            fi
            start=${curr}
            prev=${curr}
        fi
    done

    if [ "${start}" -eq "${prev}" ]; then
        ranges+=("${start}")
    else
        ranges+=("${start}-${prev}")
    fi

    local old_ifs=${IFS}
    IFS=,
    echo "${ranges[*]}"
    IFS=${old_ifs}
}

build_candidate_task_ids() {
    if [ -n "${TARGET_ENV_IDS}" ]; then
        local chunk_ids=()
        while IFS= read -r chunk_id; do
            chunk_ids+=("${chunk_id}")
        done < <(expand_env_id_list_to_chunks "${TARGET_ENV_IDS}")

        if [ "${#chunk_ids[@]}" -eq 0 ]; then
            echo "ERROR: --target-env-ids produced no env chunks." >&2
            exit 1
        fi

        if [ "${MODE}" = "pn" ]; then
            local param_chunk chunk_id
            for ((param_chunk=0; param_chunk<PARAM_CHUNKS; param_chunk++)); do
                for chunk_id in "${chunk_ids[@]}"; do
                    echo $((param_chunk * ENV_CHUNKS + chunk_id))
                done
            done
        elif [ "${MODE}" = "evac" ]; then
            echo 0
        else
            printf '%s\n' "${chunk_ids[@]}"
        fi
    else
        local task_id
        for ((task_id=0; task_id<TOTAL_TASKS; task_id++)); do
            echo "${task_id}"
        done
    fi
}

task_output_dir() {
    local task_id=$1
    local env_chunk_id

    if [ "${MODE}" = "pn" ]; then
        env_chunk_id=$((task_id % ENV_CHUNKS))
    else
        env_chunk_id=${task_id}
    fi

    local env_start env_stop
    IFS=' ' read -r env_start env_stop < <(env_range_for_chunk "${env_chunk_id}")

    if [ "${MODE}" = "evac" ]; then
        echo "${OUTPUTS_ROOT}/results_evac"
    elif [ "${MODE}" = "pn" ]; then
        echo "${OUTPUTS_ROOT}/results_pn_env_id_${env_start}_${env_stop}"
    else
        echo "${OUTPUTS_ROOT}/results_qpi_env_id_${env_start}_${env_stop}"
    fi
}

RUNNER="${WORKSPACE}/target/release/Runner"
MODE_FINGERPRINT=""
MODE_MANIFEST=""
QUERY_INPUT_FILE=""
ACTIVE_CHUNKS_CSV=""

if [ "${MODE}" != "evac" ]; then
    if [ ! -f "${CONTRACT_HELPER}" ]; then
        echo "ERROR: Stage 2 contract helper not found: ${CONTRACT_HELPER}" >&2
        exit 1
    fi
    if ! command -v "${PYTHON_BIN}" >/dev/null 2>&1; then
        echo "ERROR: Python interpreter not found: ${PYTHON_BIN}" >&2
        exit 1
    fi

    QUERY_INPUT_FILE=$("${PYTHON_BIN}" "${CONTRACT_HELPER}" resolve-config \
        --q-model-config "${Q_MODEL_CONFIG}" \
        --runner-root "${RUNNER_ROOT}" \
        --key query)
    ENV_DICT=$("${PYTHON_BIN}" "${CONTRACT_HELPER}" resolve-config \
        --q-model-config "${Q_MODEL_CONFIG}" \
        --runner-root "${RUNNER_ROOT}" \
        --key env_dict)
    validate_env_dict "${ENV_DICT}" "${TOTAL_ENV_IDS}"
    "${PYTHON_BIN}" "${CONTRACT_HELPER}" validate-representation \
        --query "${QUERY_INPUT_FILE}" \
        --env-dict "${ENV_DICT}" \
        --hla-environment-representation "${HLA_ENVIRONMENT_REPRESENTATION}" \
        --total-env-ids "${TOTAL_ENV_IDS}"
    ACTIVE_CHUNKS_CSV=$("${PYTHON_BIN}" "${CONTRACT_HELPER}" active-chunks \
        --query "${QUERY_INPUT_FILE}" \
        --total-env-ids "${TOTAL_ENV_IDS}" \
        --env-chunks "${ENV_CHUNKS}")

    if [ -z "${ACTIVE_CHUNKS_CSV}" ]; then
        echo "ERROR: the authoritative query file contains no active environment chunks: ${QUERY_INPUT_FILE}" >&2
        exit 1
    fi

    if [ "${PN_SHARDED}" -eq 1 ]; then
        if [ "${DRY_RUN}" -eq 1 ]; then
            PN_SHARD_TMP_DIR=$(mktemp -d)
            trap 'rm -rf "${PN_SHARD_TMP_DIR}"' EXIT
            PN_SHARD_PLAN="${PN_SHARD_TMP_DIR}/pn_shard_plan.json"
        else
            mkdir -p "${OUTPUTS_ROOT}"
            PN_SHARD_PLAN="${OUTPUTS_ROOT}/pn_shard_plan.json"
        fi
        "${PYTHON_BIN}" "${CONTRACT_HELPER}" build-pn-shard-plan \
            --query "${QUERY_INPUT_FILE}" \
            --output "${PN_SHARD_PLAN}" \
            --total-env-ids "${TOTAL_ENV_IDS}" \
            --env-chunks "${ENV_CHUNKS}" \
            --total-params "${TOTAL_PARAMS}" \
            --param-chunks "${PARAM_CHUNKS}" \
            --target-rows "${PN_PEPTIDES_PER_SHARD}"
    fi

    if [ "${DRY_RUN}" -eq 1 ]; then
        MODE_FINGERPRINT="DRY_RUN"
        MODE_MANIFEST="${OUTPUTS_ROOT}/${MODE}_manifest.json"
    else
        mkdir -p "${OUT_ERR_DIR}"
        echo "Building Runner binary once before fingerprinting and submission..."
        cd "${WORKSPACE}"
        cargo build --release --bin Runner
        cd "${SCRIPT_DIR}"

        PREPARE_RUN_EXTRA_ARGS=()
        if [ -n "${REGIME_MANIFEST}" ]; then
            PREPARE_RUN_EXTRA_ARGS+=(--regime-manifest "${REGIME_MANIFEST}")
        fi
        if [ "${DATASET}" = "NCI" ] && { [ "${MODE}" = "pn" ] || [ "${MODE}" = "qpi" ]; }; then
            PREPARE_RUN_EXTRA_ARGS+=(--allow-operational-provenance-change)
        fi

        IFS=$'\t' read -r MODE_FINGERPRINT QUERY_INPUT_FILE MODE_MANIFEST < <(
            "${PYTHON_BIN}" "${CONTRACT_HELPER}" prepare-run \
                --run-root "${OUTPUTS_ROOT}" \
                --run-id "${RUN_ID}" \
                --dataset "${DATASET}" \
                --mode "${MODE}" \
                --runner-root "${RUNNER_ROOT}" \
                --runner "${RUNNER}" \
                --q-model-config "${Q_MODEL_CONFIG}" \
                --submit-script "${SCRIPT_DIR}/submit_stage2_new.sh" \
                --run-script "${RUN_SCRIPT}" \
                --contract-helper "${CONTRACT_HELPER}" \
                --total-env-ids "${TOTAL_ENV_IDS}" \
                --env-chunks "${ENV_CHUNKS}" \
                --total-params "${TOTAL_PARAMS}" \
                --param-chunks "${PARAM_CHUNKS}" \
                --param-file "${PARAM_FILE}" \
                --mn-tuples "${MN_TUPLES_FILE}" \
                --pmhc-config "${PMHC_CONFIG}" \
                --pmhc-parameters-file "${PMHC_PARAMETERS_FILE}" \
                --in-vitro "${COMPUTE_IN_VITRO}" \
                --peptide-conc "${IN_VITRO_PEPTIDE_CONC}" \
                --pn-hla-scope "${PN_HLA_SCOPE}" \
                --max-num-ps-values-log2 "${MAX_NUM_PS_VALUES_LOG2}" \
                --hla-environment-representation "${HLA_ENVIRONMENT_REPRESENTATION}" \
                --representation-crosswalk "${REPRESENTATION_CROSSWALK}" \
                --pn-peptides-per-shard "${PN_PEPTIDES_PER_SHARD}" \
                --pn-shard-plan "${PN_SHARD_PLAN}" \
                --finalize-script "${FINALIZE_SHARDS_SCRIPT}" \
                "${PREPARE_RUN_EXTRA_ARGS[@]}"
        )
    fi
fi

task_env_chunk_id() {
    local task_id=$1
    if [ "${MODE}" = "pn" ]; then
        echo $((task_id % ENV_CHUNKS))
    elif [ "${MODE}" = "evac" ]; then
        echo 0
    else
        echo "${task_id}"
    fi
}

chunk_is_active() {
    local chunk_id=$1
    if [ "${MODE}" = "evac" ]; then
        return 0
    fi
    case ",${ACTIVE_CHUNKS_CSV}," in
        *",${chunk_id},"*) return 0 ;;
        *) return 1 ;;
    esac
}

candidate_task_ids=()
submit_task_ids=()
parent_task_ids=()
if [ "${PN_SHARDED}" -eq 1 ]; then
    SELECTED_ENV_CHUNKS_CSV="${ACTIVE_CHUNKS_CSV}"
    if [ -n "${TARGET_ENV_IDS}" ]; then
        SELECTED_ENV_CHUNKS_CSV=$(expand_env_id_list_to_chunks "${TARGET_ENV_IDS}" | paste -sd, -)
    fi
    candidate_csv=$("${PYTHON_BIN}" "${CONTRACT_HELPER}" pn-shard-task-ids \
        --plan "${PN_SHARD_PLAN}" --run-root "${OUTPUTS_ROOT}" \
        --fingerprint "${MODE_FINGERPRINT}" --env-chunks "${SELECTED_ENV_CHUNKS_CSV}")
    if [ -n "${candidate_csv}" ]; then
        IFS=',' read -r -a candidate_task_ids <<< "${candidate_csv}"
    fi
    if [ "${MISSING_ONLY}" -eq 1 ]; then
        submit_csv=$("${PYTHON_BIN}" "${CONTRACT_HELPER}" pn-shard-task-ids \
            --plan "${PN_SHARD_PLAN}" --run-root "${OUTPUTS_ROOT}" \
            --fingerprint "${MODE_FINGERPRINT}" --env-chunks "${SELECTED_ENV_CHUNKS_CSV}" \
            --missing-only)
    else
        submit_csv="${candidate_csv}"
    fi
    if [ -n "${submit_csv}" ]; then
        IFS=',' read -r -a submit_task_ids <<< "${submit_csv}"
    fi
    if [ "${MISSING_ONLY}" -eq 1 ]; then
        parent_csv=$("${PYTHON_BIN}" "${CONTRACT_HELPER}" pn-shard-parent-ids \
            --plan "${PN_SHARD_PLAN}" --run-root "${OUTPUTS_ROOT}" \
            --fingerprint "${MODE_FINGERPRINT}" --env-chunks "${SELECTED_ENV_CHUNKS_CSV}" \
            --missing-only)
    else
        parent_csv=$("${PYTHON_BIN}" "${CONTRACT_HELPER}" pn-shard-parent-ids \
            --plan "${PN_SHARD_PLAN}" --run-root "${OUTPUTS_ROOT}" \
            --fingerprint "${MODE_FINGERPRINT}" --env-chunks "${SELECTED_ENV_CHUNKS_CSV}")
    fi
    if [ -n "${parent_csv}" ]; then
        IFS=',' read -r -a parent_task_ids <<< "${parent_csv}"
    fi
else
    while IFS= read -r task_id; do
        env_chunk_id=$(task_env_chunk_id "${task_id}")
        if chunk_is_active "${env_chunk_id}"; then
            candidate_task_ids+=("${task_id}")
        fi
    done < <(build_candidate_task_ids | sort -n | uniq)

    if [ "${#candidate_task_ids[@]}" -eq 0 ]; then
        echo "ERROR: no candidate tasks resolved." >&2
        exit 1
    fi

    if [ "${MISSING_ONLY}" -eq 1 ]; then
        for task_id in "${candidate_task_ids[@]}"; do
            output_dir=$(task_output_dir "${task_id}")
            if [ "${MODE}" = "evac" ]; then
                done_file="${output_dir}/TASK_${task_id}.done"
                if [ ! -f "${done_file}" ]; then
                    submit_task_ids+=("${task_id}")
                fi
                continue
            fi

            done_file="${output_dir}/TASK_${task_id}.done.json"
            if ! "${PYTHON_BIN}" "${CONTRACT_HELPER}" task-complete \
                --manifest "${done_file}" \
                --fingerprint "${MODE_FINGERPRINT}" \
                --run-root "${OUTPUTS_ROOT}" >/dev/null 2>&1; then
                submit_task_ids+=("${task_id}")
            fi
        done
    else
        submit_task_ids=("${candidate_task_ids[@]}")
    fi
fi

if [ "${#submit_task_ids[@]}" -eq 0 ] && [ "${#parent_task_ids[@]}" -eq 0 ]; then
    echo "All selected ${DATASET} ${MODE} tasks have valid matching completion manifests. Nothing to submit."
    exit 0
fi

if [ "${MODE}" = "pn" ]; then
    JOB_NAME="stage2_${DATASET}_${MODE}_${HLA_ENVIRONMENT_REPRESENTATION}_${PN_HLA_SCOPE}"
else
    JOB_NAME="stage2_${DATASET}_${MODE}_${HLA_ENVIRONMENT_REPRESENTATION}"
fi
NCI_PN_TIERED=0
LOW_MEM_TASK_IDS=()
HIGH_MEM_TASK_IDS=()
LOW_ARRAY_SPEC=""
HIGH_ARRAY_SPEC=""
LOW_CONCURRENCY=0
HIGH_CONCURRENCY=0

# NCI PN memory use is determined by the environment chunk. In array 2611574,
# these chunks completed consistently at 82G across parameter chunks 0-6; all
# other NCI environment chunks consistently exceeded that allocation.
is_nci_pn_low_memory_chunk() {
    case "$1" in
        0|2|5|6|11|12|16|18|26|27|28|29|31|33|35) return 0 ;;
        *) return 1 ;;
    esac
}

if [ "${DATASET}" = "NCI" ] && [ "${MODE}" = "pn" ] && \
   [ "${HLA_ENVIRONMENT_REPRESENTATION}" = "full" ] && [ -z "${MEM_OVERRIDE}" ]; then
    if [ "${ENV_CHUNKS}" -ne 43 ]; then
        echo "ERROR: automatic NCI PN memory tiers are calibrated for ENV_CHUNKS=43; use --mem for an explicit uniform request." >&2
        exit 1
    fi
    NCI_PN_TIERED=1
    for task_id in "${submit_task_ids[@]}"; do
        env_chunk_id=$((task_id % ENV_CHUNKS))
        if is_nci_pn_low_memory_chunk "${env_chunk_id}"; then
            LOW_MEM_TASK_IDS+=("${task_id}")
        else
            HIGH_MEM_TASK_IDS+=("${task_id}")
        fi
    done

    if ! printf '%s' "${ARRAY_CONCURRENCY}" | grep -Eq '^[1-9][0-9]*$'; then
        echo "ERROR: --array-concurrency must be a positive integer." >&2
        exit 1
    fi

    if [ "${#LOW_MEM_TASK_IDS[@]}" -gt 0 ]; then
        LOW_ARRAY_SPEC=$(compress_to_ranges "${LOW_MEM_TASK_IDS[@]}")
    fi
    if [ "${#HIGH_MEM_TASK_IDS[@]}" -gt 0 ]; then
        HIGH_ARRAY_SPEC=$(compress_to_ranges "${HIGH_MEM_TASK_IDS[@]}")
    fi

    if [ "${#LOW_MEM_TASK_IDS[@]}" -gt 0 ] && [ "${#HIGH_MEM_TASK_IDS[@]}" -gt 0 ]; then
        if [ "${ARRAY_CONCURRENCY}" -lt 2 ]; then
            echo "ERROR: tiered NCI PN submission requires --array-concurrency >= 2." >&2
            exit 1
        fi
        LOW_CONCURRENCY=$((ARRAY_CONCURRENCY * ${#LOW_MEM_TASK_IDS[@]} / ${#submit_task_ids[@]}))
        if [ "${LOW_CONCURRENCY}" -lt 1 ]; then
            LOW_CONCURRENCY=1
        elif [ "${LOW_CONCURRENCY}" -ge "${ARRAY_CONCURRENCY}" ]; then
            LOW_CONCURRENCY=$((ARRAY_CONCURRENCY - 1))
        fi
        HIGH_CONCURRENCY=$((ARRAY_CONCURRENCY - LOW_CONCURRENCY))
    elif [ "${#LOW_MEM_TASK_IDS[@]}" -gt 0 ]; then
        LOW_CONCURRENCY=${ARRAY_CONCURRENCY}
    else
        HIGH_CONCURRENCY=${ARRAY_CONCURRENCY}
    fi
else
    ARRAY_SPEC=""
    if [ "${#submit_task_ids[@]}" -gt 0 ]; then
        ARRAY_SPEC=$(compress_to_ranges "${submit_task_ids[@]}")
    fi
fi
PARENT_ARRAY_SPEC=""
if [ "${#parent_task_ids[@]}" -gt 0 ]; then
    PARENT_ARRAY_SPEC=$(compress_to_ranges "${parent_task_ids[@]}")
fi

echo "============================================================"
echo "Stage 2 submission preflight"
echo "Dataset:             ${DATASET}"
echo "Mode:                ${MODE}"
echo "Grid profile:        ${GRID_PROFILE}"
echo "HLA representation:  ${HLA_ENVIRONMENT_REPRESENTATION}"
echo "PN HLA scope:        ${PN_HLA_SCOPE}"
echo "Max p_s log2:        ${MAX_NUM_PS_VALUES_LOG2}"
echo "Execution mode:      ${EXECUTION_MODE}"
echo "Env dict:            ${ENV_DICT:-not configured}"
echo "Q model config:      ${Q_MODEL_CONFIG:-not configured}"
echo "pMHC config:         ${PMHC_CONFIG:-not configured}"
echo "pMHC parameters:     ${PMHC_PARAMETERS_FILE:-not configured}"
echo "Vaccine config:      ${VACCINE_CONFIG:-not configured}"
echo "Total env IDs:       ${TOTAL_ENV_IDS}"
echo "Env chunks:          ${ENV_CHUNKS}"
echo "Target env IDs:      ${TARGET_ENV_IDS:-all}"
echo "Run ID:              ${RUN_ID}"
echo "Active env chunks:   ${ACTIVE_CHUNKS_CSV:-not-applicable}"
echo "Query input:         ${QUERY_INPUT_FILE:-not-applicable}"
echo "Mode fingerprint:    ${MODE_FINGERPRINT:-not-used-for-evac}"
echo "Mode manifest:       ${MODE_MANIFEST:-not-applicable}"
echo "Parameter file:      ${PARAM_FILE}"
echo "Parameter SHA-256:   ${PARAM_FILE_SHA256}"
echo "M/N file:            ${MN_TUPLES_FILE}"
echo "M/N SHA-256:         ${MN_TUPLES_FILE_SHA256}"
echo "Representation map:  ${REPRESENTATION_CROSSWALK:-not-applicable}"
echo "PN shard target:      ${PN_PEPTIDES_PER_SHARD:-0}"
echo "PN shard plan:        ${PN_SHARD_PLAN:-not-applicable}"
echo "Candidate tasks:     ${#candidate_task_ids[@]}"
echo "Tasks to submit:     ${#submit_task_ids[@]}"
if [ "${PN_SHARDED}" -eq 1 ]; then
    echo "Parent merge tasks:  ${#parent_task_ids[@]}"
fi
echo "Outputs root:        ${OUTPUTS_ROOT}"
if [ "${NCI_PN_TIERED}" -eq 1 ]; then
    echo "NCI PN memory tiers: enabled"
    if [ "${#LOW_MEM_TASK_IDS[@]}" -gt 0 ]; then
        echo "Low-memory tier:     ${#LOW_MEM_TASK_IDS[@]} tasks, ${NCI_PN_LOW_MEM}, array ${LOW_ARRAY_SPEC}%${LOW_CONCURRENCY}"
    fi
    if [ "${#HIGH_MEM_TASK_IDS[@]}" -gt 0 ]; then
        echo "High-memory tier:    ${#HIGH_MEM_TASK_IDS[@]} tasks, ${NCI_PN_HIGH_MEM}, array ${HIGH_ARRAY_SPEC}%${HIGH_CONCURRENCY}"
    fi
    echo "Combined concurrency: ${ARRAY_CONCURRENCY}"
else
    echo "Array spec:          ${ARRAY_SPEC:-no compute tasks}%${ARRAY_CONCURRENCY}"
    echo "Memory:              ${MEM}"
    echo "Job name:            ${JOB_NAME}"
fi
echo "Out/err directory:   ${OUT_ERR_DIR}"
echo "Compute in vitro:    ${COMPUTE_IN_VITRO}"
if [ "${COMPUTE_IN_VITRO}" = "1" ]; then
    echo "Peptide conc:        ${IN_VITRO_PEPTIDE_CONC} nM"
fi
echo "============================================================"

if [ "${MODE}" = "pn" ]; then
    params_per_chunk=$((TOTAL_PARAMS / PARAM_CHUNKS))
    for ((param_chunk=0; param_chunk<PARAM_CHUNKS; param_chunk++)); do
        param_start=$((param_chunk * params_per_chunk))
        param_stop=$((param_start + params_per_chunk))
        if [ "${param_chunk}" -eq $((PARAM_CHUNKS - 1)) ]; then
            param_stop=${TOTAL_PARAMS}
        fi
        echo "Parameter chunk ${param_chunk}: rows [${param_start}, ${param_stop})"
    done
fi

SLURM_EXPORT_SPEC="ALL,DATASET=${DATASET},STAGE2_MODE=${MODE},HLA_ENVIRONMENT_REPRESENTATION=${HLA_ENVIRONMENT_REPRESENTATION},PN_HLA_SCOPE=${PN_HLA_SCOPE},MAX_NUM_PS_VALUES_LOG2=${MAX_NUM_PS_VALUES_LOG2},RUN_ID=${RUN_ID},MODE_FINGERPRINT=${MODE_FINGERPRINT},MODE_MANIFEST=${MODE_MANIFEST},QUERY_INPUT_FILE=${QUERY_INPUT_FILE},CONTRACT_HELPER=${CONTRACT_HELPER},PYTHON_BIN=${PYTHON_BIN},PARAM_FILE=${PARAM_FILE},MN_TUPLES_FILE=${MN_TUPLES_FILE},PARAM_FILE_SHA256=${PARAM_FILE_SHA256},MN_TUPLES_FILE_SHA256=${MN_TUPLES_FILE_SHA256},PMHC_CONFIG=${PMHC_CONFIG},PMHC_PARAMETERS_FILE=${PMHC_PARAMETERS_FILE},COMPUTE_PN=${COMPUTE_PN},COMPUTE_Q=${COMPUTE_Q},COMPUTE_PI=${COMPUTE_PI},COMPUTE_EVAC=${COMPUTE_EVAC},EXECUTION_MODE=${EXECUTION_MODE},TOTAL_PARAMS=${TOTAL_PARAMS},PARAM_CHUNKS=${PARAM_CHUNKS},TOTAL_ENV_IDS=${TOTAL_ENV_IDS},ENV_CHUNKS=${ENV_CHUNKS},RUNNER=${RUNNER},RUNNER_ROOT=${RUNNER_ROOT},OUTPUTS_ROOT=${OUTPUTS_ROOT},COMPUTE_IN_VITRO=${COMPUTE_IN_VITRO},IN_VITRO_PEPTIDE_CONC=${IN_VITRO_PEPTIDE_CONC},Q_MODEL_CONFIG=${Q_MODEL_CONFIG},EXTERNAL_VALIDATION_INPUT_ROOT=${EXTERNAL_VALIDATION_INPUT_ROOT},VACCINE_CONFIG=${VACCINE_CONFIG},PN_SHARD_PLAN=${PN_SHARD_PLAN},SURVIVOR_GRID_HELPER=${SURVIVOR_GRID_HELPER},GRID_PROFILE=${GRID_PROFILE}"

if [ "${DRY_RUN}" -eq 1 ]; then
    echo "Resolved worker submission command:"
    if [ "${PN_SHARDED}" -eq 1 ]; then
        if [ -n "${ARRAY_SPEC}" ]; then
            printf 'sbatch --job-name=%q --partition=%q --nodes=1 --ntasks=1 --cpus-per-task=%q --mem=%q --time=%q --array=%q --export=%q %q\n' \
                "${JOB_NAME}_shards" "${PARTITION}" "${CPUS_PER_TASK}" "${MEM}" "${TIME_LIMIT}" \
                "${ARRAY_SPEC}%${ARRAY_CONCURRENCY}" "${SLURM_EXPORT_SPEC}" "${RUN_SCRIPT}"
        fi
        printf 'sbatch --job-name=%q --partition=%q --nodes=1 --ntasks=1 --cpus-per-task=1 --mem=%q --time=%q --array=%q --export=%q %q\n' \
            "${JOB_NAME}_merge" "${PARTITION}" "${PN_SHARD_MERGE_MEM}" "${PN_SHARD_MERGE_TIME}" \
            "${PARENT_ARRAY_SPEC}%${ARRAY_CONCURRENCY}" "${SLURM_EXPORT_SPEC}" "${FINALIZE_SHARDS_SCRIPT}"
    elif [ "${NCI_PN_TIERED}" -eq 1 ]; then
        echo "  NCI full-PN tiered arrays use the two array specifications printed above."
    else
        printf 'sbatch --job-name=%q --partition=%q --nodes=1 --ntasks=1 --cpus-per-task=%q --mem=%q --time=%q --array=%q --export=%q %q\n' \
            "${JOB_NAME}" "${PARTITION}" "${CPUS_PER_TASK}" "${MEM}" "${TIME_LIMIT}" \
            "${ARRAY_SPEC}%${ARRAY_CONCURRENCY}" "${SLURM_EXPORT_SPEC}" "${RUN_SCRIPT}"
    fi
    echo "Dry run requested; skipping manifest creation, cargo build, and sbatch."
    exit 0
fi

mkdir -p "${OUT_ERR_DIR}"

if [ "${MODE}" = "evac" ]; then
    echo "Building Runner binary once before E-vac submission..."
    cd "${WORKSPACE}"
    cargo build --release --bin Runner
    cd "${SCRIPT_DIR}"
fi

submit_slurm_array() {
    local job_name=$1
    local array_spec=$2
    local concurrency=$3
    local memory=$4
    local out_file="${OUT_ERR_DIR}/${job_name}_%A_%a.out"
    local err_file="${OUT_ERR_DIR}/${job_name}_%A_%a.err"

    echo "Submitting ${job_name}: ${array_spec}%${concurrency}, memory=${memory}"
    sbatch \
        --job-name="${job_name}" \
        --partition="${PARTITION}" \
        --nodes=1 \
        --ntasks=1 \
        --cpus-per-task="${CPUS_PER_TASK}" \
        --mem="${memory}" \
        --time="${TIME_LIMIT}" \
        --output="${out_file}" \
        --error="${err_file}" \
        --array="${array_spec}%${concurrency}" \
        --export="${SLURM_EXPORT_SPEC}" \
        "${RUN_SCRIPT}"
}

if [ "${PN_SHARDED}" -eq 1 ]; then
    SHARD_JOB_ID=""
    if [ -n "${ARRAY_SPEC}" ]; then
        echo "Submitting ${JOB_NAME}_shards: ${ARRAY_SPEC}%${ARRAY_CONCURRENCY}, memory=${MEM}"
        SHARD_JOB_ID=$(sbatch --parsable \
            --job-name="${JOB_NAME}_shards" \
            --partition="${PARTITION}" \
            --nodes=1 \
            --ntasks=1 \
            --cpus-per-task="${CPUS_PER_TASK}" \
            --mem="${MEM}" \
            --time="${TIME_LIMIT}" \
            --output="${OUT_ERR_DIR}/${JOB_NAME}_shards_%A_%a.out" \
            --error="${OUT_ERR_DIR}/${JOB_NAME}_shards_%A_%a.err" \
            --array="${ARRAY_SPEC}%${ARRAY_CONCURRENCY}" \
            --export="${SLURM_EXPORT_SPEC}" \
            "${RUN_SCRIPT}")
        SHARD_JOB_ID=${SHARD_JOB_ID%%;*}
        echo "Submitted P/N shard array job ${SHARD_JOB_ID}."
    fi

    echo "Submitting ${JOB_NAME}_merge: ${PARENT_ARRAY_SPEC}%${ARRAY_CONCURRENCY}, memory=${PN_SHARD_MERGE_MEM}"
    if [ -n "${SHARD_JOB_ID}" ]; then
        MERGE_JOB_ID=$(sbatch --parsable \
            --job-name="${JOB_NAME}_merge" --partition="${PARTITION}" \
            --nodes=1 --ntasks=1 --cpus-per-task=1 \
            --mem="${PN_SHARD_MERGE_MEM}" --time="${PN_SHARD_MERGE_TIME}" \
            --output="${OUT_ERR_DIR}/${JOB_NAME}_merge_%A_%a.out" \
            --error="${OUT_ERR_DIR}/${JOB_NAME}_merge_%A_%a.err" \
            --array="${PARENT_ARRAY_SPEC}%${ARRAY_CONCURRENCY}" \
            --dependency="afterok:${SHARD_JOB_ID}" \
            --export="${SLURM_EXPORT_SPEC}" "${FINALIZE_SHARDS_SCRIPT}")
    else
        MERGE_JOB_ID=$(sbatch --parsable \
            --job-name="${JOB_NAME}_merge" --partition="${PARTITION}" \
            --nodes=1 --ntasks=1 --cpus-per-task=1 \
            --mem="${PN_SHARD_MERGE_MEM}" --time="${PN_SHARD_MERGE_TIME}" \
            --output="${OUT_ERR_DIR}/${JOB_NAME}_merge_%A_%a.out" \
            --error="${OUT_ERR_DIR}/${JOB_NAME}_merge_%A_%a.err" \
            --array="${PARENT_ARRAY_SPEC}%${ARRAY_CONCURRENCY}" \
            --export="${SLURM_EXPORT_SPEC}" "${FINALIZE_SHARDS_SCRIPT}")
    fi
    echo "Submitted P/N merge array job ${MERGE_JOB_ID%%;*}."
elif [ "${NCI_PN_TIERED}" -eq 1 ]; then
    if [ "${#LOW_MEM_TASK_IDS[@]}" -gt 0 ]; then
        submit_slurm_array "${JOB_NAME}_lowmem" "${LOW_ARRAY_SPEC}" "${LOW_CONCURRENCY}" "${NCI_PN_LOW_MEM}"
    fi
    if [ "${#HIGH_MEM_TASK_IDS[@]}" -gt 0 ]; then
        submit_slurm_array "${JOB_NAME}_highmem" "${HIGH_ARRAY_SPEC}" "${HIGH_CONCURRENCY}" "${NCI_PN_HIGH_MEM}"
    fi
else
    submit_slurm_array "${JOB_NAME}" "${ARRAY_SPEC}" "${ARRAY_CONCURRENCY}" "${MEM}"
fi
