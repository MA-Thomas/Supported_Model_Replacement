# IRIS Rust workflow

These four crates form one sequence. Each stage validates the manifest from the
previous stage, and every output directory must be new.

```text
cohort sources
  -> external_validation_inputs
  -> iris_fullroster_pipeline (transfers and 60-system bundles)
  -> select_adaptive_hillq
  -> iris_fullroster_pipeline (70-system bundles)
  -> directed_round_robin_organizer
```

Build all four binaries from the repository root:

```bash
cargo build --release --manifest-path supported_ap_code/Cargo.toml \
  --package external_validation_inputs \
  --package iris_fullroster_pipeline \
  --package select_adaptive_hillq \
  --package directed_round_robin_organizer
```

## Run locally: prepare the tournament

### 1. Build the full-roster inputs

Required inputs: the three cohorts' filtered queries, HLA environment
dictionaries, long-peptide mappings, TOML templates, and frozen parameter
tables. Their paths and expected hashes are declared in
`IRIS_scripts/external_validation_inputs.example.json`.

```bash
supported_ap_code/target/release/external_validation_inputs build \
  --config IRIS_scripts/external_validation_inputs.example.json \
  --input-root /Users/thomm15/Work_Data \
  --output /new/path/full_roster_inputs
```

Output: an audited package containing the full candidate rosters, mappings,
crosswalks, and `manifest.json`.

### 2. Score transfers and build the fixed bundles

Required inputs:

- the full-roster package from step 1;
- full, focal, and mono Stage-2 tensors for all three cohorts;
- five per-observation NCI summary files; and
- a production config made from
  `IRIS_scripts/iris_fullroster_pipeline.example.json`, with every absolute
  path and SHA-256 value filled in.

```bash
PIPELINE=supported_ap_code/target/release/iris_fullroster_pipeline

$PIPELINE validate-config --config /path/pipeline.json
$PIPELINE transfer-batch --config /path/pipeline.json \
  --output /new/path/transfers
$PIPELINE validate-transfers --package /new/path/transfers
$PIPELINE build-fixed-bundles --config /path/pipeline.json \
  --transfers /new/path/transfers \
  --output /new/path/fixed_bundles
```

Output: 30 validated transfer jobs and PR/ROC bundles with 60 systems each.

### 3. Select adaptive L2 systems

Required inputs: the transfer package and fixed bundles from step 2.

```bash
supported_ap_code/target/release/select_adaptive_hillq \
  --source-root /new/path/transfers \
  --bundle-root /new/path/fixed_bundles \
  --output /new/path/adaptive_selection
```

This exhaustive search is CPU-intensive but practical on a workstation. It
produces five PDAC-selected and five all-context-selected systems per metric.

### 4. Build the final bundles and plans

Required inputs: the fixed bundles from step 2 and selection manifest from
step 3.

```bash
$PIPELINE build-combined-bundles \
  --base-bundles /new/path/fixed_bundles \
  --selection /new/path/adaptive_selection \
  --output /new/path/final_bundles

ORGANIZER=supported_ap_code/target/release/directed_round_robin_organizer
$ORGANIZER validate-bundle --bundle /new/path/final_bundles/pr
$ORGANIZER validate-bundle --bundle /new/path/final_bundles/roc
$ORGANIZER plan --bundle /new/path/final_bundles/pr \
  --output /new/path/plans/pr --matches-per-shard 50
$ORGANIZER plan --bundle /new/path/final_bundles/roc \
  --output /new/path/plans/roc --matches-per-shard 50
```

Output: validated PR and ROC bundles with 70 systems and 7,245 matches each,
plus deterministic, resumable plans.

## Run on the IRIS cluster: execute the tournament

Planning and smoke tests can run locally. The full `run-shard` workload should
run on IRIS because it evaluates 14,490 matches with the production resampling
and optimization settings.

Upload the repository source, excluding `supported_ap_code/target/`. The
validated production `pr/` and `roc/` bundles are already included under
`cluster_inputs/full_roster_selfgated_dual_selection_v1/`.

Do **not** upload the tensors, NCI summaries, input package, transfers, fixed
bundles, adaptive surfaces, or local plans. The final bundles already contain
the endpoint labels and score vectors needed for every match. Rebuild the
organizer and regenerate the plans on IRIS so they record the Linux executable.

Run the plans' shards on IRIS; then use `status`, `reduce`, and `audit` to
produce and verify one complete reduction per metric. Use `induce-view`
afterward to derive the two 65-system policy views without rerunning matches.

The exact upload command, expected bundle hashes, and
Slurm commands are documented in
[`IRIS_scripts/CLUSTER_RUNBOOK.md`](../../IRIS_scripts/CLUSTER_RUNBOOK.md).

## The main rule

Treat every manifest hash as part of the scientific result. Never edit or
partially reuse a published output directory; create a new one and let the
next stage validate it.
