# Complete-F full-roster regeneration and production replacement plan

## Purpose

This plan replaces the current coverage-by-omission production lineage with a
complete Level-1 \(F\) tensor for every declared PDAC, COVID SPIKE, and COVID
NONSPIKE 9--12-mer/HLA tuple. It then rebuilds every dependent artifact:

```text
complete candidate/query package
  -> complete full/focal/mono F tensors
  -> fixed-L2 transfers and 60-system bundles
  -> joint (alpha, q, c, kappa) selection
  -> combined 70-system bundles
  -> cluster_inputs package
  -> PR/CNAP and AUROC tournament
```

The existing `artifacts/iris_fullroster_production_20260820` and
`cluster_inputs/full_roster_selfgated_dual_selection_v1` trees are invalid as
production evidence because they descend from uncomputed candidates encoded as
floors. Do not delete them until the replacement lineage has passed all gates
below. At the end, remove them rather than maintaining compatibility paths or
parallel production versions in the repository.

## Non-negotiable contracts

1. The roster is a set of unique
   `(endpoint identity, minimal epitope, normalized HLA)` tuples, not a
   multiset.
2. Every declared tuple is computed at Level 1. Missing computation is an
   error, not `log(1e-12)`.
3. `log(1e-12)` is permitted only for a completed biological zero signal.
4. The NCI-selected Level-1 geometry, decision regime, and exposure/tau grid
   remain frozen. Do not reselect Level 1.
5. The full `(alpha, q, c, kappa)` grid is frozen before selection surfaces are
   inspected.
6. A new stage always writes to a new output directory. Never patch a
   checksum-linked artifact tree in place.
7. Old production artifacts are removed only after the new cluster bundle and
   tournament pilot validate successfully.

## Working paths

Set these once in a fresh shell. Use a run identifier that will remain attached
to all manifests and logs.

```bash
REPO_ROOT=/Users/thomm15/Documents/Supported_Model_Replacement
WORK_DATA_ROOT=/Users/thomm15/Work_Data
RUN_ID=complete_f_YYYYMMDD

STAGE_ROOT=/path/to/immutable_runs/${RUN_ID}
INPUT_BUNDLE=${WORK_DATA_ROOT}/IRIS_scripts/complete_f_input_package
PDAC_INPUT_ROOT=${WORK_DATA_ROOT}/Jayon
COVID_SPIKE_INPUT_ROOT=${WORK_DATA_ROOT}/Cansu_Covid_Spike
COVID_NONSPIKE_INPUT_ROOT=${WORK_DATA_ROOT}/Cansu_Covid_Nonspike
TENSOR_ROOT=${STAGE_ROOT}/tensors
TRANSFER_ROOT=${STAGE_ROOT}/full_roster_transfers
FIXED_BUNDLES=${STAGE_ROOT}/full_roster_fixed_l2
ADAPTIVE_RUN=${STAGE_ROOT}/adaptive_hillq_selection
COMBINED_BUNDLES=${STAGE_ROOT}/full_roster_70_systems
TOURNAMENT_RUN=${STAGE_ROOT}/tournament

INPUT_CONFIG=${STAGE_ROOT}/external_validation_inputs.complete_f.json
PIPELINE_CONFIG=${STAGE_ROOT}/iris_fullroster_pipeline.complete_f.json
ALPHA_VALUES='<PREREGISTERED_SPACE_SEPARATED_ALPHA_ORDERS>'

cd "${REPO_ROOT}"
mkdir -p "${STAGE_ROOT}"
```

Do not use a previous run directory for `STAGE_ROOT`.

## Phase 0 — Preserve an audit snapshot; do not delete yet

Record the state being superseded before changing code or data.

```bash
cd "${REPO_ROOT}"
git status --short
du -sh artifacts cluster_inputs
git ls-files artifacts cluster_inputs \
  > "${STAGE_ROOT}/superseded_tracked_files.txt"
shasum -a 256 \
  cluster_inputs/full_roster_selfgated_dual_selection_v1/pr/bundle_manifest.json \
  cluster_inputs/full_roster_selfgated_dual_selection_v1/roc/bundle_manifest.json \
  > "${STAGE_ROOT}/superseded_bundle_manifest_hashes.txt"
```

Relevant existing evidence:

- `artifacts/iris_fullroster_production_20260820/full_roster_inputs_v1/manifest.json`
- `cluster_inputs/README.md`
- `IRIS_scripts/docs/COMPLETE_F_CLUSTER_RUNBOOK.md`

**Gate 0:** the superseded file roster and manifest hashes exist under the new
staging root. Nothing has been deleted.

## Phase 1 — Make the repository complete-F-ready

This phase is required before generating the new query package. The current
`external_validation_inputs` implementation still derives `scoreable|floor`
from the old filtered query, and `iris_fullroster_pipeline` still accepts floor
mapping rows. Those are historical semantics, not the replacement production
contract.

### 1.1 Update the input builder

Primary code and tests:

- `supported_ap_code/crates/external_validation_inputs/src/builder.rs`
- `supported_ap_code/crates/external_validation_inputs/tests/end_to_end.rs`
- `IRIS_scripts/configs/input_packaging/external_validation_inputs.example.json`

Required behavior:

- Enumerate every endpoint-specific 9--12-mer/HLA tuple.
- Remove exact duplicate biological tuples before computation.
- Emit complete full and mono computational query rosters, not only the old
  gate-passing subset.
- Give every retained mapping row a computable representation-specific
  environment ID.
- Require production mappings to contain only `mapping_status=scoreable`.
- Retain zero-valued completed tuples as scoreable rows; do not convert them to
  omission floors.
- Update audits to report complete tuple counts, duplicates removed, completed
  zeros, and zero missing computations.
- Replace the old expected query/scoreable/floor counts and source hashes in a
  copied production configuration. Do not silently alter historical hashes.

Start the new configuration from the existing example:

```bash
cp IRIS_scripts/configs/input_packaging/external_validation_inputs.example.json "${INPUT_CONFIG}"
```

Edit `${INPUT_CONFIG}` only after the new complete-query sources and their
SHA-256 hashes are known.

### 1.2 Make the transfer pipeline fail closed on omission floors

Primary code and tests:

- `supported_ap_code/crates/iris_fullroster_pipeline/src/transfer.rs`
- `supported_ap_code/crates/iris_fullroster_pipeline/src/contract.rs`
- `supported_ap_code/crates/iris_fullroster_pipeline/tests/compact_pipeline.rs`

Required behavior:

- Reject a production mapping row whose status is not `scoreable`.
- Require every mapping row to resolve uniquely to a completed tensor
  observation and every selected tau value.
- Preserve a computed \(F=0\) as a real score of `log(1e-12)`.
- Record zero missing observations in the transfer manifest and per-job
  summaries.
- Preserve exact set deduplication before every fixed-L2 aggregate.

### 1.3 Make the entire adaptive grid explicit in the Slurm contract

The Rust selector already supports explicit `alpha`, `q`, `c`, and `kappa`
axes. The current Slurm wrapper requires alpha but relies on binary defaults for
the other axes. Before production, update these scripts so all grid and solver
settings are accepted, stored in `plan.env`/`run.env`, passed to planning, and
passed unchanged to final reduction:

- `IRIS_scripts/adaptive_selection/submit_adaptive_hillq_selection_slurm.sh`
- `IRIS_scripts/adaptive_selection/run_adaptive_hillq_plan.slurm`
- `IRIS_scripts/adaptive_selection/finalize_adaptive_hillq_selection_slurm.sh`

Expose and persist at least:

```text
--alpha-values
--q-values
--c-min --c-max --c-step
--kappa-min --kappa-max --kappa-points
--solver-absolute-tolerance --solver-max-iterations
```

The array worker need not receive these separately because it consumes the
immutable plan.

### 1.4 Update the tournament finalizer for the replacement system IDs

The selector and combined-bundle builder emit system IDs containing
`self_gated_power_hillq`, but the current tournament finalizer still tests for
the former `self_gated_hillq` IDs. Update:

- `IRIS_scripts/tournament/finalize_directed_round_robin_slurm.sh`

Replace both five-system arrays with the exact IDs emitted by the validated
combined bundle:

```text
<model>__self_gated_power_hillq_pdac_selected
<model>__self_gated_power_hillq_all_contexts_selected
```

Do not retain a fallback list of old IDs. Test that a 70-system replacement
bundle triggers both induced-view commands and that a malformed roster fails
the expected dimension/system-ID audit.

### 1.5 Reconcile documentation with the replacement contract

Remove production claims that uncomputed tuples may be represented as floors
or that the old Stage-2 tensors can be reused. Review at least:

- `IRIS_scripts/iris_fullroster_round_robin_plan.md`
- `supported_ap_code/crates/external_validation_inputs/README.md`
- `supported_ap_code/crates/iris_fullroster_pipeline/README.md`
- `supported_ap_code/crates/README.md`
- `supported_ap_code/crates/select_adaptive_hillq/README.md`

Useful stale-language audit:

```bash
rg -n 'floor rows|mapping_status.*floor|do not need to be recomputed|reuse.*tensor|omission-floor' \
  IRIS_scripts supported_ap_code/crates
```

Historical descriptions may remain only when unmistakably labeled as
superseded provenance.

### 1.6 Build and test the code boundary

```bash
cargo fmt --manifest-path supported_ap_code/Cargo.toml --all -- --check

cargo test --manifest-path supported_ap_code/Cargo.toml \
  --package external_validation_inputs \
  --package iris_fullroster_pipeline \
  --package select_adaptive_hillq \
  --package directed_round_robin_organizer

cargo clippy --manifest-path supported_ap_code/Cargo.toml \
  --workspace --all-targets -- -D warnings

bash -n \
  IRIS_scripts/adaptive_selection/submit_adaptive_hillq_selection_slurm.sh \
  IRIS_scripts/adaptive_selection/run_adaptive_hillq_plan.slurm \
  IRIS_scripts/adaptive_selection/run_adaptive_hillq_selection_array.slurm \
  IRIS_scripts/adaptive_selection/finalize_adaptive_hillq_selection_slurm.sh \
  IRIS_scripts/tournament/submit_directed_round_robin_slurm.sh \
  IRIS_scripts/tournament/run_directed_round_robin_array.slurm \
  IRIS_scripts/tournament/finalize_directed_round_robin_slurm.sh

cargo build --release --manifest-path supported_ap_code/Cargo.toml \
  --package external_validation_inputs \
  --package iris_fullroster_pipeline \
  --package select_adaptive_hillq \
  --package directed_round_robin_organizer --bins
```

**Gate 1:** tests and strict Clippy pass; the production builder and transfer
path reject omission floors; all adaptive grid dimensions appear in the
immutable Slurm plan contract.

## Phase 2 — Build the complete query/mapping package

Build into a new directory. The command refuses to overwrite an existing
package.

```bash
supported_ap_code/target/release/external_validation_inputs build \
  --config "${INPUT_CONFIG}" \
  --input-root "${WORK_DATA_ROOT}" \
  --output "${INPUT_BUNDLE}"

supported_ap_code/target/release/external_validation_inputs validate \
  --bundle "${INPUT_BUNDLE}"
```

Inspect the manifest and audit files:

```bash
find "${INPUT_BUNDLE}" -maxdepth 1 -type f -print | sort
rg -n '"floor|missing|duplicate|scoreable|query_rows' \
  "${INPUT_BUNDLE}/manifest.json" \
  "${INPUT_BUNDLE}"/*_inputs_audit.json
```

Install the manifest-validated dataset outputs directly into the established
cohort roots. These are stable paths, not run-ID directories:

```bash
for DATASET in PDAC COVID_SPIKE COVID_NONSPIKE; do
  python3 "${WORK_DATA_ROOT}/IRIS_scripts/validate_complete_f_stage2_inputs.py" \
    install \
    --bundle "${INPUT_BUNDLE}" \
    --dataset "${DATASET}" \
    --work-data-root "${WORK_DATA_ROOT}"
done
```

The resulting primary query paths are:

```text
${PDAC_INPUT_ROOT}/pdac_full_deduplicated_query.csv
${COVID_SPIKE_INPUT_ROOT}/covid_spike_full_deduplicated_query.csv
${COVID_NONSPIKE_INPUT_ROOT}/covid_nonspike_full_deduplicated_query.csv
```

The installer also places the cohort's generated TOMLs, mono query and HLA
dictionary, mappings, crosswalks, secondary PDAC view files, and a
`*_complete_f_inputs_manifest.json` in that same cohort root. It refuses to
overwrite an installed complete-F file. The combined `${INPUT_BUNDLE}` remains
the cross-cohort manifest package used by downstream validation; it is not the
Stage-2 query location.

**Gate 2:** all three cohorts retain the intended endpoint population; exact
duplicates are removed; every declared tuple has a query row and
`mapping_status=scoreable`; the package validator passes; no production
omission-floor rows remain.

## Phase 3 — Recalculate complete Level-1 F tensors

The Level-1 tensor computation driver lives outside this repository. Do not
invent or substitute a local command. Use the authoritative
MHC-competition/Stage-2 workflow under `WORK_DATA_ROOT`, with the frozen
NCI-selected Level-1 geometry, decision regime, and exposure/tau grid.

The external run must consume the generated complete queries/configurations
from `${PDAC_INPUT_ROOT}`, `${COVID_SPIKE_INPUT_ROOT}`, and
`${COVID_NONSPIKE_INPUT_ROOT}` and produce these nine immutable tensor trees:

```text
${TENSOR_ROOT}/pdac/{full,focal,mono}
${TENSOR_ROOT}/covid_spike/{full,focal,mono}
${TENSOR_ROOT}/covid_nonspike/{full,focal,mono}
```

After copying those stable cohort-root files to the same cohort roots on IRIS,
use the Work_Data wrapper. For example:

```bash
cd /data1/lukszam/Marcus/IRIS_scripts
bash complete_f_stage2/submit_complete_f_stage2.sh \
  --dataset PDAC \
  --run-id "${RUN_ID}" \
  --q-model-config /data1/lukszam/Marcus/Jayon/binding_sim_updated_PDAC_Full_Deduplicated.toml \
  --external-validation-input-root /data1/lukszam/Marcus/Jayon \
  --hla-environment-representation full \
  --pn-hla-scope all \
  --dry-run
```

Repeat for COVID spike and non-spike, then for mono with each installed mono
TOML, crosswalk, and manifest-declared environment count. Remove `--dry-run`
only after all representations report the expected query rows and active
environment geometry.

Each tree must contain:

```text
f_tensor.parquet
f_tensor.observations.parquet
f_tensor.metadata.json
```

Before launching, record the exact external executable, repository commit,
container/module environment, input paths, frozen Level-1 parameters, and full
command line in:

```text
${STAGE_ROOT}/f_tensor_runbook.md
```

After completion, hash every tensor contract file:

```bash
find "${TENSOR_ROOT}" -type f \
  \( -name 'f_tensor.parquet' \
     -o -name 'f_tensor.observations.parquet' \
     -o -name 'f_tensor.metadata.json' \) \
  -exec shasum -a 256 {} + | LC_ALL=C sort \
  > "${STAGE_ROOT}/f_tensor_sha256.txt"
```

Verify that observation identities match the complete query roster, that each
expected tau is present exactly once per observation, and that no tuple was
silently dropped. A completed zero signal is valid; a missing observation is
not.

**Gate 3:** all nine tensor trees are complete, hashes are frozen, tuple-level
coverage is exact, and the old NCI-selected Level-1 settings—not newly selected
ones—were used.

## Phase 4 — Freeze and validate the pipeline configuration

Copy the typed example and replace every path/hash placeholder with the new
input and tensor contracts plus the existing frozen NCI summaries:

```bash
cp IRIS_scripts/configs/pipeline/iris_fullroster_pipeline.example.json "${PIPELINE_CONFIG}"
shasum -a 256 "${INPUT_BUNDLE}/manifest.json"
cat "${STAGE_ROOT}/f_tensor_sha256.txt"
```

The completed configuration must point to:

- `${INPUT_BUNDLE}` and its manifest hash;
- all nine new tensor directories and their three file hashes;
- the five frozen per-observation NCI summaries and their hashes;
- `log_epsilon = 1e-12`;
- the declared twelve fixed-L2 operators;
- the frozen tournament label and computational contracts.

Validate it before producing transfers:

```bash
supported_ap_code/target/release/iris_fullroster_pipeline validate-config \
  --config "${PIPELINE_CONFIG}"
```

**Gate 4:** configuration validation passes and every absolute path resolves to
the new complete-F lineage or an explicitly frozen NCI source.

## Phase 5 — Regenerate transfers and fixed-L2 bundles

```bash
PIPELINE=supported_ap_code/target/release/iris_fullroster_pipeline

"${PIPELINE}" transfer-batch \
  --config "${PIPELINE_CONFIG}" \
  --output "${TRANSFER_ROOT}"

"${PIPELINE}" validate-transfers \
  --package "${TRANSFER_ROOT}"

"${PIPELINE}" build-fixed-bundles \
  --config "${PIPELINE_CONFIG}" \
  --transfers "${TRANSFER_ROOT}" \
  --output "${FIXED_BUNDLES}"
```

Required checks:

- Exactly 30 transfer jobs exist: 3 cohorts × 5 component models × 2 metric
  branches.
- Every mapping tuple resolves uniquely to a completed tau score.
- Endpoint and label counts are unchanged.
- The number of omitted computations is zero.
- Completed zero-signal prevalence is reported separately from missingness.
- Reconstructed `max` and `logsumexp` checks pass.
- PR and ROC fixed bundles each contain 60 systems.

Validate the bundle dimensions and content hashes:

```bash
ORGANIZER=supported_ap_code/target/release/directed_round_robin_organizer

"${ORGANIZER}" validate-bundle --bundle "${FIXED_BUNDLES}/pr"
"${ORGANIZER}" validate-bundle --bundle "${FIXED_BUNDLES}/roc"
"${ORGANIZER}" bundle-content-hash --bundle "${FIXED_BUNDLES}/pr"
"${ORGANIZER}" bundle-content-hash --bundle "${FIXED_BUNDLES}/roc"
```

**Gate 5:** transfers validate, fixed bundles validate, and the complete-F
cohort diagnostics—not the omission-floor results—are used to preregister the
alpha grid and any transport threshold.

## Phase 6 — Diagnose clean data and freeze the joint adaptive grid

Before launching selection, produce a diagnostic report from the complete-F
transfers covering at least:

- cohort/component performance of `mean`, `logmeanexp`, `max`, and
  `logsumexp`;
- candidate-count and completed-zero prevalence by cohort and label;
- isolated-leader versus plateau examples;
- component-level reversals that L2 cannot repair;
- proposed alpha values and the rationale for their spacing;
- the full q/c/kappa grid and solver contract;
- a preregistered quantitative leave-one-cohort-out transport gate.

Store the rationale in `${STAGE_ROOT}/adaptive_grid.md` and the literal shell
values in `${STAGE_ROOT}/adaptive_grid.env`:

```bash
ALPHA_VALUES='<PREREGISTERED_SPACE_SEPARATED_ALPHA_ORDERS>'
Q_VALUES='<PREREGISTERED_SPACE_SEPARATED_Q_ORDERS>'
C_MIN='<PREREGISTERED_C_MIN>'
C_MAX='<PREREGISTERED_C_MAX>'
C_STEP='<PREREGISTERED_C_STEP>'
KAPPA_MIN='<PREREGISTERED_KAPPA_MIN>'
KAPPA_MAX='<PREREGISTERED_KAPPA_MAX>'
KAPPA_POINTS='<PREREGISTERED_KAPPA_POINTS>'
SOLVER_TOLERANCE='<PREREGISTERED_SOLVER_TOLERANCE>'
SOLVER_ITERATIONS='<PREREGISTERED_SOLVER_ITERATIONS>'
```

Do not extend or densify the grid after viewing selection ranks.

**Gate 6:** clean-data diagnosis is complete and the full joint grid is frozen
before selection.

## Phase 7 — Select adaptive L2 parameters on IRIS

Upload the validated transfers, fixed bundles, selector source, wrappers, and
frozen grid to a new IRIS staging root. From the local Mac:

```bash
CLUSTER_HOST=replace_with_iris_ssh_host
IRIS_REPO_ROOT=/data1/lukszam/Marcus/Supported_Model_Replacement
IRIS_SELECTION_STAGE=/data1/lukszam/Marcus/Complete_F_Adaptive_Selection

ssh "${CLUSTER_HOST}" \
  "mkdir -p '${IRIS_REPO_ROOT}/IRIS_scripts' '${IRIS_SELECTION_STAGE}'"

rsync -av --exclude 'target/' \
  "${REPO_ROOT}/supported_ap_code/" \
  "${CLUSTER_HOST}:${IRIS_REPO_ROOT}/supported_ap_code/"

rsync -av \
  "${REPO_ROOT}/IRIS_scripts/adaptive_selection/submit_adaptive_hillq_selection_slurm.sh" \
  "${REPO_ROOT}/IRIS_scripts/adaptive_selection/run_adaptive_hillq_plan.slurm" \
  "${REPO_ROOT}/IRIS_scripts/adaptive_selection/run_adaptive_hillq_selection_array.slurm" \
  "${REPO_ROOT}/IRIS_scripts/adaptive_selection/finalize_adaptive_hillq_selection_slurm.sh" \
  "${REPO_ROOT}/IRIS_scripts/adaptive_selection/adaptive_hillq_cluster_helper.py" \
  "${REPO_ROOT}/IRIS_scripts/shared/slurm_round_robin_helper.py" \
  "${CLUSTER_HOST}:${IRIS_REPO_ROOT}/IRIS_scripts/"

rsync -av "${TRANSFER_ROOT}/" \
  "${CLUSTER_HOST}:${IRIS_SELECTION_STAGE}/full_roster_transfers/"
rsync -av "${FIXED_BUNDLES}/" \
  "${CLUSTER_HOST}:${IRIS_SELECTION_STAGE}/full_roster_fixed_l2/"
rsync -av "${STAGE_ROOT}/adaptive_grid.env" \
  "${CLUSTER_HOST}:${IRIS_SELECTION_STAGE}/adaptive_grid.env"
```

On IRIS, build the Linux binaries, source the one frozen grid, and use a new
adaptive run root. First run a measured pilot, inspect its status/resource
report, then run the full immutable plan with exactly the same values:

```bash
cd /data1/lukszam/Marcus/Supported_Model_Replacement

IRIS_SELECTION_STAGE=/data1/lukszam/Marcus/Complete_F_Adaptive_Selection
IRIS_TRANSFER_ROOT=${IRIS_SELECTION_STAGE}/full_roster_transfers
IRIS_FIXED_BUNDLES=${IRIS_SELECTION_STAGE}/full_roster_fixed_l2
IRIS_ADAPTIVE_RUN=${IRIS_SELECTION_STAGE}/adaptive_hillq_selection
source "${IRIS_SELECTION_STAGE}/adaptive_grid.env"

cargo build --release --manifest-path supported_ap_code/Cargo.toml \
  --package select_adaptive_hillq --bins

bash IRIS_scripts/adaptive_selection/submit_adaptive_hillq_selection_slurm.sh \
  --mode pilot \
  --source-root "${IRIS_TRANSFER_ROOT}" \
  --bundle-root "${IRIS_FIXED_BUNDLES}" \
  --run-root "${IRIS_ADAPTIVE_RUN}" \
  --alpha-values "${ALPHA_VALUES}" \
  --q-values "${Q_VALUES}" \
  --c-min "${C_MIN}" --c-max "${C_MAX}" --c-step "${C_STEP}" \
  --kappa-min "${KAPPA_MIN}" --kappa-max "${KAPPA_MAX}" \
  --kappa-points "${KAPPA_POINTS}" \
  --solver-absolute-tolerance "${SOLVER_TOLERANCE}" \
  --solver-max-iterations "${SOLVER_ITERATIONS}"

bash IRIS_scripts/adaptive_selection/submit_adaptive_hillq_selection_slurm.sh \
  --mode full \
  --source-root "${IRIS_TRANSFER_ROOT}" \
  --bundle-root "${IRIS_FIXED_BUNDLES}" \
  --run-root "${IRIS_ADAPTIVE_RUN}" \
  --alpha-values "${ALPHA_VALUES}" \
  --q-values "${Q_VALUES}" \
  --c-min "${C_MIN}" --c-max "${C_MAX}" --c-step "${C_STEP}" \
  --kappa-min "${KAPPA_MIN}" --kappa-max "${KAPPA_MAX}" \
  --kappa-points "${KAPPA_POINTS}" \
  --solver-absolute-tolerance "${SOLVER_TOLERANCE}" \
  --solver-max-iterations "${SOLVER_ITERATIONS}"
```

The q/c/kappa/solver flags above become valid after Phase 1.3 updates the
Slurm wrapper; do not launch with a mixture of script-level values and implicit
binary defaults.

Inspect:

```text
${IRIS_ADAPTIVE_RUN}/full/status/status.json
${IRIS_ADAPTIVE_RUN}/full/status/audit.json
${IRIS_ADAPTIVE_RUN}/full/resource_report/summary.json
${IRIS_ADAPTIVE_RUN}/full/selection/selection_manifest.json
${IRIS_ADAPTIVE_RUN}/full/selection/selected_parameters.csv
${IRIS_ADAPTIVE_RUN}/full/selection/leave_one_cohort_out.csv
```

After Gate 7 passes, copy the compact final selection and audit material back
to the local immutable staging root; do not copy shard results into the
repository:

```bash
mkdir -p "${ADAPTIVE_RUN}/full"
rsync -av "${CLUSTER_HOST}:${IRIS_SELECTION_STAGE}/adaptive_hillq_selection/full/selection/" \
  "${ADAPTIVE_RUN}/full/selection/"
rsync -av "${CLUSTER_HOST}:${IRIS_SELECTION_STAGE}/adaptive_hillq_selection/full/status/" \
  "${ADAPTIVE_RUN}/full/status/"
rsync -av "${CLUSTER_HOST}:${IRIS_SELECTION_STAGE}/adaptive_hillq_selection/full/resource_report/" \
  "${ADAPTIVE_RUN}/full/resource_report/"
```

**Gate 7:** all CNAP shards audit as complete; each component/metric/policy has
one selected `(alpha,q,c,kappa)` tuple; no selected `c` or `kappa` lies on a
grid boundary; the preregistered transport gate passes or the run is reported
as a failed model-development attempt rather than retuned.

## Phase 8 — Build and validate the combined 70-system bundles

```bash
"${PIPELINE}" build-combined-bundles \
  --base-bundles "${FIXED_BUNDLES}" \
  --selection "${ADAPTIVE_RUN}/full/selection" \
  --output "${COMBINED_BUNDLES}"

"${ORGANIZER}" validate-bundle --bundle "${COMBINED_BUNDLES}/pr"
"${ORGANIZER}" validate-bundle --bundle "${COMBINED_BUNDLES}/roc"
"${ORGANIZER}" bundle-content-hash --bundle "${COMBINED_BUNDLES}/pr" \
  > "${STAGE_ROOT}/combined_pr_bundle_hash.txt"
"${ORGANIZER}" bundle-content-hash --bundle "${COMBINED_BUNDLES}/roc" \
  > "${STAGE_ROOT}/combined_roc_bundle_hash.txt"
```

**Gate 8:** each metric bundle contains 70 systems, 3 evaluations, and 7,245
pair-cohort matches; provenance points only to the new complete-F transfers,
selection, and base bundles.

## Phase 9 — Stage and validate the replacement cluster inputs

Do this only after Gates 0--8 pass. Keep the superseded package temporarily,
until the replacement tournament pilot passes. The coexistence is a migration
checkpoint, not a supported compatibility mode.

This plan uses the final cluster package name:

```text
cluster_inputs/full_roster_selfgated_dual_selection_complete_f
```

Copy only the compact combined bundles needed by the tournament:

```bash
cd "${REPO_ROOT}"
git status --short -- artifacts cluster_inputs

mkdir -p cluster_inputs/full_roster_selfgated_dual_selection_complete_f
cp -R "${COMBINED_BUNDLES}/pr" \
  cluster_inputs/full_roster_selfgated_dual_selection_complete_f/pr
cp -R "${COMBINED_BUNDLES}/roc" \
  cluster_inputs/full_roster_selfgated_dual_selection_complete_f/roc
```

Create `IRIS_scripts/configs/tournament/config.cluster.full_roster_complete_f.json` from the old
cluster config and point `prebuilt_bundle_root` at the staged complete-F
package. Update the prospective paths and expected hashes in:

- `cluster_inputs/README.md`
- `IRIS_scripts/configs/tournament/config.cluster.full_roster_complete_f.json`
- `IRIS_scripts/docs/COMPLETE_F_CLUSTER_RUNBOOK.md`
- `IRIS_scripts/README.md`
- `supported_ap_code/crates/README.md`

Validate the staged package and record the new hashes. Do not remove the old
package yet:

```bash
"${ORGANIZER}" validate-bundle \
  --bundle cluster_inputs/full_roster_selfgated_dual_selection_complete_f/pr
"${ORGANIZER}" validate-bundle \
  --bundle cluster_inputs/full_roster_selfgated_dual_selection_complete_f/roc
"${ORGANIZER}" bundle-content-hash \
  --bundle cluster_inputs/full_roster_selfgated_dual_selection_complete_f/pr
"${ORGANIZER}" bundle-content-hash \
  --bundle cluster_inputs/full_roster_selfgated_dual_selection_complete_f/roc

git diff --check
git status --short -- artifacts cluster_inputs IRIS_scripts supported_ap_code
```

**Gate 9:** both staged compact bundles validate and the new
documentation/configuration hashes match the organizer's content hashes. The
old tracked package remains available until Gate 10.

## Phase 10 — Upload and run the new tournament on IRIS

Use a new remote run root. Never reuse plans or results from the omission-floor
tournament because their bundle content hashes and executable contracts differ.

Update the local variables in `IRIS_scripts/docs/COMPLETE_F_CLUSTER_RUNBOOK.md`, then upload the
code, new cluster configuration, and new compact bundles. The expected shape is
still 70 systems × 3 evaluations and 7,245 matches per metric unless Gate 8
documents an intentional contract change.

From the local Mac, after replacing the host name:

```bash
LOCAL_ROOT=/Users/thomm15/Documents/Supported_Model_Replacement
CLUSTER_HOST=replace_with_iris_ssh_host
CLUSTER_ROOT=/data1/lukszam/Marcus/Supported_Model_Replacement

ssh "${CLUSTER_HOST}" \
  "mkdir -p '${CLUSTER_ROOT}/IRIS_scripts' '${CLUSTER_ROOT}/cluster_inputs'"

rsync -av --exclude 'target/' \
  "${LOCAL_ROOT}/supported_ap_code/" \
  "${CLUSTER_HOST}:${CLUSTER_ROOT}/supported_ap_code/"

rsync -av \
  "${LOCAL_ROOT}/IRIS_scripts/" \
  "${CLUSTER_HOST}:${CLUSTER_ROOT}/IRIS_scripts/"

rsync -av \
  "${LOCAL_ROOT}/cluster_inputs/full_roster_selfgated_dual_selection_complete_f/" \
  "${CLUSTER_HOST}:${CLUSTER_ROOT}/cluster_inputs/full_roster_selfgated_dual_selection_complete_f/"
```

On IRIS:

```bash
cd /data1/lukszam/Marcus/Supported_Model_Replacement

cargo build --release \
  --manifest-path supported_ap_code/Cargo.toml \
  --package directed_round_robin_organizer \
  --bin directed_round_robin_organizer

bash IRIS_scripts/tournament/submit_directed_round_robin_slurm.sh \
  --mode pilot \
  --config IRIS_scripts/configs/tournament/config.cluster.full_roster_complete_f.json \
  --run-root /data1/lukszam/Marcus/Supported_Model_Replacement_Runs/full_roster_complete_f \
  --covid-spike-label-set threshold_zero \
  --threads 8 \
  --pilot-matches-per-shard 3 \
  --max-concurrent 16 \
  --mem 32G \
  --time 04:00:00 \
  --prepare-only

bash IRIS_scripts/tournament/submit_directed_round_robin_slurm.sh \
  --mode pilot \
  --config IRIS_scripts/configs/tournament/config.cluster.full_roster_complete_f.json \
  --run-root /data1/lukszam/Marcus/Supported_Model_Replacement_Runs/full_roster_complete_f \
  --covid-spike-label-set threshold_zero \
  --threads 8 \
  --pilot-matches-per-shard 3 \
  --max-concurrent 16 \
  --mem 32G \
  --time 04:00:00
```

Inspect the pilot logs, resource summary, audit, and reduction. If the pilot is
clean, submit the full arrays:

```bash
bash IRIS_scripts/tournament/submit_directed_round_robin_slurm.sh \
  --mode full \
  --config IRIS_scripts/configs/tournament/config.cluster.full_roster_complete_f.json \
  --run-root /data1/lukszam/Marcus/Supported_Model_Replacement_Runs/full_roster_complete_f \
  --covid-spike-label-set threshold_zero \
  --threads 8 \
  --full-matches-per-shard 50 \
  --max-concurrent 200 \
  --mem 32G \
  --time 2-00:00:00
```

The finalizer scripts are:

- `IRIS_scripts/tournament/run_directed_round_robin_array.slurm`
- `IRIS_scripts/tournament/finalize_directed_round_robin_slurm.sh`
- `IRIS_scripts/shared/slurm_round_robin_helper.py`

For a valid 70-system replacement bundle, the updated
`finalize_directed_round_robin_slurm.sh` automatically derives the PDAC-only
and all-context 65-system views from the common verdict table. Verify these
outputs rather than rerunning matches:

```text
${RUN_ROOT}/full/induced_views/pr/pdac_only
${RUN_ROOT}/full/induced_views/pr/all_contexts_equal_weight
${RUN_ROOT}/full/induced_views/roc/pdac_only
${RUN_ROOT}/full/induced_views/roc/all_contexts_equal_weight
```

**Gate 10:** every planned verdict exists exactly once; audit and reduction
pass for both metrics; both induced views have the declared node sets; final
scientific reporting separates the effect of complete Level-1 computation from
the effect of the new adaptive rule and refitted parameters.

## Phase 11 — Retire the superseded lineage and finalize the repository

Only after Gate 10 passes should the tracked omission-floor lineage be removed.
Review the exact targets and current status first. These deletions are
intentional and recoverable from Git history, but not from the working tree.

```bash
cd "${REPO_ROOT}"
git status --short -- artifacts cluster_inputs IRIS_scripts

git rm -r -- artifacts/iris_fullroster_production_20260820
git rm -r -- cluster_inputs/full_roster_selfgated_dual_selection_v1
test -f IRIS_scripts/deprecated/tournament/config.cluster.full_roster_selfgated_dual_selection_v1_deprecated.json

mkdir -p artifacts/iris_fullroster_complete_f
cp -R "${INPUT_BUNDLE}" artifacts/iris_fullroster_complete_f/full_roster_inputs
cp -R "${TRANSFER_ROOT}" artifacts/iris_fullroster_complete_f/full_roster_transfers
cp -R "${FIXED_BUNDLES}" artifacts/iris_fullroster_complete_f/full_roster_fixed_l2
cp -R "${ADAPTIVE_RUN}/full/selection" \
  artifacts/iris_fullroster_complete_f/adaptive_hillq_selection
cp -R "${COMBINED_BUNDLES}" \
  artifacts/iris_fullroster_complete_f/full_roster_70_systems
cp "${PIPELINE_CONFIG}" \
  artifacts/iris_fullroster_complete_f/iris_fullroster_pipeline.production.json
```

There should now be one active production artifact root, one cluster package,
and one cluster config. Audit stale identities and old expected hashes:

```bash
rg -n 'iris_fullroster_production_20260820|full_roster_selfgated_dual_selection_v1|f47d6ec1|05a99064' \
  . --glob '!supported_ap_code/target/**' --glob '!.git/**'

"${ORGANIZER}" validate-bundle \
  --bundle cluster_inputs/full_roster_selfgated_dual_selection_complete_f/pr
"${ORGANIZER}" validate-bundle \
  --bundle cluster_inputs/full_roster_selfgated_dual_selection_complete_f/roc

git diff --check
git status --short
git diff --stat
```

Before committing, verify that no raw sensitive cohort data, tensor trees,
cluster logs, or intermediate selection shards were copied into the repository.
The compact `cluster_inputs` bundles are sufficient for tournament execution;
the larger `artifacts` tree should contain only the intentionally retained,
auditable production lineage.

**Gate 11:** no active file references the old production paths or hashes; the
new compact bundles still validate after publication; the Git diff contains
only the intended code, documentation, replacement artifacts, and deletions.

## Stop conditions

Stop rather than patching around any of the following:

- a declared tuple lacks a tensor observation or tau row;
- a production mapping contains an omission `floor` status;
- exact duplicates change an endpoint score through multiplicity;
- endpoint identities or labels change unexpectedly;
- an NCI Level-1 setting differs from the frozen selection;
- the adaptive plan and finalizer do not carry the identical joint grid;
- selected `c` or `kappa` lies on a frozen grid boundary;
- leave-one-cohort-out transport fails its preregistered threshold;
- a bundle, manifest, executable, or plan hash differs from its recorded value;
- any stage proposes reusing an output directory from the superseded lineage.
