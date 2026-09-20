# IRIS NCI cluster pipeline audit

Historical audit: this records the state **before** the subsequent refactor.
The portable distributed pipeline and finalist stages are now implemented;
see the directory README for the current workflow and validation commands.

Audit date: 2026-09-09. Scope: the current working tree of
`directed_round_robin_organizer`, `iris_nci_parameter_tournament`, and this Slurm
directory, including the existing uncommitted accelerated-execution changes.
This document records an audit and proposed implementation contract; the new
distributed and finalist stages are not implemented by this audit.

## Conclusion

The requested pipeline can be self-contained with respect to scientific input
data. The current scripts and Rust source alone are insufficient: they refer to
14 files in the separate local `Work_Data` tree. Those files can all be included
in one portable package. No upstream IRIS scoring workflow needs to run on the
cluster to reproduce the existing tensor-to-score preparation.

Both crates need changes for the proposed distributed accelerated workflow and
component-finalist selection. The existing comparison kernels, deterministic
pair identifiers/seeds, bundle validation, and Version 2 selection logic provide
the foundation; this audit identifies no need to replace those statistical
kernels to add these stages.

## Required updates

### 1. Package inputs and resolve paths relative to the package

The all-regimes configuration contains absolute Mac paths. `HashedInput`
validates its path directly, and `PipelineConfig::load` does not resolve a
relative input path against the configuration directory. Merely replacing
absolute paths with relative strings would make execution depend on the process
working directory.

Preparation records the canonical original config path in `manifest.json`.
`audit_prepared` later opens that path. Its all-regimes checks also reopen each
component's metric table and primary tensor metadata. Supplying a relocated
config through `audit-prepared --config` checks its hash but does not replace the
manifest's original config reference.

Required: an export/package operation that copies each distinct input once,
verifies its configured hash, creates a portable configuration, and records a
package manifest. Resolve input paths against the configuration/package root;
embed the configuration or reference it within the package. Retain compatibility
with historical configurations and manifests rather than rewriting old runs.

Evidence:
[input validation](/Users/thomm15/Documents/Supported_Model_Replacement/supported_ap_code/crates/iris_nci_parameter_tournament/src/config.rs:67),
[preparation manifest](/Users/thomm15/Documents/Supported_Model_Replacement/supported_ap_code/crates/iris_nci_parameter_tournament/src/bundle.rs:56),
[prepared audit](/Users/thomm15/Documents/Supported_Model_Replacement/supported_ap_code/crates/iris_nci_parameter_tournament/src/bundle.rs:448).

### 2. Choose an explicit finalization data contract

Both Version 2 entry points call `PipelineConfig::load`, which validates every
original metric, tensor, observations, and metadata input. Their selection
calculations then use the loaded organizer bundle and match evidence. Therefore
a prepared-scores-only transfer still requires code changes before finalization
can run without the original tensors.

Recommended initial implementation: include all 14 source files in the portable
package. This preserves source availability for the existing validation contract
and is smaller to transfer than the prepared scores.

An optional prepared-only export would instead need a runtime policy structure
separate from the source-input configuration, embedded roster/grid evidence for
the all-regimes audit, and a clear distinction between validating prepared
artifacts and reproducing scores from the original tensors. Simply disabling
input checks would weaken the existing contract.

Version 2 also records canonical absolute paths for its source files, and its
auditor follows them. Use references relative to a declared run/package root in
the new manifest format so a completed run can be moved and audited. The
auditor currently checks hashes and selection structure, rather than replaying
the entire operational selection; a finalist builder should validate the
expected parent bundle, plan, metric, and policy before accepting a selection.

Evidence:
[exhaustive finalization](/Users/thomm15/Documents/Supported_Model_Replacement/supported_ap_code/crates/iris_nci_parameter_tournament/src/version2.rs:285),
[accelerated finalization](/Users/thomm15/Documents/Supported_Model_Replacement/supported_ap_code/crates/iris_nci_parameter_tournament/src/version2.rs:347),
[source references](/Users/thomm15/Documents/Supported_Model_Replacement/supported_ap_code/crates/iris_nci_parameter_tournament/src/version2.rs:133),
[output audit](/Users/thomm15/Documents/Supported_Model_Replacement/supported_ap_code/crates/iris_nci_parameter_tournament/src/version2.rs:704).

### 3. Add distributed accelerated work to the organizer

The accelerated scheduler is one local coordinator. It loops over targets,
issues Rayon batches of comparisons, constructs the survivor set, completes
survivor-pair reports, and publishes one audited certificate. There is no
accelerated target-shard worker or partial-result merger.

Required additions:

- A deterministic target assignment bound to one immutable accelerated plan.
  Each target retains the full challenger roster, including excluded candidates.
- Workers that publish completed target obligations and exclusion witnesses.
  Missing or interrupted work must remain distinguishable from survival.
- Deterministic ownership of comparison artifacts. Existing publication reports
  a conflict if another writer creates the same artifact concurrently; launching
  duplicate coordinators is not a distributed scheduling solution.
- A merge stage that verifies complete target coverage and survivor obligations.
- A second work manifest/array for missing numerical comparisons within the
  final survivor set, followed by the full operational-input certificate audit.
- Retention of full-context SCC handling where the judge cannot establish an
  ordering capability. Ordered-target shortcuts cannot replace that fallback.

Keep the current local accelerated and exhaustive interfaces. The present
certificate auditor already checks roster partitioning, exclusion witnesses,
survivor obligations, and operational-pair completeness; build on these checks.

Evidence:
[coordinator](/Users/thomm15/Documents/Supported_Model_Replacement/supported_ap_code/crates/directed_round_robin_organizer/src/accelerated/scheduler.rs:130),
[certificate audit](/Users/thomm15/Documents/Supported_Model_Replacement/supported_ap_code/crates/directed_round_robin_organizer/src/accelerated/certificate.rs:400),
[publication conflict](/Users/thomm15/Documents/Supported_Model_Replacement/supported_ap_code/crates/directed_round_robin_organizer/src/artifact.rs:408).

### 4. Add component-finalist bundles to the NCI crate

The crate currently constructs ten component/metric bundles and finalizes each
one independently. It has no command to construct a cross-component bundle.

Add a builder that consumes audited parent selections and their prepared score
bundles, advances all `s_op` members, preserves component/regime identity, and
creates separate PR and ROC finalist bundles. Bind each finalist to its parent
selection and score vector, and require compatible metric-specific policies.

Join observations by biological identity and verify labels, uniqueness, and
complete coverage. Existing endpoint IDs are generated from `obs_idx`, which
alone is not a cross-component identity guarantee. For these particular prepared
inputs, all five components were checked and have the same 4,430 unique
identities, labels, and row order. The implementation should enforce identity
alignment rather than depend on that incidental ordering.

Reuse the organizer and Version 2 logic for the two finalist tournaments.
Preserve exact ties; report a unique winning component only when the final
selected regimes all belong to one component. Explicitly handle empty and
singleton finalist sets instead of assuming five contestants. With one finalist
per component, each metric requires only ten unordered pairs.

This is hierarchical selection among first-stage winners, not a claim of
equivalence to one tournament over all 21,000 component/regime combinations.

Evidence:
[CLI surface](/Users/thomm15/Documents/Supported_Model_Replacement/supported_ap_code/crates/iris_nci_parameter_tournament/src/main.rs:18),
[observation identity fields](/Users/thomm15/Documents/Supported_Model_Replacement/supported_ap_code/crates/iris_nci_parameter_tournament/src/tensor.rs:16),
[endpoint IDs and identity export](/Users/thomm15/Documents/Supported_Model_Replacement/supported_ap_code/crates/iris_nci_parameter_tournament/src/bundle.rs:310).

### 5. Refactor the Slurm submission stages and deployment assumptions

The wrapper currently obtains the repository root through Git, prepares data
synchronously before submission, and creates ten accelerated coordinator tasks.
Its dependent finalizer processes the ten branches and stops there. Its dry run
prints the first array command only, and existing shell tests do not exercise
real `sbatch` dependency submission.

Required: package-root discovery independent of `.git`; preparation on an
allocated compute node; distributed target and survivor-pair stages; dependent
branch finalization; two finalist tournaments; and a final audited summary.
Use the allocated CPU count for Rayon and retain configurable array concurrency.
Record submitted job IDs and support safe resumption without duplicate live
submissions. Print/test the entire dependency plan in dry-run mode. Dynamic work
counts need a supported submission mechanism after the preceding manifests are
created; do not assume compute-node submission is allowed without checking IRIS.

The two crate directories are not a complete source build: both depend on the
root `supported-ap` package through `path = "../.."`. Ship the Rust workspace
and lockfile, or compatible Linux executables built from it. Third-party Cargo
dependencies must be available when building; vendor them if an offline build
is required. This is a software-build requirement separate from scientific data.
The existing scripts also use Bash, Python 3, standard Unix utilities, and Slurm.

Evidence:
[submission wrapper](/Users/thomm15/Documents/Supported_Model_Replacement/IRIS_scripts/nci_parameter_tournament/submit_nci_parameter_tournaments_slurm.sh:7),
[finalizer](/Users/thomm15/Documents/Supported_Model_Replacement/IRIS_scripts/nci_parameter_tournament/finalize_nci_parameter_tournaments_slurm.sh:1),
[workspace](/Users/thomm15/Documents/Supported_Model_Replacement/supported_ap_code/Cargo.toml:11).

## Scientific input inventory

The current all-regimes configuration references these distinct files. Repeated
references from reciprocal component models do not require duplicate copies.

| Input type | Files | Bytes |
| --- | ---: | ---: |
| Score-component tensors | 3 | 310,290,125 |
| Observation tables | 3 | 223,664 |
| Tensor/grid metadata | 3 | 66,053 |
| Regime metric tables | 5 | 2,388,338 |
| Total | 14 | 312,968,180 |

That is 298.47 MiB of source inputs. The existing ten prepared branches occupy
3,666,136,752 bytes, or 3.41 GiB, before match outputs. The recommended package
contains these inputs, portable configuration/manifests, submission scripts, and
the build/runtime payload. Prepared scores and all later artifacts are generated
inside the cluster run directory. No separate `Work_Data` mount is then needed.

## Reproducibility and throughput checks

- The ROC configuration has a two-second optimization time limit as well as a
  node budget. The solver checks a wall-clock deadline. Identical seeds therefore
  do not by themselves guarantee identical optimizer bounds or downstream
  selections under different CPU contention. Retain the declared policy while
  benchmarking; any decision to change time/node limits requires a newly bound
  configuration. See
  [deadline handling](/Users/thomm15/Documents/Supported_Model_Replacement/supported_ap_code/src/auroc/solver.rs:533).
- Each worker currently loads a complete bundle. Many small tasks can repeatedly
  parse the same large score table and read it from shared storage. Benchmark
  target-group size, memory, I/O, and Rayon threads on IRIS before choosing
  production defaults. A compact shared score representation is a possible
  later optimization, not a prerequisite for correctness.
- Build provenance currently probes the compile-time workspace path at runtime;
  Git/compiler metadata can be absent after deployment. Embed immutable build
  metadata and bind the deployed executables in the package manifest. Execution
  provenance is recorded separately from plan provenance; existing validation
  does not enforce that every worker uses one identical executable. See
  [provenance capture](/Users/thomm15/Documents/Supported_Model_Replacement/supported_ap_code/crates/directed_round_robin_organizer/src/provenance.rs:21).

## Verification performed and required acceptance tests

The current release tests passed: 21 organizer tests and 14 NCI tests, 35 total.
All three shell scripts passed `bash -n`.

```sh
cargo test --release --locked --offline \
  --manifest-path supported_ap_code/Cargo.toml \
  --package directed_round_robin_organizer \
  --package iris_nci_parameter_tournament
```

Temporary negative probes confirmed that both finalizers fail immediately when
an original tensor path is unavailable, and that `audit-prepared --config` still
fails when the manifest's original config path is unavailable. The production
run and its input files were not modified by these probes. No tournament or
Slurm job was launched by this audit.

Acceptance tests for the implementation should cover package relocation with
the original source tree unavailable; execution from an unrelated working
directory; changed/missing input hashes; relocation of completed outputs;
distributed versus local/exhaustive selection on small PR/ROC fixtures; shard
coverage, interrupted work, retries and duplicate ownership; SCC fallback;
survivor-pair completion; finalist ties, empty/singleton sets, reordered rows,
label/identity mismatches, and incorrect parent selections. Add a fake-`sbatch`
test for the complete dependency chain and a small Linux/IRIS smoke run before
the production 4,200-regime pipeline.
