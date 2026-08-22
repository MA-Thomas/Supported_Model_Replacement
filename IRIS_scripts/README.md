# IRIS tournament score provider

This directory contains the IRIS-side provider for
`directed_round_robin_organizer`. The provider packages existing Rust transfer
predictions into one portable, single-metric tournament bundle. It does not
calculate AP, CNAP, AUROC, support, survival, or any verdict.

The provider validates four classes of inputs:

1. the `evaluation_COVID_SPIKE`, `evaluation_COVID_NONSPIKE`, and
   `evaluation_PDAC` tensor/observation/metadata trees;
2. mappings, crosswalks, manifests, and audits under
   `Run_Scripts_Selected_ParamSets_for_PDAC_COVID_Transfer`;
3. each model's metric-specific NCI summary; and
4. existing Rust `long_peptide_predictions.csv` and `summary.json` transfer
   artifacts for all five models, three evaluations, and the selected metric.

COVID SPIKE uses `(patient_id, mutation, long_peptide)` as its analysis-unit
identity. Mutation-specific response disagreement is retained as accepted,
irreducible measurement error. The provider neither collapses those records
to `(patient_id, long_peptide)` nor chooses a replacement response. COVID
NONSPIKE and PDAC use `(patient_id, long_peptide)`; PDAC has no mutation field
in its scientific endpoint identity.

## Rust full-roster input builder

Production mappings, mono inputs, crosswalks, generated TOMLs, audits, and the
input manifest are built by the workspace crate `external_validation_inputs`.
Its example configuration locks the authoritative source SHA-256 hashes and
expected cohort counts:

```text
cargo run --release \
  --manifest-path supported_ap_code/Cargo.toml \
  --package external_validation_inputs -- \
  build \
  --config IRIS_scripts/external_validation_inputs.example.json \
  --input-root /Users/thomm15/Work_Data \
  --output /path/to/full_roster_inputs_v1
```

The output includes representation-neutral candidate rosters and separate
full- and mono-environment mappings. The builder does not choose a numeric
score for `floor` candidates; transfer scoring applies the frozen
`log(1e-12)` policy.

## Provider commands

Use `config.example.json` only when rebuilding bundles from upstream transfer
outputs. All scientific choices, including the non-count aggregation menu and
evidence policy, must be explicit. Aggregation families are not part of either
the provider or organizer contract.

The configuration also requires one named
`covid_spike_label_specification`. The included example selects
`cd8_IFNg_dmso_adj > 0`. To select the stored higher SPIKE definition, use an
ID such as `higher_threshold_0p53`, set the SPIKE threshold to `0.53`, and keep
the strict `>` operator. This choice affects SPIKE only. COVID NONSPIKE is
fixed at `cd8_TNFa_IFNg_dmso_adj > 0`; the provider rejects a configuration
that applies the SPIKE `0.53` threshold to NONSPIKE.

Tournament labels are reconstructed from the configured authoritative mapping
source. Labels embedded in older transfer prediction files are retained and
audited as provenance but do not override the selected SPIKE definition. Score
vectors remain the existing label-independent transfer scores.

```text
python3 iris_score_provider.py validate --config config.json --metric pr
python3 iris_score_provider.py build --config config.json --metric pr --output /path/to/pr_bundle
python3 iris_score_provider.py build --config config.json --metric roc --output /path/to/roc_bundle
```

`build` creates a new directory and refuses to overwrite an existing path. It
calls the configured organizer binary twice: first to compute the portable
content hash, then to validate the finalized bundle. Absolute source paths are
confined to `source_provenance.json`; every executable bundle path is relative.

Run the provider tests with:

```text
python3 -m unittest discover -s tests -v
```

## Slurm pilot and full runs

See `CLUSTER_RUNBOOK.md` for the complete cluster copy, configuration, build,
pilot, full-run, monitoring, output-layout, and resumption instructions.

For the current 70-system workload, use
`config.cluster.full_roster_selfgated_dual_selection_v1.json`. It selects the
immutable PR and ROC bundles under
`../cluster_inputs/full_roster_selfgated_dual_selection_v1`; no configuration
editing or upstream biological files are required on a compute node.

`submit_directed_round_robin_slurm.sh` prepares immutable PR and ROC bundles,
creates deterministic plans, and submits `run_directed_round_robin_array.slurm`.
The required command-line label-set ID must match the configuration, preventing
an accidental run with a different SPIKE definition.

First build the organizer on the cluster:

```text
cargo build --release \
  --manifest-path /path/to/Supported_Model_Replacement/supported_ap_code/Cargo.toml \
  --package directed_round_robin_organizer \
  --bin directed_round_robin_organizer
```

Run a timing pilot containing three PR matches and three ROC matches. With the
organizer's evaluation-interleaved plan, each metric's pilot shard contains one
SPIKE, one NONSPIKE, and one PDAC match:

```text
bash submit_directed_round_robin_slurm.sh \
  --mode pilot \
  --config /path/to/config.json \
  --run-root /data1/path/iris_round_robin \
  --covid-spike-label-set threshold_zero
```

This three-match default is the shortest evaluation-stratified timing check and
uses the full production replications and optimization policy. Increase
`--pilot-matches-per-shard` to `12` for four matches per evaluation and a more
stable runtime sample. Reducing scientific replications would create a smoke
test, not a valid estimate of production runtime, and is intentionally not
done implicitly by the scheduler wrapper.

Run the full bundle after using the pilot measurements to choose `--threads`,
`--full-matches-per-shard`, memory, wall time, and array concurrency. The
launcher derives the exact match count from the validated bundle; the current
70-system bundle contains 7,245 matches per metric:

```text
bash submit_directed_round_robin_slurm.sh \
  --mode full \
  --config /path/to/config.json \
  --run-root /data1/path/iris_round_robin \
  --covid-spike-label-set threshold_zero \
  --threads 8 \
  --full-matches-per-shard 50 \
  --max-concurrent 16
```

`--threads` sets both Slurm `--cpus-per-task` and the size of the organizer's
single shared Rayon pool. The pool schedules independent matches as well as
parallel computational evaluations within a match. OpenMP, MKL, and OpenBLAS
remain restricted to one thread.

The pilot runs shard 0 for both metrics because their computational costs can
differ. Each array task writes elapsed time, maximum resident memory, and exact
match-artifact bytes under `<run-root>/<mode>/resource_usage/`. A dependent
finalizer aggregates those records under `resource_report/`. In full mode it
also uses the bounded Rayon pool to create or validate the PR and ROC
reductions. Context-revision finalization composes from those compact audited
reductions rather than reparsing the raw match reports.

Two dedicated single-context workflows avoid rerunning unchanged comparisons:

- `submit_pdac_revision_round_robin_slurm.sh` replaces only PDAC with
  `pdac_no_splen_evac_grid4`.
- `submit_covid_spike_revision_round_robin_slurm.sh` replaces only SPIKE with
  the strict `cd8_IFNg_dmso_adj > 0.53` definition.

The corresponding providers create one-evaluation bundles with 60 systems and
1,770 matches per metric. Each finalizer composes its revised verdicts with the
audited threshold-zero reductions. The SPIKE wrapper therefore retains the
original PDAC context; it does not implicitly combine both revisions. See the
cluster runbook for the exact contracts and commands.

Use `--prepare-only` to build and validate the bundles and plans without
submitting jobs, or `--dry-run` to print the array submission command. Slurm
resources default to the `componc_cpu` partition and `lukszam` account seen in
the existing IRIS scripts, but all relevant resource choices are command-line
options. Neither wrapper writes to the configured source-data directories.

## Historical selected adaptive Hill-2 tournament

`adaptive_hill2_selected_parameters_v1.json` freezes one component-specific
parameter pair for each model and metric branch. The packaged bundles under
`prebuilt_bundles/adaptive_hill2_selected_v1/` preserve the original 60 systems
and append five adaptive systems, giving 65 systems and 6,240 matches per
metric across the original PDAC, threshold-zero SPIKE, and NONSPIKE contexts.

This workflow is retained for provenance. Its historical prebuilt bundles are
not required by the current 70-system tournament.
