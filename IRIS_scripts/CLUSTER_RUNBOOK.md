# IRIS 70-system round-robin runbook

This workflow evaluates 70 systems for PR/CNAP and AUROC across PDAC, COVID
SPIKE, and COVID NONSPIKE: 7,245 matches per metric and 14,490 total.

Use these cluster locations:

```text
Code and immutable inputs:
/data1/lukszam/Marcus/Supported_Model_Replacement

Generated plans, match artifacts, logs, and reductions:
/data1/lukszam/Marcus/Supported_Model_Replacement_Runs/full_roster_selfgated_dual_selection_v1
```

## 1. Upload the cluster package

Run this on the local Mac after replacing `CLUSTER_HOST`:

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
  "${LOCAL_ROOT}/IRIS_scripts/config.cluster.full_roster_selfgated_dual_selection_v1.json" \
  "${LOCAL_ROOT}/IRIS_scripts/iris_score_provider.py" \
  "${LOCAL_ROOT}/IRIS_scripts/slurm_round_robin_helper.py" \
  "${LOCAL_ROOT}/IRIS_scripts/submit_directed_round_robin_slurm.sh" \
  "${LOCAL_ROOT}/IRIS_scripts/run_directed_round_robin_array.slurm" \
  "${LOCAL_ROOT}/IRIS_scripts/finalize_directed_round_robin_slurm.sh" \
  "${CLUSTER_HOST}:${CLUSTER_ROOT}/IRIS_scripts/"

rsync -av \
  "${LOCAL_ROOT}/cluster_inputs/full_roster_selfgated_dual_selection_v1/" \
  "${CLUSTER_HOST}:${CLUSTER_ROOT}/cluster_inputs/full_roster_selfgated_dual_selection_v1/"
```

No tensors, NCI summaries, transfer outputs, selection surfaces, local plans,
or macOS build products are needed. The packaged bundles already contain all
endpoint labels and aligned score vectors used by every match.

## 2. Build and validate on IRIS

Log in to IRIS and run:

```bash
cd /data1/lukszam/Marcus/Supported_Model_Replacement

cargo build --release \
  --manifest-path supported_ap_code/Cargo.toml \
  --package directed_round_robin_organizer \
  --bin directed_round_robin_organizer

ORGANIZER=supported_ap_code/target/release/directed_round_robin_organizer
BUNDLES=cluster_inputs/full_roster_selfgated_dual_selection_v1

"${ORGANIZER}" validate-bundle --bundle "${BUNDLES}/pr"
"${ORGANIZER}" validate-bundle --bundle "${BUNDLES}/roc"
```

Validation must report 70 systems, three evaluations, and 7,245 matches for
each metric. Expected bundle hashes:

```text
PR   f47d6ec1c15cfe7eacc9a98d3e01f345c26ca7b40d55dd9d3e043961a22a873a
ROC  05a9906435467a1d80d195cf9acabc8ed45c8ae95025a8a060cb2a76a97b5433
```

Stop if either validation or hash differs.

## 3. Prepare without submitting

This validates and installs both bundles and creates cluster-native pilot
plans without calling `sbatch`. The `-x` flag prints each preparation stage so
the command does not appear silently stalled:

```bash
cd /data1/lukszam/Marcus/Supported_Model_Replacement/IRIS_scripts && bash -x ./submit_directed_round_robin_slurm.sh --mode pilot --config config.cluster.full_roster_selfgated_dual_selection_v1.json --run-root /data1/lukszam/Marcus/Supported_Model_Replacement_Runs/full_roster_selfgated_dual_selection_v1 --covid-spike-label-set threshold_zero --threads 8 --pilot-matches-per-shard 3 --max-concurrent 16 --mem 32G --time 04:00:00 --prepare-only
```

## 4. Run the timing pilot

Repeat the command from step 3 with only `--prepare-only` removed. The pilot
runs six production-policy matches: one match from each evaluation for each
metric. It does not reduce scientific replication or optimization settings.

The launcher prints the array job ID and its dependent finalizer job ID.
Monitor them with:

```bash
squeue -u "$USER"
sacct -j ARRAY_JOB_ID --format=JobID,State,Elapsed,MaxRSS,AllocCPUS
```

Review the pilot resource report before choosing full-run threads, shard size,
memory, wall time, and concurrency:

```text
/data1/lukszam/Marcus/Supported_Model_Replacement_Runs/full_roster_selfgated_dual_selection_v1/pilot/resource_report/
```

For a more stable pilot, use `--pilot-matches-per-shard 12`, which runs four
matches from each evaluation per metric. Use a new run root when changing the
pilot shard size.

## 5. Run all 14,490 matches

The completed six-match pilot used at most 175 MiB per task. A 4 GiB request
therefore leaves ample headroom when a full shard has up to twelve active
matches, while avoiding the pilot's unnecessary 32 GiB reservation. A
60-task concurrency cap permits at most 720 CPUs and 240 GiB across the
running array. Slurm may run fewer tasks when resources or fair-share limits
require it.

Keep the 48-hour wall-time request. The eight-thread PR pilot took about 52
minutes for three matches; simple linear scaling at eight threads gives roughly
14.6 hours for an average 50-match PR shard. Twelve threads may shorten that
time, while the larger limit protects against imperfect scaling and variation
among comparisons.

The following command uses 50 matches per shard, producing 145 PR shards and
145 ROC shards:

```bash
cd /data1/lukszam/Marcus/Supported_Model_Replacement/IRIS_scripts && bash -x ./submit_directed_round_robin_slurm.sh --mode full --config config.cluster.full_roster_selfgated_dual_selection_v1.json --run-root /data1/lukszam/Marcus/Supported_Model_Replacement_Runs/full_roster_selfgated_dual_selection_v1 --covid-spike-label-set threshold_zero --threads 12 --full-matches-per-shard 50 --max-concurrent 60 --mem 4G --time 2-00:00:00
```

Each task receives twelve CPUs and runs at most twelve matches concurrently.
After every shard succeeds, the finalizer checks completeness, audits and
reduces both metrics, and creates the PDAC-only and all-context 65-system
induced views.

## 6. Resume and inspect results

A run is resumable. Repeat the exact full command with the same run root,
configuration, threads, and sharding settings. Valid completed artifacts are
reused; only missing matches are computed.

Do not edit a bundle, plan, `run.env`, or completed match artifact. If any
scientific or execution setting changes, use a new run root.

Full-run outputs are under:

```text
.../full/plans/
.../full/results/
.../full/logs/
.../full/resource_report/
.../full/reductions/pr/
.../full/reductions/roc/
.../full/induced_views/
```

Keep generated runs outside the copied source directory. Always build the
Linux organizer on IRIS; do not upload or run the local macOS executable.
