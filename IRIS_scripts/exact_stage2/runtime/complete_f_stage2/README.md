# Exact survivor-derived Stage 2

This launcher computes the merged NCI survivor union on the complete PDAC,
COVID Spike, and COVID Nonspike query rosters. PR and ROC remain separate
selection branches but share Stage 2 computations.

| Construction | Consumers | Geometries | Exact geometry/MN regimes | Tau-expanded cells per observation |
|---|---|---:|---:|---:|
| full | Full HLA; mono-Q/full-PN | 20 | 28 | 168 |
| focal | Focal HLA | 20 | 27 | 162 |
| mono | Old monoallelic; full-Q/mono-PN | 40 | 89 | 534 |

Every P/N task receives one geometry, all six tau values, and only the M/N
pairs required for that geometry. The union M/N CSV describes the tensor
index axis; it is never passed to Runner for an exact P/N task. The Rust
Runner and the P/N/Q/Pi computation crates are unchanged.

## Deployment

On IRIS, place the updated files at `/home/thomm15/New_Approaches_Scripts/`:

- `submit_stage2_new.sh`, `run_stage_2_new.sh`, `stage2_run_contract.py`,
  `survivor_grid.py`, `finalize_stage2_pn_shards.sh`, and
  `validate_complete_f_stage2_inputs.py`;
- the complete `complete_f_stage2/` and `complete_f_input_package/` directories;
- updated `evaluation_code_PDAC/` and `evaluation_code_COVID/` source trees.

The assembly scripts build their code from these launcher-side source trees.
No changes to `/data1/lukszam/Marcus/New_Approaches/Runner` or its scientific
calculation crates are needed. The shared launcher still builds the unchanged
Runner before fingerprinting it. Python 3.10+, Bash, Cargo, and the existing
cluster Runner dependencies are required.

The input package directly supplies the generated TOMLs, queries, mappings,
and environment dictionaries. Its files must stay at the same paths during a
run. No separate cohort-input installation is needed for this launcher.

## Preflight and submit

From `/home/thomm15/New_Approaches_Scripts`:

```bash
bash complete_f_stage2/submit_complete_f_stage2.sh \
  --dataset all --construction all \
  --run-id-prefix s0_exact_20260916_v1 --dry-run
```

After reviewing the resolved resources, remove `--dry-run` to submit. This
creates the three run IDs `s0_exact_20260916_v1_full`,
`s0_exact_20260916_v1_focal`, and `s0_exact_20260916_v1_mono` under each cohort's
existing output root. It runs Q/Pi once per construction and P/N only on the
exact survivor grid. It does not submit NCI or E-vac computation.

Use `--dataset PDAC|COVID_SPIKE|COVID_NONSPIKE` and
`--construction full|focal|mono` to limit the launch. `--mode qpi|pn` selects a
single phase. `--missing-only` resumes a matching run using validated task
receipts; it never reuses results from the superseded fixed-grid runs. All
selected configurations are preflighted before any job is submitted.

Defaults: P/N workers request 16 CPUs and 60G; Q/Pi workers request eight CPUs
and 30G. The P/N shard target is 4,000 observations, and array concurrency is
11 **per submitted array**. For one full launch with the default CPU settings,
the nine P/N arrays use at most 9 × 11 × 16 = 1,584 CPUs and the nine Q/Pi
arrays use at most 9 × 11 × 8 = 792 CPUs: 2,376 CPUs combined, below 2,500.
Each merge array waits for its own P/N computation array to succeed and uses
fewer CPUs, so merges do not increase this upper bound. This bound excludes
other launches/account jobs and must be recalculated if CPU requests or
array concurrency are overridden. Expanded M/N cases still need a cluster resource
check. `--target-env-ids 0` can run a small environment subset first,
then `--missing-only` without that option fills the remaining environments.
Choose an active environment printed by preflight for the selected run.
Available controls include `--pn-peptides-per-shard`, `--array-concurrency`,
`--pn-cpus-per-task`, `--qpi-cpus-per-task`, `--pn-mem`, `--qpi-mem`, and
`--partition`. `--cpus-per-task` and `--mem` override their respective settings
for both computation modes. Wall-time limits remain 72 hours for P/N and
50 minutes for Q/Pi. Shard merges request one CPU, 16G, and two hours.

Geometry work is shared across tau values and M/N thresholds. Fewer requested
cells are not a claim of proportional wall-clock savings.

## Assembly after all Stage 2 tasks and shard merges succeed

For each of `full`, `focal`, and `mono`, run:

```bash
KIND=full
RUN_ID=s0_exact_20260916_v1_${KIND}
sbatch --export="ALL,RUN_ID=${RUN_ID}" evaluation_code_PDAC/slurm/run_assemble.sh
sbatch --export="ALL,RUN_ID=${RUN_ID},CANSU_DATASET=COVID_SPIKE" evaluation_code_COVID/slurm/run_assemble_covid.sh
sbatch --export="ALL,RUN_ID=${RUN_ID},CANSU_DATASET=COVID_NONSPIKE" evaluation_code_COVID/slurm/run_assemble_covid.sh
```

Do this for all three values of `KIND`. Assembly audits task receipts and output
hashes, verifies the input package, then requires every exact parameter/MN cell
for every observation. Missing, duplicated, nonfinite, or unrequested cells
abort assembly. Sparse holes are not written as zero-valued rows.

The resulting metadata uses `assembly_schema_version=stage3_survivor_sparse_v2`
and includes `required_parameter_mn_cells`, `n_required_regimes`, and the
survivor-grid path/hash. `n_params` and `n_mn` remain index-axis sizes; their
product is **not** the number of required cells. Consumers must honor the
explicit required-cell list. The parameter and M/N integer indices are local to
each tensor; align components by parameter values and observation identities.
The exact component-specific survivor lists remain in each grid manifest;
the full/mono computational unions are not new tournament candidate rosters.

The subsequent seven-aggregation expansion and joint tournaments are separate
from this Stage 2/assembly update.

## Input provenance and regeneration

`grids/*_grid.json` records the source certificate and registry hashes, each
component/metric survivor identity, and the exact per-geometry M/N files. The
same files are manifest-bound in `complete_f_input_package/`. Regenerate them
with `IRIS_scripts/exact_stage2/build_grids.py` in the Supported Model
Replacement repository, then rebuild the cohort package using the updated
`external_validation_inputs` packaging configuration. The old 18-row files and
`external-validation` grid profile are no longer accepted by this workflow.
