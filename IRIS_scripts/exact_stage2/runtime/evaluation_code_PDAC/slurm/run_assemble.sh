#!/bin/bash

# ============================================================================
# Stage 3a: Data Assembly - Assemble F tensor from CSV outputs (long peptides)
# ============================================================================
# Reads all CSV files from Stage 2 outputs and creates unified Parquet files.
# Uses assemble_tensor_longpep which builds (nmer × HLA) observations from a
# mapping CSV instead of MT/WT label CSVs.
# This is a single job (not array) - runs once after all Stage 2 jobs complete.

#SBATCH --job-name=assemble_tensor_longpep
#SBATCH --partition=componc_cpu
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --mem=250G
#SBATCH --cpus-per-task=8
#SBATCH --time=4:00:00
#SBATCH --output=/home/thomm15/New_Approaches_Scripts/Out_and_Err/assemble_longpep_%j.out
#SBATCH --error=/home/thomm15/New_Approaches_Scripts/Out_and_Err/assemble_longpep_%j.err

# ============================================================================
# Configuration
# ============================================================================
echo "Job started: $(date)"
echo "Node: $(hostname)"
echo "Job ID: $SLURM_JOB_ID"
echo "CPUs allocated: $SLURM_CPUS_PER_TASK"

# Set Rayon threads for Rust parallelism
export RAYON_NUM_THREADS=$SLURM_CPUS_PER_TASK

set -euo pipefail


# Paths
ROOT_DIR="${WORKSPACE:-/data1/lukszam/Marcus/New_Approaches}"
LAUNCHER_DIR="${LAUNCHER_DIR:-/home/thomm15/New_Approaches_Scripts}"
INPUT_PACKAGE="${COMPLETE_F_INPUT_PACKAGE:-${LAUNCHER_DIR}/complete_f_input_package}"
CONTRACT_HELPER="${LAUNCHER_DIR}/stage2_run_contract.py"
RUN_ID="${RUN_ID:?Export RUN_ID for the validated PDAC Stage 2 rerun before submitting assembly}"
COHORT_ROOT="${INPUT_PACKAGE}"
INSTALL_MANIFEST="${INPUT_PACKAGE}/manifest.json"
RESULTS_DIR="${ROOT_DIR}/Runner/Outputs_PDAC/runs/${RUN_ID}"

if [ ! -d "${RESULTS_DIR}" ]; then
    echo "ERROR: Stage-2 results dir not found: ${RESULTS_DIR}"
    exit 1
fi

RUN_MANIFEST="${RESULTS_DIR}/run_manifest.json"
QPI_MANIFEST="${RESULTS_DIR}/qpi_manifest.json"
PN_MANIFEST="${RESULTS_DIR}/pn_manifest.json"
for manifest in "${RUN_MANIFEST}" "${QPI_MANIFEST}" "${PN_MANIFEST}"; do
    if [ ! -f "${manifest}" ]; then
        echo "ERROR: Stage-2 manifest not found: ${manifest}"
        exit 1
    fi
done

mapfile -t RUN_CONTRACT < <(
    python3 -c \
        'import json, pathlib, sys; d=json.loads(pathlib.Path(sys.argv[1]).read_text()); print(d["dataset"]); print(d["run_id"]); print(d["hla_environment_representation"]); print(d["query_input_file"])' \
        "${RUN_MANIFEST}"
)
if [ "${#RUN_CONTRACT[@]}" -ne 4 ]; then
    echo "ERROR: could not resolve the Stage-2 common contract from ${RUN_MANIFEST}"
    exit 1
fi
MANIFEST_DATASET="${RUN_CONTRACT[0]}"
MANIFEST_RUN_ID="${RUN_CONTRACT[1]}"
HLA_REPRESENTATION="${RUN_CONTRACT[2]}"
QUERY_PEPTIDES="${RUN_CONTRACT[3]}"
if [ "${MANIFEST_DATASET}" != "PDAC" ]; then
    echo "ERROR: PDAC assembly received manifest for ${MANIFEST_DATASET}"
    exit 1
fi
if [ "${MANIFEST_RUN_ID}" != "${RUN_ID}" ]; then
    echo "ERROR: RUN_ID=${RUN_ID} but run manifest records ${MANIFEST_RUN_ID}"
    exit 1
fi
case "${HLA_REPRESENTATION}" in
  full) MAPPING="${COHORT_ROOT}/pdac_full_longpep_mapping.csv" ;;
  mono) MAPPING="${COHORT_ROOT}/pdac_mono_longpep_mapping.csv" ;;
  *)
    echo "ERROR: unsupported HLA representation in run manifest: ${HLA_REPRESENTATION}"
    exit 1
    ;;
esac
if [ ! -f "${MAPPING}" ]; then
    echo "ERROR: ${HLA_REPRESENTATION} mapping CSV not found: ${MAPPING}"
    exit 1
fi
if [ -z "${QUERY_PEPTIDES}" ] || [ ! -f "${QUERY_PEPTIDES}" ]; then
    echo "ERROR: query_input_file from ${RUN_MANIFEST} is not readable: ${QUERY_PEPTIDES}"
    exit 1
fi
if [ ! -f "${CONTRACT_HELPER}" ]; then
    echo "ERROR: Stage-2 contract helper not found: ${CONTRACT_HELPER}"
    exit 1
fi

# Fail closed: the selected mapping and the Stage-2 query must match the
# cohort's complete-F install manifest exactly (guards against stale or
# *_deprecated inputs sitting beside the live files).
if [ ! -f "${INSTALL_MANIFEST}" ]; then
    echo "ERROR: complete-F inputs manifest not found: ${INSTALL_MANIFEST}"
    exit 1
fi
echo "Verifying mapping and query against complete-F manifest: ${INSTALL_MANIFEST}"
python3 - "${INSTALL_MANIFEST}" "${MAPPING}" "${QUERY_PEPTIDES}" <<'PYEOF'
import hashlib, json, os, sys
manifest_path, *inputs = sys.argv[1:]
files = json.load(open(manifest_path)).get("output_files", {})
def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()
ok = True
for path in inputs:
    name = os.path.basename(path)
    entry = files.get(name)
    if entry is None:
        print(f"ERROR: {name} not listed in complete-F manifest"); ok = False; continue
    expected = entry["sha256"] if isinstance(entry, dict) else entry
    actual = sha256(path)
    if expected != actual:
        print(f"ERROR: SHA-256 mismatch for {name}"); print(f"  expected {expected}"); print(f"  actual   {actual}"); ok = False
    else:
        print(f"OK: {name} matches complete-F manifest ({actual[:12]}...)")
sys.exit(0 if ok else 1)
PYEOF

echo "Auditing all Q/Pi task manifests and output hashes..."
python3 "${CONTRACT_HELPER}" audit-run \
    --run-root "${RESULTS_DIR}" \
    --mode-manifest "${QPI_MANIFEST}" \
    --verify-hashes
echo "Auditing all P/N task manifests and output hashes..."
python3 "${CONTRACT_HELPER}" audit-run \
    --run-root "${RESULTS_DIR}" \
    --mode-manifest "${PN_MANIFEST}" \
    --verify-hashes
OUTPUT_DIR="${ROOT_DIR}/Runner/evaluation_PDAC/runs/${RUN_ID}"
OUTPUT_TENSOR="${OUTPUT_DIR}/f_tensor.parquet"

# Create output directory
mkdir -p "${OUTPUT_DIR}"

# Navigate to evaluation workspace
cd "${LAUNCHER_DIR}/evaluation_code_PDAC"

# ============================================================================
# Build and run assembly
# ============================================================================
echo "Building assemble_tensor_longpep binary..."
cargo build --release --bin assemble_tensor_longpep

echo "Running data assembly..."
echo "Run ID: ${RUN_ID}"
echo "Representation: ${HLA_REPRESENTATION}"
echo "Results: ${RESULTS_DIR}"
echo "Mapping: ${MAPPING}"
echo "Query roster: ${QUERY_PEPTIDES}"
echo "NOTE: expected HLAs come only from the filtered query roster; missing values abort assembly"

"${LAUNCHER_DIR}/evaluation_code_PDAC/target/release/assemble_tensor_longpep" \
    --results-dirs "${RESULTS_DIR}" \
    --dataset PDAC \
    --run-id "${RUN_ID}" \
    --run-manifest "${RUN_MANIFEST}" \
    --qpi-manifest "${QPI_MANIFEST}" \
    --pn-manifest "${PN_MANIFEST}" \
    --mapping "${MAPPING}" \
    --query-peptides "${QUERY_PEPTIDES}" \
    --output "${OUTPUT_TENSOR}" \
    --threads "${SLURM_CPUS_PER_TASK}"

echo "Job finished: $(date)"
