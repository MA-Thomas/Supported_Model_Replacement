# IRIS directed round-robin cluster runbook

This runbook uses the following cluster locations:

```text
Code:
/data1/lukszam/Marcus/Supported_Model_Replacement

Generated bundles, plans, match artifacts, logs, and reductions:
/data1/lukszam/Marcus/Supported_Model_Replacement_Runs
```

Keep generated runs outside the source directory so that later source-code
copies or updates cannot overwrite tournament artifacts.

## 1. Copy the repository to the cluster

Copy `Supported_Model_Replacement/` to:

```text
/data1/lukszam/Marcus/Supported_Model_Replacement
```

The local `supported_ap_code/target/` directory may be omitted. It contains
macOS build products, can be large, and cannot be executed on the Linux
cluster. The organizer must be rebuilt on the cluster.

For example, run `rsync` from the computer that contains the local directory,
replacing `CLUSTER_HOST` with the SSH host name:

```bash
rsync -av --exclude 'supported_ap_code/target/' \
  /Users/thomm15/Documents/Supported_Model_Replacement/ \
  CLUSTER_HOST:/data1/lukszam/Marcus/Supported_Model_Replacement/
```

## 2. Build the organizer on the cluster

Log in to the cluster and run:

```bash
cd /data1/lukszam/Marcus/Supported_Model_Replacement

cargo build --release \
  --manifest-path supported_ap_code/Cargo.toml \
  --package directed_round_robin_organizer \
  --bin directed_round_robin_organizer
```

The resulting executable is:

```text
/data1/lukszam/Marcus/Supported_Model_Replacement/supported_ap_code/target/release/directed_round_robin_organizer
```

The Slurm launcher finds this executable automatically from its own repository
location unless `--organizer` is supplied explicitly.

## 3. Create a cluster-specific provider configuration

Copy the example rather than editing it in place:

```bash
cd /data1/lukszam/Marcus/Supported_Model_Replacement/IRIS_scripts
cp config.example.json config.cluster.threshold_zero.json
```

Replace every local `/Users/thomm15/Work_Data/...` path with the corresponding
cluster path. Check these configuration entries carefully:

- `transfer_root`;
- `run_inputs_root`;
- every `models.*.nci_summary`;
- every `evaluations.*.evaluation_dir`; and
- every `evaluations.*.mapping_source`.

Every configured input must be readable from a Slurm compute node, not only
from the login node. The launcher writes an effective, absolute-path copy of
the configuration into the run root. It overrides `organizer_binary` with the
release executable selected by the launcher.

### COVID label choice

The wrapper requires an explicit SPIKE label-set ID. For the historical
threshold-zero definition, the configuration must contain:

```json
"covid_spike_label_specification": {
  "id": "threshold_zero",
  "description": "COVID SPIKE response is positive when cd8_IFNg_dmso_adj is strictly greater than 0.0.",
  "threshold": 0.0,
  "comparison_operator": ">",
  "response_field": "cd8_IFNg_dmso_adj"
}
```

The SPIKE evaluation entry must use the same response field, threshold, and
operator. If the stored higher SPIKE definition is selected instead, use an
explicit ID such as `higher_threshold_0p53` and set the SPIKE threshold to
`0.53` with the strict `>` operator.

The higher threshold applies to SPIKE only. COVID NONSPIKE remains fixed at:

```text
cd8_TNFa_IFNg_dmso_adj > 0
```

The provider rejects a configuration that applies the SPIKE `0.53` threshold
to NONSPIKE.

## 4. Validate preparation without submitting jobs

The following command builds and validates the immutable PR and ROC bundles
and creates the evaluation-stratified pilot plans, but does not call `sbatch`:

```bash
cd /data1/lukszam/Marcus/Supported_Model_Replacement/IRIS_scripts

bash submit_directed_round_robin_slurm.sh \
  --mode pilot \
  --config config.cluster.threshold_zero.json \
  --run-root /data1/lukszam/Marcus/Supported_Model_Replacement_Runs/threshold_zero \
  --covid-spike-label-set threshold_zero \
  --prepare-only
```

To print the intended `sbatch` command without submitting it, replace
`--prepare-only` with `--dry-run`.

## 5. Submit the reduced timing pilot

```bash
cd /data1/lukszam/Marcus/Supported_Model_Replacement/IRIS_scripts

bash submit_directed_round_robin_slurm.sh \
  --mode pilot \
  --config config.cluster.threshold_zero.json \
  --run-root /data1/lukszam/Marcus/Supported_Model_Replacement_Runs/threshold_zero \
  --covid-spike-label-set threshold_zero
```

The default pilot runs three PR matches and three ROC matches. Each metric's
pilot shard contains one deterministic match from each evaluation: SPIKE,
NONSPIKE, and PDAC. The matches use the complete production scientific policy;
the launcher does not reduce replications or optimization budgets.

The launcher prints two job IDs:

1. the PR/ROC Slurm array job; and
2. an `afterok` finalizer job that aggregates resource measurements.

Monitor them with standard Slurm commands, for example:

```bash
squeue -u "$USER"
sacct -j ARRAY_JOB_ID --format=JobID,State,Elapsed,MaxRSS,AllocCPUS
```

Pilot logs and resource reports are written below:

```text
/data1/lukszam/Marcus/Supported_Model_Replacement_Runs/threshold_zero/pilot/logs/
/data1/lukszam/Marcus/Supported_Model_Replacement_Runs/threshold_zero/pilot/resource_usage/
/data1/lukszam/Marcus/Supported_Model_Replacement_Runs/threshold_zero/pilot/resource_report/
```

The resource report records elapsed time, maximum resident memory, and exact
match-artifact bytes. Review it before choosing the full-run thread count,
matches per shard, memory, wall time, and maximum concurrent array tasks.

For a more stable timing sample, add:

```text
--pilot-matches-per-shard 12
```

That runs four matches from each evaluation per metric.

## 6. Submit the complete PR and ROC workloads

The same run root can be used for full mode. Pilot and full plans and results
are kept in separate subdirectories, while the immutable bundles are reused.

For example:

```bash
cd /data1/lukszam/Marcus/Supported_Model_Replacement/IRIS_scripts

bash submit_directed_round_robin_slurm.sh \
  --mode full \
  --config config.cluster.threshold_zero.json \
  --run-root /data1/lukszam/Marcus/Supported_Model_Replacement_Runs/threshold_zero \
  --covid-spike-label-set threshold_zero \
  --threads 8 \
  --full-matches-per-shard 50 \
  --max-concurrent 16 \
  --mem 32G \
  --time 2-00:00:00
```

Full mode requires exactly 5,310 PR matches and 5,310 ROC matches. With 50
matches per shard, each metric has 107 shards, for 214 array tasks in total.
At most 16 array tasks run concurrently in the example above. Each task gets
eight CPUs and runs at most eight matches concurrently.

After every array task succeeds, the dependent finalizer:

1. aggregates resource measurements;
2. checks result status;
3. audits completeness;
4. performs the deterministic reduction for PR and ROC; and
5. audits both reductions.

The reductions are written below:

```text
/data1/lukszam/Marcus/Supported_Model_Replacement_Runs/threshold_zero/full/reductions/pr/
/data1/lukszam/Marcus/Supported_Model_Replacement_Runs/threshold_zero/full/reductions/roc/
```

## 7. Resume an interrupted run

Match artifacts are resumable. Re-run the same launcher command with the same
mode, configuration, run root, label-set ID, sharding settings, and thread
count. Existing artifacts are validated and reused; only missing matches are
computed.

Do not delete or edit `run.env`, a plan, a bundle, or completed match artifacts
to force a retry. If scientific settings, label choice, or sharding settings
change, use a new run root so that the two executions remain distinct and
auditable.

## 8. Important operational boundaries

- Do not run the local macOS organizer executable on the cluster; always build
  the Linux release executable there.
- Do not put generated runs inside the copied source directory.
- Set Slurm `--cpus-per-task` through `--threads`; the launcher keeps these
  values equal.
- The organizer owns match-level parallelism. Nested Rayon, OpenMP, MKL, and
  OpenBLAS pools are set to one thread by the array worker.
- The wrapper does not alter any configured source-data directory.
- Use a distinct run root for each SPIKE label specification.
