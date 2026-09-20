# ProteinGym single-context graph study

This study applies the manuscript's primary framework to the existing 91-assay
ProteinGym cohort. It retains EVE ensemble, ESM-1v ensemble, and ESM-2 650M as the
three-model roster and uses both orientations of every pair in every assay.
Each assay is one empirical evaluation. No empirical subset operator combines
assays; their completed support graphs are reduced afterward.

Candidate-conservative reduction is primary. Replacement-conservative reduction
is also reported to show the different cross-context question. The original
`../proteingym/` joint empirical study and `../proteingym_nested/` conditional
illustration remain unchanged. This study has its own output directory.

## Policy and scope

The existing manual-cutoff, single-substitution cohort, paired labels/scores,
score orientation, and 1%–90% target-prevalence interval are retained. The metric
is tie-averaged prior-standardized paired CNAP. The thresholds are `delta=0`,
`d=0`, and `gamma=0.81`; computational order is `K_C=2`.

Each direction first faces its observed gate. A failed direction stops. For a
passing direction, every order-two computational challenge includes its
observed anchor. Supported magnitude and literal survival must both pass.

The computational procedure samples the observed number of residue positions
with replacement within each assay, carrying every retained substitution at a
selected position. Missing-class draws are rejected as complete draws and
redrawn, with a cap of 10,000 attempts. Model pairs, directions, and target
prevalences share each assay's draws. Stable assay-ID seeds and canonical
variant ordering make draws independent of thread scheduling, pair ordering,
and input row order.

This is a retrospective methodological illustration conditional on that
computational design. Resampling positions tests dependence on represented
positions; it does not create new assays, justify transport to unrepresented
mechanisms, or assert independent biological replications. No model is trained
or tuned. The study does not claim prospective validation of a deployment
policy. It does not include NCI. AUROC is not recomputed in this AP study.

## Run

Use the parent `domain_study/.venv` environment and cached Cargo dependencies.
No new downloads are required when the retained parent scores and audit exist.
From the repository root:

```sh
sh supported_ap_code/domain_study/proteingym_contexts/run.sh publication
```

The publication profile uses 200 computational draws per eligible pair/context,
257 prevalence grid points with local and endpoint-cell minimization, and a `1e-8` search
tolerance. The name identifies numerical settings, not a claim of prospective
study design. The default seed is `20260917`. A quick profile uses 20 draws and
33 grid points, writes to `outputs_quick/`, and is not substituted for the
publication results.

The plan, input hashes, and numerical-source hashes are saved before the run.
Each pair/context result is checkpointed atomically. Repeating the same command
resumes missing jobs and validates all existing results. Changing inputs,
implementation, or numerical parameters requires a different output directory;
existing results are never silently deleted or mixed with another run.

```sh
# Redraw and validate saved publication results without recomputing effects.
sh supported_ap_code/domain_study/proteingym_contexts/run.sh render

# Independent-seed or larger-J sensitivity, saved separately.
sh supported_ap_code/domain_study/proteingym_contexts/run.sh publication \
  --seed 20260918 --replications 400 --output /absolute/path/to/separate-results
```

`RAYON_NUM_THREADS` controls Rust parallelism. The driver can also be invoked
directly with `--example` pointing to a built `proteingym_contexts` example.
The one-row/order-one boundary of the existing Rust nested-support function is
used solely to compute each single-context summary; it performs no aggregation
across assays.

## Validation and outputs

The Python layer independently checks each anchored summary with the ordered
finite-subset formula. It requires all 546 directed results before reduction,
checks the unmodified-library anchors against the previous publication artifacts,
records corrections from endpoint-cell minimization, and verifies
acyclicity and the containment of candidate-conservative survivors within the
replacement-conservative set. Failed gates have no computational assessment.

Numerical checks refine every observed profile and eight deterministically
spaced draws in each computed pair/context at twice the grid density and
tenfold tighter tolerance. The half-length computational prefix is compared
with the complete list. This tests numerical/list-size stability, not
independent-seed reproducibility or statistical confidence. If a checked effect
moves by more than `1e-7`, or a checked gate/floor classification changes, the
study stops before publishing aggregate results.

`outputs/REPORT.md` explains the results and qualifications. `summary.json`
contains the graph reductions and diagnostics. The `tables/` directory contains
all directed assessments, context survivor sets, and model admissibility
counts. `reports/` retains observed effects, eligible computational effects,
and limiting prevalences. `figures/` contains PDF/PNG views of context
admissibility, cross-context counts, and the two-part computational decision.
`manifest.json` hashes the saved artifacts.

Run the study-specific tests from `supported_ap_code/`:

```sh
cargo test --example proteingym_contexts --offline
domain_study/.venv/bin/python -B -m unittest \
  domain_study.proteingym_contexts.test_context_story -v
```

The initial coarse-grid run is retained in `outputs_initial_grid_check/` for
numerical audit only. Its refinement check detected four discrepancies above
`1e-7` and stopped before reporting reductions. The final implementation
explicitly minimizes both endpoint grid cells in each direction, where a
coarse grid can miss a nearby interior extremum. Primary results in `outputs/`
use this corrected search and must pass the same refinement check.
