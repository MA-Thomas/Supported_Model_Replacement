# Complete-F cluster runbook

This is the cluster-facing operating guide for the complete-F replacement
workflow. The ordered scientific gates remain in the repository-root
`COMPLETE_F_FULL_ROSTER_REGENERATION_PLAN.md`; do not substitute the deprecated
70-system filtered-roster workflow.

Run-specific plan IDs, measurements, and decisions belong beside the artifacts
in `artifacts/<run-id>/CLUSTER_EXECUTION_LOG.md` rather than in this reusable
guide.

## Inputs that must exist first

1. An immutable package built with `external_validation_inputs` from the active
   cohort-native generators under `/Users/thomm15/Work_Data`.
2. Six manifest-validated Stage-2 query/TOML pairs: three cohorts times full
   and mono HLA representation.
3. Completed PN and QPI tensors whose manifests report no missing computation.
4. A downstream pipeline transfer manifest with `pdac_full` as the primary
   PDAC view and `pdac_rojas_sethna` as a non-selecting secondary view.

## Standard paths

On the local Mac:

```bash
LOCAL_ROOT=/Users/thomm15/Documents/Supported_Model_Replacement
ARTIFACT_ID=iris_fullroster_complete_f_20260904
CLUSTER_HOST=thomm15@islogin01
CLUSTER_ROOT=/data1/lukszam/Marcus/Supported_Model_Replacement
```

On the cluster:

```bash
cd /data1/lukszam/Marcus/Supported_Model_Replacement
CLUSTER_ROOT=/data1/lukszam/Marcus/Supported_Model_Replacement
ARTIFACT_ID=iris_fullroster_complete_f_20260904
RUN_ROOT=/data1/lukszam/Marcus/Supported_Model_Replacement_Runs/adaptive_hillq_complete_f_20260904
```

Never place a run under the source repository, and never reuse one of the local
`grid_diagnostics_v3/local_cost_plan_*` plans on Linux. A plan is bound to the
executable that created it.

## Check and upload files

Use checksum dry runs when files were first uploaded through a browser. No
output from `grep '^<f'` means that all regular files in that command match.

From the local Mac:

```bash
rsync -avcni --itemize-changes --exclude='.DS_Store' \
  "${LOCAL_ROOT}/artifacts/${ARTIFACT_ID}/iris_fullroster_pipeline.complete_f.json" \
  "${LOCAL_ROOT}/artifacts/${ARTIFACT_ID}/full_roster_inputs" \
  "${LOCAL_ROOT}/artifacts/${ARTIFACT_ID}/full_roster_transfers" \
  "${LOCAL_ROOT}/artifacts/${ARTIFACT_ID}/full_roster_fixed_l2" \
  "${CLUSTER_HOST}:${CLUSTER_ROOT}/artifacts/${ARTIFACT_ID}/" \
  | grep '^<f'

rsync -avcni --itemize-changes \
  --exclude='target/' --exclude='.DS_Store' \
  "${LOCAL_ROOT}/supported_ap_code/" \
  "${CLUSTER_HOST}:${CLUSTER_ROOT}/supported_ap_code/" \
  | grep '^<f'
```

Rerun the corresponding command without `-n` to transfer only missing or
changed content. `-c` prevents a file with matching content from being copied
again.

```bash
rsync -avci --partial --progress --exclude='.DS_Store' \
  "${LOCAL_ROOT}/artifacts/${ARTIFACT_ID}/iris_fullroster_pipeline.complete_f.json" \
  "${LOCAL_ROOT}/artifacts/${ARTIFACT_ID}/full_roster_inputs" \
  "${LOCAL_ROOT}/artifacts/${ARTIFACT_ID}/full_roster_transfers" \
  "${LOCAL_ROOT}/artifacts/${ARTIFACT_ID}/full_roster_fixed_l2" \
  "${CLUSTER_HOST}:${CLUSTER_ROOT}/artifacts/${ARTIFACT_ID}/"

rsync -avci --partial --progress \
  --exclude='target/' --exclude='.DS_Store' \
  "${LOCAL_ROOT}/supported_ap_code/" \
  "${CLUSTER_HOST}:${CLUSTER_ROOT}/supported_ap_code/"
```

The active configs and Slurm workflow must also be present. In particular,
`IRIS_scripts/configs/pipeline/iris_fullroster_pipeline.example.json` is a test
fixture for `iris_fullroster_pipeline`.

```bash
ssh "${CLUSTER_HOST}" \
  "mkdir -p '${CLUSTER_ROOT}/IRIS_scripts/configs' '${CLUSTER_ROOT}/IRIS_scripts/adaptive_selection' '${CLUSTER_ROOT}/IRIS_scripts/shared'"

rsync -avci --partial --progress --exclude='.DS_Store' --exclude='__pycache__/' \
  "${LOCAL_ROOT}/IRIS_scripts/configs/" \
  "${CLUSTER_HOST}:${CLUSTER_ROOT}/IRIS_scripts/configs/"

rsync -avci --partial --progress --exclude='.DS_Store' --exclude='__pycache__/' \
  "${LOCAL_ROOT}/IRIS_scripts/adaptive_selection/" \
  "${CLUSTER_HOST}:${CLUSTER_ROOT}/IRIS_scripts/adaptive_selection/"

rsync -avci --partial --progress --exclude='.DS_Store' --exclude='__pycache__/' \
  "${LOCAL_ROOT}/IRIS_scripts/shared/" \
  "${CLUSTER_HOST}:${CLUSTER_ROOT}/IRIS_scripts/shared/"
```

These commands deliberately do not use `--delete`. Never apply an unreviewed
recursive delete to the repository, artifact root, or run-output root.

## Build and test Linux binaries

On the cluster:

```bash
cd "${CLUSTER_ROOT}"

cargo build --release \
  --manifest-path supported_ap_code/Cargo.toml \
  --package select_adaptive_hillq --bins \
  --package directed_round_robin_organizer

cargo test --release \
  --manifest-path supported_ap_code/Cargo.toml \
  --package external_validation_inputs \
  --package iris_fullroster_pipeline \
  --package directed_round_robin_organizer \
  --package select_adaptive_hillq
```

A `No such file or directory` failure in either of the
`iris_fullroster_pipeline` contract tests normally means the pipeline example
fixture was not copied; it is not a numerical or compilation failure.

## Adaptive-selection preflight

```bash
cd "${CLUSTER_ROOT}"

test -x supported_ap_code/target/release/select_adaptive_hillq
test -x supported_ap_code/target/release/select_adaptive_hillq_cluster
test -f "artifacts/${ARTIFACT_ID}/full_roster_transfers/manifest.json"
test -f "artifacts/${ARTIFACT_ID}/full_roster_fixed_l2/pr/bundle_manifest.json"
test -f "artifacts/${ARTIFACT_ID}/full_roster_fixed_l2/roc/bundle_manifest.json"

echo "Adaptive-selection preflight passed"
```

The compact broad-budget grid selected after the complete-F label-blind review
is:

```text
alpha:  0.5 1 2 4 inf
q:      0.5 1 2 inf
c:      -11.0 through 3.2 by 0.2
kappa:  0.02 through 4.0, 6 log-spaced points
```

Every grid axis is explicit. The launcher persists it in `plan.env` and
`run.env`, and rejects a reused plan whose grid differs.

## Adaptive-selection pilot

Launch from the cluster login node. The wrapper submits planning to a compute
node, waits for the immutable Linux plan, then submits the pilot array and its
dependent finalizer.

```bash
bash IRIS_scripts/adaptive_selection/submit_adaptive_hillq_selection_slurm.sh \
  --mode pilot \
  --source-root "${CLUSTER_ROOT}/artifacts/${ARTIFACT_ID}/full_roster_transfers" \
  --bundle-root "${CLUSTER_ROOT}/artifacts/${ARTIFACT_ID}/full_roster_fixed_l2" \
  --run-root "${RUN_ROOT}" \
  --alpha-values "0.5 1 2 4 inf" \
  --q-values "0.5 1 2 inf" \
  --c-min -11.0 --c-max 3.2 --c-step 0.2 \
  --kappa-min 0.02 --kappa-max 4.0 --kappa-points 6 \
  --threads 12 \
  --matches-per-shard 250 \
  --pilot-shards 2 \
  --mem 20G \
  --time 08:00:00
```

Inspect the resulting plan and pilot:

```bash
python3 IRIS_scripts/adaptive_selection/adaptive_hillq_cluster_helper.py \
  plan-info --plan "${RUN_ROOT}/plan"

cat "${RUN_ROOT}/pilot/status/status.json"
cat "${RUN_ROOT}/pilot/resource_report/summary.json"
```

The initial two shards are the light `full_hla/PDAC` block. Benchmark a later
mixed-COVID shard before reducing production memory and wall time:

```bash
sbatch \
  --partition componc_cpu \
  --account lukszam \
  --nodes 1 \
  --ntasks 1 \
  --job-name iris_adaptive_heavy_pilot \
  --array 0-0 \
  --cpus-per-task 12 \
  --mem 2G \
  --time 00:30:00 \
  --output "${RUN_ROOT}/pilot/logs/heavy_%A_%a.out" \
  --error "${RUN_ROOT}/pilot/logs/heavy_%A_%a.err" \
  --export "ALL,IRIS_ADAPTIVE_RUN_ENV=${RUN_ROOT}/pilot/run.env,IRIS_ADAPTIVE_SHARD_OFFSET=384" \
  IRIS_scripts/adaptive_selection/run_adaptive_hillq_selection_array.slurm

cat "${RUN_ROOT}/pilot/resource_usage/shard_384.json"
```

Shard 384 crosses `full_q_mono_pn` SPIKE and NONSPIKE contexts, so it is a
better stress check for input size and memory than shards 0 and 1.

## Adaptive-selection full run

For a target of at most approximately 500 allocated CPUs, use 41 simultaneous
tasks at 12 CPUs per task:

```text
41 tasks * 12 CPUs/task = 492 CPUs
```

The wall-time limit is per array task, not for the entire 414-shard array.

```bash
bash IRIS_scripts/adaptive_selection/submit_adaptive_hillq_selection_slurm.sh \
  --mode full \
  --source-root "${CLUSTER_ROOT}/artifacts/${ARTIFACT_ID}/full_roster_transfers" \
  --bundle-root "${CLUSTER_ROOT}/artifacts/${ARTIFACT_ID}/full_roster_fixed_l2" \
  --run-root "${RUN_ROOT}" \
  --alpha-values "0.5 1 2 4 inf" \
  --q-values "0.5 1 2 inf" \
  --c-min -11.0 --c-max 3.2 --c-step 0.2 \
  --kappa-min 0.02 --kappa-max 4.0 --kappa-points 6 \
  --threads 12 \
  --matches-per-shard 250 \
  --max-concurrent 41 \
  --mem 2G \
  --time 00:30:00
```

The full results directory is separate from the pilot results, so the three
pilot shards are intentionally recomputed. The finalizer starts only after all
full array tasks succeed. It has a separate 12-CPU, 32-GiB, 8-hour allocation.

## Monitor, audit, and resume adaptive selection

```bash
squeue -u thomm15

supported_ap_code/target/release/select_adaptive_hillq_cluster status \
  --plan "${RUN_ROOT}/plan" \
  --results "${RUN_ROOT}/full/results"
```

After successful finalization:

```bash
cat "${RUN_ROOT}/full/status/status.json"
cat "${RUN_ROOT}/full/status/audit.json"
cat "${RUN_ROOT}/full/resource_report/summary.json"
test -f "${RUN_ROOT}/full/selection/selection_manifest.json"
```

If an array task fails and the dependent finalizer does not run, inspect the
corresponding log and resource record. Rerunning the identical full launcher is
restartable: completed shard files are verified and reused, while missing
shards are computed. Do not alter the executable, inputs, grid, replication
count, or shard size underneath an existing plan.

## Tournament pilot

Proceed only after adaptive selection has produced replacement PR and ROC
bundles and their hashes have been audited. Create
`IRIS_scripts/configs/tournament/config.cluster.full_roster_complete_f.json`
with those frozen bundle paths.

```bash
cd "${CLUSTER_ROOT}"
bash IRIS_scripts/tournament/submit_directed_round_robin_slurm.sh \
  --mode pilot \
  --config IRIS_scripts/configs/tournament/config.cluster.full_roster_complete_f.json \
  --run-root /data1/lukszam/Marcus/Supported_Model_Replacement_Runs/full_roster_complete_f \
  --covid-spike-label-set threshold_zero \
  --pilot-matches-per-shard 3 \
  --prepare-only
```

Audit the prepared bundles, plan dimensions, hashes, and the three-evaluation
pilot before removing `--prepare-only`. Use its resource report to set threads,
memory, wall time, shard size, and array concurrency for the full tournament.

## Full tournament

```bash
cd "${CLUSTER_ROOT}"
bash IRIS_scripts/tournament/submit_directed_round_robin_slurm.sh \
  --mode full \
  --config IRIS_scripts/configs/tournament/config.cluster.full_roster_complete_f.json \
  --run-root /data1/lukszam/Marcus/Supported_Model_Replacement_Runs/full_roster_complete_f \
  --covid-spike-label-set threshold_zero \
  --threads REPLACE_FROM_PILOT \
  --full-matches-per-shard REPLACE_FROM_PILOT \
  --max-concurrent REPLACE_FROM_PILOT \
  --mem REPLACE_FROM_PILOT \
  --time REPLACE_FROM_PILOT
```

Completion requires both PR and ROC reductions, complete shard status, and
successful organizer audits. Secondary Rojas/Sethna metrics are reported by
the downstream pipeline and adaptive-selection outputs; they never add a
fourth selection cohort or a second PDAC tensor.
