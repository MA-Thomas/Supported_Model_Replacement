# NCI parameter tournaments

## Portable distributed pipeline on IRIS

The new entry point is `submit_nci_cluster_pipeline.sh`. It runs the ten
all-regime tournaments using target arrays plus Rayon, then runs a separate
cross-component finalist tournament for PR and ROC. The historical submission
scripts documented below remain available.

Deployment layout:

```text
/data1/lukszam/Marcus/Supported_Model_Replacement/
  supported_ap_code/                  # complete Cargo workspace and Cargo.lock
  IRIS_scripts/nci_parameter_tournament/
    submit_nci_cluster_pipeline.sh
    cluster_pipeline.py
    run_nci_cluster_stage.slurm
    input_package/                   # generated, about 298 MiB of source data
      config.json
      source_config.json
      manifest.json
      inputs/
```

Copy the complete `supported_ap_code` workspace, including its root `src/`,
`Cargo.toml`, `Cargo.lock`, and member crates. The two tournament crates depend on
the root `supported-ap` library. Build Linux binaries on IRIS; Mac binaries are
not the cluster runtime. Python 3.8 or newer, Bash, and Slurm are required. The
local build was verified with Rust 1.91.0. Building requires the locked Cargo
dependencies to be cached or downloadable; use vendored dependencies for an
offline build. The default combinatorial ROC backend requires no external solver.

The generated `input_package/` covers all 14 configured source files using 12
content-deduplicated payload files, with their configured hashes. **Copy this directory explicitly:
it is ignored by Git and will not arrive through a Git checkout alone.** No
separate `Work_Data` directory, upstream IRIS code, or external dataset is needed
on the cluster. To regenerate the package locally under a new output path:

```bash
supported_ap_code/target/release/iris_nci_parameter_tournament export-inputs \
  --config IRIS_scripts/nci_parameter_tournament/config.nci.all_regimes.v1.json \
  --output /path/to/new_input_package
```

On the IRIS login node:

```bash
cd /data1/lukszam/Marcus/Supported_Model_Replacement
cargo build --release --locked --manifest-path supported_ap_code/Cargo.toml \
  --package directed_round_robin_organizer \
  --package iris_nci_parameter_tournament

bash IRIS_scripts/nci_parameter_tournament/submit_nci_cluster_pipeline.sh \
  --run-root /data1/lukszam/Marcus/Supported_Model_Replacement/cluster_outputs/nci_all4200_cluster_v1 \
  --dry-run
```

Remove `--dry-run` to submit. Dry run prints **all eight stages and their
dependencies**, without preparing data, creating a run directory, or submitting
jobs. Defaults are your `componc_cpu` partition and `lukszam` account, 4 CPUs per
task, 100 target shards per tournament, 16 completion shards per tournament,
100 concurrent tasks per array, 20G memory per task, and a two-day time limit.
Override these through `--threads`, `--target-shards`, `--completion-shards`,
`--max-concurrent`, `--mem`, and `--time`; benchmark on IRIS before committing to
large allocations. `--project-root` overrides the deployment root above, and
`--input-package`, `--pipeline`, and `--organizer` override individual inputs.

The submitted dependency chain is:

```text
prepare (1 job)
  -> targets (10 × target-shards)
  -> merge (10 tasks)
  -> complete (10 × completion-shards)
  -> branch-finalize (10 tasks)
  -> finalist-prepare (1 job)
  -> finalist-run (2 tasks: PR, ROC)
  -> publish (1 job)
```

Every stage depends on successful completion of the previous stage. All jobs are
submitted from the login node; compute jobs do not invoke `sbatch`. Preparation
runs on an allocated compute node. Array sizes are fixed at submission, so
shards with no assigned work publish empty completion receipts. The submitter
checks `MaxArraySize` when available; reduce shard counts if the requested array
exceeds the site's limit. The default target array has 1,000 tasks. A context
requiring general SCC analysis assigns that tournament to target shard zero;
other shards publish empty receipts, preserving correctness without duplicating
full-roster comparisons.

Targets retain every challenger, including excluded candidates. Merging audits
all target obligations before completion tasks fill in numerical reports among
survivors. Final branch outputs require the full operational certificate audit.
All tied `s_op` winners advance to the second stage. Component selection uses the
same metric-specific policy and Version 2 rules; exact ties across components
remain ties. Empty and singleton finalist sets are reported without inventing
comparisons. The result is hierarchical selection among component-specific
winning regimes.

The submitter snapshots both executables, the input package, and the workflow
scripts into the run directory. Workers verify the frozen payload and use
`SLURM_CPUS_PER_TASK` for Rayon. `jobs.json` records every submitted job; if
submission fails, jobs already recorded from that attempt are cancelled.
Duplicate submissions are refused while earlier jobs remain queued or running.
After all previous jobs have stopped, repeat the identical command with
`--resume` to validate/reuse completed artifacts and rerun the dependency chain.
Changed inputs, executables, driver code, or settings require a new run root.
Interrupted snapshot creation without a `run.json` also requires a fresh run root.

Final outputs:

- `version2/MODEL/METRIC/selection.json`: each component's regime selection.
- `finalists/`: immutable finalist bundles, parent bindings, and observation joins.
- `final_version2/METRIC/selection.json`: final tournament selections when there
  are at least two finalists.
- `component_winners.json`: PR/ROC winning regimes, component sets, and a unique
  component only when exactly one component remains.

New input and result manifests use relative references. Move the **whole run
folder**, including its input package and evidence, to preserve auditability.
Original absolute-path manifests remain readable. The packaged source config is
retained as provenance; its old Mac paths are not runtime dependencies.

The existing ROC policy retains its two-second optimization deadline. Hardware
contention can affect how far a time-limited solve progresses, so fixed seeds
alone do not guarantee identical numerical bounds across machines. This update
does not alter the evidence policy, computational design, seed, or solver limits.

Validation commands (small fixtures; no production submissions):

```bash
cargo test --release --locked --manifest-path supported_ap_code/Cargo.toml \
  --package directed_round_robin_organizer \
  --package iris_nci_parameter_tournament
python3 -m unittest discover -s IRIS_scripts/nci_parameter_tournament \
  -p test_cluster_pipeline.py -v
```


This workflow declares one parameter set as one tournament system. It supports
two roster modes and two independent execution modes for each of five component
models and each metric.

The original roster procedure remains available. It:

1. ranks all 4,200 regimes by empirical AP for the CNAP branch or empirical AUROC for the AUROC branch (`regime_idx` breaks exact metric ties deterministically);
2. scans downward, accepting the first regime with a new exact `(d_pos, d_neg, M, N)` tuple, until `candidate_count` regimes have been accepted;
3. constructs an immutable organizer bundle and runs the existing complete directed round robin;
4. uses the organizer's fixed-policy graph reduction to obtain `S0`;
5. applies Appendix A Version 2: lexicographic incoming survival with challengers frozen to `S0`, followed only if needed by metric-specific regret against all of `S0`: worst-target CNAP regret for PR, or empirical AUROC regret at `Gamma_op = 1` for ROC.

Exact final vectors remain tied. The workflow never substitutes an empirical-metric tie-break for Version 2.

The frozen configuration currently sets `candidate_count = 20` and uses the manuscript's `[0.01, 0.30]` CNAP range. Its file hashes bind the three NCI tensors and the five metric tables used to construct the native and reciprocal component models.

## All 4,200 regimes with accelerated execution

`config.nci.all_regimes.v1.json` adds `"roster_mode": "all_regimes"` and sets
`candidate_count` to 4,200. It retains the existing input files, assessment
policy, prevalence region, replication count, support order, and master seed.
The original configuration is unchanged.

All-regimes preparation includes every steepness variant in regime-index
order, without empirical-metric screening or tuple/score deduplication. It
requires exactly 4,200 distinct regimes covering the tensor's full declared
geometry × M/N grid. In this mode `candidate_count` is a completeness check;
it does not truncate the table. PR and ROC use the same parameter-set universe
within each component model, with separate metric-specific system IDs.

Build the two release binaries:

```bash
cargo build --release --manifest-path supported_ap_code/Cargo.toml \
  --package directed_round_robin_organizer \
  --package iris_nci_parameter_tournament
```

For the all-regimes path, first prepare and audit the ten bundles and compact
plans without submitting jobs:

```bash
bash IRIS_scripts/nci_parameter_tournament/submit_nci_parameter_tournaments_slurm.sh \
  --config IRIS_scripts/nci_parameter_tournament/config.nci.all_regimes.v1.json \
  --execution-mode accelerated --batch-size 1 \
  --run-root /path/to/nci_all_regimes_run \
  --prepare-only
```

Run the same command without `--prepare-only` to submit. `--dry-run` prints the
submission command after preparing and auditing the inputs. The accelerated
array has **ten tasks**, one coordinator per component-model/metric pair. Each
coordinator uses its allocated Rayon threads within matches; larger batches
can also evaluate multiple opponents concurrently. It resumes from validated
immutable match artifacts if interrupted before publishing its selection.

Accelerated results are written under `RUN_ROOT/selections/MODEL/METRIC/`,
containing the selection certificate, audited summary, and execution counts.
These plans request complete numerical inputs within S0 for Version 2. The
finalizer independently audits each certificate and invokes
`finalize-version2-accelerated`; it does not create a purported complete graph
reduction from early-stopped results. Final Version 2 outputs use the same
`RUN_ROOT/version2/MODEL/METRIC/selection.json` layout as the original workflow.

Reusing a run root requires the same configuration, execution mode, and (for
accelerated plans) batch size and operational output scope. Use a new run root
when switching from the old shortlist to all regimes. These scripts coordinate
local organizer runs through Slurm; a single tournament is not split across
multiple accelerated coordinators.

## Original shortlist and exhaustive execution

Both defaults remain unchanged: the original N=20 configuration and exhaustive
shard execution. Submit them with:

```bash
bash IRIS_scripts/nci_parameter_tournament/submit_nci_parameter_tournaments_slurm.sh \
  --run-root /path/to/nci_parameter_tournament_run
```

Use `--prepare-only` to build and audit all ten bundles and plans without submitting jobs. Final selections are written to `RUN_ROOT/version2/MODEL/METRIC/selection.json`; the separately retained `s0`, intermediate `t`, and final `s_op` arrays make unresolved ties explicit. Each incoming profile names its binding admissible challenger. When the tertiary rule is invoked, each regret profile names the binding evaluation, comparator, and regret. CNAP profiles also report the binding prevalence; AUROC profiles report the candidate and comparator empirical AUROCs. Selection schema 2 declares `tertiary_rule` explicitly and stores ROC results in `auroc_regret_profiles`, with `operational_auroc_gamma: 1.0` and no operational prevalence. Schema 1 selections retain their historical CNAP interpretation. The optional configuration field `tournament.operational_auroc_gamma` defaults to 1; other values are rejected until case-mix regret optimization is implemented. Existing output artifacts are immutable; applying the revised rule to saved evidence requires a new finalization output directory.

Roster and execution choices can also be combined independently: a shortlist
can use `--execution-mode accelerated`, and a complete roster can use
`--execution-mode exhaustive` when all comparisons and graph reports are
required. `--matches-per-shard` applies only to exhaustive execution;
`--batch-size` applies only to accelerated execution.

The historical `config.nci.v1.json` is preserved byte for byte because completed
N=20 artifacts bind its hash. Its old AUROC/CNAP scope wording describes those
historical results; new finalizations use the metric-specific rule above and
record it explicitly in selection schema 2. The all-regimes configuration
declares the updated scope and `operational_auroc_gamma: 1.0`.
