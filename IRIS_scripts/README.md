# IRIS workflow entry points

This directory contains repository-side packaging, validation, selection, and
cluster orchestration. Biological candidate enumeration is deliberately kept
with the cohort source data under `/Users/thomm15/Work_Data`; this repository
does not maintain a second copy of those generators.

## Directory map

| Directory | Responsibility |
|---|---|
| `configs/input_packaging/` | Inputs to `external_validation_inputs` |
| `configs/pipeline/` | Complete-F downstream pipeline configuration |
| `configs/tournament/` | Active provider and tournament examples |
| `adaptive_selection/` | Adaptive L2 parameter-selection Slurm workflow |
| `tournament/` | Generic directed round-robin Slurm workflow |
| `providers/` | Generic IRIS tournament bundle provider |
| `revisions/pdac/` | PDAC-only context revision |
| `revisions/covid_spike/` | COVID-spike label revision |
| `shared/` | Helpers shared by tournament and revision launchers |
| `docs/` | Design and operating documentation |
| `deprecated/` | Reproducibility-only assets excluded from new runs |
| `tests/` | Repository-side unit tests |

Anything under `deprecated/`, and every file whose basename ends in
`_deprecated`, is historical provenance. New commands and configurations must
not depend on those paths.

## Repository versus Work_Data ownership

`/Users/thomm15/Work_Data/IRIS_scripts` owns the executable Stage-2 boundary:

```text
submit_stage2_new.sh
run_stage_2_new.sh
stage2_run_contract.py
validate_complete_f_stage2_inputs.py
complete_f_stage2/
```

That code resolves cluster paths and consumes cohort-local data. The
repository retains input-package construction, downstream transfer contracts,
adaptive selection, tournament orchestration, providers, configs, tests, and
scientific documentation. Duplicating those repository workflows under
Work_Data would create two editable implementations and is intentionally
avoided.

## Cohort-native biological roster builders

The active generators are:

```text
/Users/thomm15/Work_Data/Cansu_Covid_Nonspike/Marcus_preprocessing/
  build_complete_f_candidate_table.py
  build_complete_f_endpoint_mapping.py

/Users/thomm15/Work_Data/Cansu_Covid_Spike/Marcus_preprocessing/
  build_complete_f_candidate_table.py
  build_complete_f_endpoint_mapping.py

/Users/thomm15/Work_Data/Jayon/Marcus_pre_processing/
  build_complete_f_endpoint_views.py
```

The COVID builders enumerate all eligible 9–12mer windows and attach measured
endpoint responses. The PDAC builder emits both the primary `pdac_full` view
and the nested `pdac_rojas_sethna` secondary view. None applies Kd,
antigen-processing, or stability filtering.

## Package and validate complete-F inputs

From the repository root:

```bash
cargo run --release \
  --manifest-path supported_ap_code/Cargo.toml \
  --package external_validation_inputs -- \
  build \
  --config IRIS_scripts/configs/input_packaging/external_validation_inputs.example.json \
  --input-root /Users/thomm15/Work_Data \
  --output /Users/thomm15/Work_Data/IRIS_scripts/complete_f_input_package

for DATASET in PDAC COVID_SPIKE COVID_NONSPIKE; do
  python3 /Users/thomm15/Work_Data/IRIS_scripts/validate_complete_f_stage2_inputs.py \
    validate \
    --bundle /Users/thomm15/Work_Data/IRIS_scripts/complete_f_input_package \
    --dataset "${DATASET}" \
    --representation full
  python3 /Users/thomm15/Work_Data/IRIS_scripts/validate_complete_f_stage2_inputs.py \
    validate \
    --bundle /Users/thomm15/Work_Data/IRIS_scripts/complete_f_input_package \
    --dataset "${DATASET}" \
    --representation mono
done
```

Then install each dataset's complete package directly into its established
cohort root—without a run directory:

```bash
for DATASET in PDAC COVID_SPIKE COVID_NONSPIKE; do
  python3 /Users/thomm15/Work_Data/IRIS_scripts/validate_complete_f_stage2_inputs.py \
    install \
    --bundle /Users/thomm15/Work_Data/IRIS_scripts/complete_f_input_package \
    --dataset "${DATASET}"
done
```

The validator binds each generated Stage-2 TOML and query to the package
manifest. A declared-but-missing computation is an error; only a completed
zero signal may become the numerical log floor.

## Downstream workflow

After complete tensors exist, start from
`configs/pipeline/iris_fullroster_pipeline.example.json`. Adaptive selection is
launched with:

```bash
bash IRIS_scripts/adaptive_selection/submit_adaptive_hillq_selection_slurm.sh \
  --mode pilot \
  --source-root /path/to/full_roster_transfers \
  --bundle-root /path/to/fixed_l2_bundle \
  --run-root /path/to/adaptive_selection_run \
  --alpha-values "0.5 1 2 4 inf" \
  --q-values "0.5 1 2 inf" \
  --c-min -11.0 --c-max 3.2 --c-step 0.2 \
  --kappa-min 0.02 --kappa-max 4.0 --kappa-points 6
```

These values are the current compact broad-budget recommendation from the
complete-F label-blind diagnostics. Once chosen for a run, the launcher
requires every grid axis explicitly and persists it in both the immutable
planning and final-selection environments.

The generic tournament launcher is:

```bash
bash IRIS_scripts/tournament/submit_directed_round_robin_slurm.sh \
  --mode pilot \
  --config /path/to/complete_f_tournament_config.json \
  --run-root /path/to/tournament_run \
  --covid-spike-label-set threshold_zero
```

The launchers resolve the repository root through Git, so moving them into
subdirectories does not change binary paths. Worker/finalizer paths are
resolved relative to their workflow directory; shared helpers are referenced
explicitly through `../shared` or `../../shared`.

## Detailed plans

- [`../COMPLETE_F_FULL_ROSTER_REGENERATION_PLAN.md`](../COMPLETE_F_FULL_ROSTER_REGENERATION_PLAN.md)
  is the production replacement runbook.
- [`../COMPLETE_F_INPUT_ROSTERS_AND_PDAC_VIEWS_IMPLEMENTATION_PLAN.md`](../COMPLETE_F_INPUT_ROSTERS_AND_PDAC_VIEWS_IMPLEMENTATION_PLAN.md)
  explains roster identity, deduplication, and PDAC view semantics.
- [`docs/iris_fullroster_round_robin_plan.md`](docs/iris_fullroster_round_robin_plan.md)
  documents the downstream model/tournament architecture.
- [`docs/CLUSTER_RUNBOOK_deprecated.md`](docs/CLUSTER_RUNBOOK_deprecated.md)
  preserves commands for the superseded filtered-roster tournament and must not
  be used for complete-F regeneration.

## Tests

```bash
python3 -m unittest discover -s IRIS_scripts/tests -p 'test_*.py' -v
python3 -m unittest discover \
  -s /Users/thomm15/Work_Data/IRIS_scripts/tests \
  -p 'test_validate_complete_f_stage2_inputs.py' -v
python3 -m unittest discover \
  -s /Users/thomm15/Work_Data/Jayon/Marcus_pre_processing/tests \
  -p 'test_build_complete_f_endpoint_views.py' -v
```
