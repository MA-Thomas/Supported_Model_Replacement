# select_adaptive_hillq

Rust implementation of the authoritative self-gated adaptive Hill-q L2
aggregation and parameter-selection workflow.

For each endpoint it forms the complete 9--12-mer candidate roster, removes
exact duplicates by endpoint + n-mer + normalized HLA, assigns
`ln(1e-12)` to declared uncomputed candidates, and computes

```text
C_q = log(sum(exp(z - max(z)))) * (1 - 1/N_q)
S   = max(z) + C_q * sigmoid((c - S) / kappa).
```

The implicit equation is solved by bisection on its certified interval. For
the PR branch, every `(q, c, kappa)` candidate is compared directionally with
the same component model's fixed-`max` reference using the complete staged
CNAP procedure. The parameter-selection challenge inherits the validated PR
tournament contract except for its separately declared finite roster of 200
replications; the downstream 600-replication replacement tournament is not
changed. The observed gate is evaluated first; only candidates that pass it
receive the full staged projection challenge. Parameter triples are
eligible only when CNAP supports adaptive-over-`max` in every cohort required
by the selection policy. Exact duplicate score rankings (including tie blocks)
share one cached assessment. The ROC branch retains its AUROC-based selection.

Two independent output policies are produced:

- `pdac_only`: PDAC determines the selected triple; SPIKE and NONSPIKE are
  directional transport diagnostics.
- `all_contexts_equal_weight`: PDAC, SPIKE, and NONSPIKE determine the selected
  triple with equal cohort weight.

The crate also writes leave-one-cohort-out diagnostics, endpoint scores,
baseline reconstruction audits, compressed full surfaces, and a hashed
selection manifest. Large surface CSVs are streamed into gzip rather than
materialized as formatted strings.

## Build and test

From `supported_ap_code/`:

```bash
cargo build --release -p select_adaptive_hillq --bins
cargo test -p select_adaptive_hillq
```

Rust 1.85+ is required.

## Inputs

`--source-root` must be a validated Rust transfer package containing
`manifest.json` and all 30 transfer tasks:

```text
<pdac|covid_spike|covid_nonspike>/<model>/<pr|roc>/
  long_peptide_predictions.csv
  target_tau_selection_by_observation.csv
  summary.json
```

Each `summary.json` names the corresponding full-roster mapping CSV. Mapping
rows must use `mapping_status=scoreable|floor` and must retain HLA for both
statuses. Scoreable rows must resolve exactly once to the tau table; floor rows
must not resolve to it. Prediction files must use `l2_variant`.

`--bundle-root` supplies the authoritative endpoint identities, labels, COVID
label contracts, exact model-matched fixed-`max` reference scores, and the
validated PR tournament contract. Reconstructed fixed-`max` scores are checked
against that reference before PR selection begins.

## Cluster execution

The dense PR grid is executed through an immutable, restartable lifecycle:

```text
select_adaptive_hillq_cluster plan
select_adaptive_hillq_cluster run-shard
select_adaptive_hillq_cluster status
select_adaptive_hillq_cluster audit
select_adaptive_hillq --cnap-plan ... --cnap-results ...
```

Planning deduplicates exact score rankings and tie blocks within each
model/cohort. Each result shard is bound to the plan, input hashes, selection
contract, and planning executable. Existing valid shards are safely reused.
The final selector audits the complete shard set before reconstructing the
full surfaces and applying either selection policy.

The production Slurm entry point is
`IRIS_scripts/submit_adaptive_hillq_selection_slurm.sh`. It supports `pilot`
and `full` modes, bounded sequential array batches, measured resource reports,
and an `afterok` final audit/reduction.

## Monolithic run

This remains available for small grids and development checks. The production
dense grid should use the cluster lifecycle.

```bash
cargo run -p select_adaptive_hillq --release -- \
  --source-root /path/to/full_roster_transfers \
  --bundle-root ../IRIS_scripts/prebuilt_bundles/full_roster_fixed_l2 \
  --output ../IRIS_scripts/analysis_reports/adaptive_hillq_dual_selection_v3 \
  --selection-replications 200 \
  --q-values 0 0.5 1 1.5 2 3 4 inf
```

Important options:

```text
--solver-absolute-tolerance 1e-10
--solver-max-iterations 64
--near-optimal-rank-tolerance 0.01
--selection-replications 200
--c-min/-max/-step
--kappa-min/-max/-points
--threads <N>
```

The output directory must not already exist. Selected `c` or `kappa` values on
a grid boundary cause a fail-closed error requesting a wider grid.

## Principal outputs

- `joint_parameter_grid.csv`
- `selected_parameters.csv` (both policies, 20 rows)
- `selected_parameters_pdac_only.csv` (10 rows)
- `selected_parameters_all_contexts.csv` (10 rows)
- `selected_endpoint_scores.csv`
- `leave_one_cohort_out.csv`
- `baseline_reproduction.csv`
- `surfaces/<branch>__<model>/*.csv.gz`
- `selection_manifest.json`
