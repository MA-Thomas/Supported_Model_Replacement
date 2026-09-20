# Validation record

Validated on 2026-09-16 against the merged NCI certificates in
`cluster_outputs/nci_all4200_cluster_12cpu_v1`.

- Generated exact unions: full 28 regimes / 20 geometries; focal 27 / 20;
  mono 89 / 40. All six frozen tau values are retained. The manifests preserve
  the 191 PR and 25 ROC survivor memberships, including cross-component reuse.
- The existing Rust cohort-input builder rebuilt and validated the package:
  129 manifest-bound output files, validation status `pass`. Every retained
  cohort input file is byte-identical to the preceding package.
- All 18 launch-mode dry runs passed: Q/Pi and P/N for all nine
  dataset/construction combinations, including mono environment counts and
  representation crosswalks. The test used local path overrides and did not
  call `sbatch` or execute scientific Runner calculations.
- 22 Python tests passed. The new integration test uses an instrumented fake
  Runner to check the actual shell worker interface for two geometries with
  disjoint M/N sets, all six taus, and peptide sharding. It executes both shard
  finalizers and verifies the published parent receipts and checksums.
- Both real Rust assembly binaries successfully assembled that synthetic raw
  output into exactly 24 tensor rows (two observations × 12 requested cells),
  with an explicit 12-cell sparse metadata list. Neither computed nor emitted
  the 12 unrequested Cartesian cells per observation.
- Assembly-specific Rust tests passed for each codebase: manifest contracts,
  exact grid coverage, required observation rosters, missing Q/Pi, and
  rejection of duplicate/missing/extra cells. The new sparse validator also
  rejects substitution of a duplicate cell for a missing cell even when row
  counts would otherwise match.
- Changed input hashes and mismatched construction/representation flags are
  rejected. The assembler verifies every recorded raw output's size and hash.
- Shell syntax checks passed. Installation was smoke-tested against a temporary
  copy, including removal of the two retired fixed-grid CSV interfaces.
- The source hashes of all 25 Rust files under Runner, TCR_availability,
  MHC_competition_solver, and pMHC_TCR_interaction_strength were recorded and
  checked during installation. No scientific numerical routines were edited.

The initial exploratory `cargo test --lib` run also exposed the existing
unrelated `tests::test_auc_perfect` failure in the evaluation library. The AUC
function and its test are unchanged; this task did not alter them. The relevant
assembly tests above were run independently and pass. This is not a claim that
all unrelated tests in those older evaluation crates pass.

No cluster jobs or real Stage 2 computations were run. Expanded M/N workloads
need a small cluster resource check before full submission; the prior 60G /
4,000-observation shard settings are starting defaults, not new measurements.

Resource defaults updated on 2026-09-16 at the user's request: P/N uses 16 CPUs
and 60G; Q/Pi uses eight CPUs and 30G. The 4,000-observation shard target and
wall-time limits are unchanged. All 18 computation-mode preflights passed;
all 27 generated Slurm requests (including nine shard-merge arrays) were
checked for CPUs, memory, and time. The installed launcher and README match
the source overlay. No jobs were submitted. `resource_update_receipt.json`
records the installed file hashes and backup location.

The default array concurrency was subsequently lowered to 11 to keep one full
launch below the requested 2,500-CPU ceiling. All 18 mode preflights passed
again; all 27 generated array commands use `%11`. With the default CPU
requests, the nine P/N arrays and nine Q/Pi arrays total at most 2,376 CPUs.
Dependent merge arrays replace their completed P/N arrays and use fewer CPUs.
This is a per-launch bound, excluding unrelated jobs and additional launches;
resource overrides require recalculating it. No cluster jobs were submitted.

The exact output schema is `stage3_survivor_sparse_v2`. Downstream all-grid
consumers must use its explicit required-cell list rather than assume
`n_params * n_mn` cells per observation. Building the seven aggregation variants
and the joint tournament bundles remains subsequent work.
