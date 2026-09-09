# IRIS full-roster and frozen-hybrid 65-system round robin

## Purpose and current status

This document records both the earlier coverage-by-omission workflow and its
complete-F replacement. Sections 1--13 are historical design and provenance;
they must not be used as production instructions. The current contract is
stated immediately below: one frozen adaptive L2 function is applied unchanged
to every component model and enters the tournament alongside the twelve fixed
L2 operators. The earlier adaptive parameter-selection and 70-system
dual-policy design is superseded.

The revised scientific contract is settled: long-peptide L2 aggregation must
use a complete computed Level-1 tensor over the endpoint-specific candidate
roster. The numerical zero-signal floor is

\[
\log(10^{-12}) \approx -27.6310211.
\]

The complete-F tensors and downstream transfers have been regenerated under an
immutable schema-3 contract. The relevant Rust components are:
The relevant Rust components are:

1. `external_validation_inputs`: constructs and audits the authoritative full candidate rosters
   and the full/mono representation mappings.
2. `iris_fullroster_pipeline`: executes 30 primary transfer jobs plus ten
   metrics-only Rojas/Sethna secondary-view jobs and directly builds the
   provenance-pinned 65-system PR/ROC bundles.
3. `directed_round_robin_organizer`: plans, evaluates, reduces, and audits each
   65-system tournament.
4. `select_adaptive_hillq`: development and audit machinery; it is no longer a
   production handoff in this tournament.

Every handoff requires immutable typed configuration and exact content hashes.
Legacy parity is not part of the production acceptance contract, although the
60 fixed score vectors were checked for exact preservation during this
regeneration.

---

## Current production contract (supersedes Sections 1--13)

The frozen adaptive system is
`endpoint_local_epitope_second_hla_hybrid_v1`, with
`c=-2.2`, `kappa=0.13`, `t=-6.45`, `delta=0.02`, `B=1`, and `w=0.12`.
These constants were developed in the Full-HLA context and are transported
unchanged to Full HLA, Focal HLA, Old monoallelic, Mono-Q/full-PN, and
Full-Q/mono-PN. PR and ROC use their respective frozen Level-1 score branches;
the adaptive L2 function and constants do not change by metric, cohort, or
component model.

Per metric, the roster is:

- 60 fixed systems: five component models times twelve fixed L2 operators;
- five frozen-hybrid systems: one per component model.

The direct bundle has 65 systems. Across PDAC, COVID SPIKE, and COVID
NONSPIKE, each metric therefore requires

\[
\binom{65}{2}\times3=6{,}240
\]

pair-evaluation matches. There is no adaptive-selection stage, no dual-policy
union, and no post-tournament policy-induced view.

The production commands are:

```bash
PIPELINE=supported_ap_code/target/release/iris_fullroster_pipeline

$PIPELINE validate-config --config /path/to/pipeline.schema3.json
$PIPELINE transfer-batch --config /path/to/pipeline.schema3.json \
  --output /new/path/full_roster_transfers
$PIPELINE validate-transfers --package /new/path/full_roster_transfers
$PIPELINE build-tournament-bundles \
  --config /path/to/pipeline.schema3.json \
  --transfers /new/path/full_roster_transfers \
  --output /new/path/full_roster_65_systems
```

Bundle construction validates the exact adaptive constants and requires 13
systems per component model. The cluster helper independently rejects a bundle
unless it has exactly 65 systems and exactly the five expected frozen-hybrid
identifiers. Build the organizer and generate tournament plans on IRIS so the
plan records the Linux executable identity.

The immutable local realization is documented in
`artifacts/iris_fullroster_frozen_hybrid_v1_20260905/IMMUTABLE_CONTRACT.md`.

---

## 1. The original problem: scoreable-only aggregation

> **Historical design, superseded for production.** Sections 1--5 document
> how the omission-floor artifacts were constructed. Their claims about
> assigning floors and reusing Stage-2 tensors do not apply to the complete-F
> workflow in Section 6.

Immunogenicity labels attach to measured **long-peptide endpoints**, while the mechanistic score
\(F=P\cdot N\cdot Q\cdot\Pi\) is computed for a shorter candidate defined by an
`(n-mer, HLA allele)` pair. L2 aggregation collapses the candidate scores for one long peptide
into one endpoint score.

Two candidate sets must be distinguished:

- \(R_j\), the **scoreable roster**, contains candidates that passed the upstream
  presentation/stability filters and therefore have an observation in the F tensor.
- \(U_j\), the **full candidate roster**, contains every 9--12-mer frame linked to the measured
  long peptide paired with every distinct allele in that patient's full HLA environment.

The old transfer path aggregated over \(R_j\). It used a floor only when \(R_j\) was empty, so
gate-failed candidates were usually absent rather than represented as finite low evidence. This
is harmless for `max` whenever at least one scoreable candidate exists, but it changes
coverage-sensitive operators such as `mean`, `median`, `top-k`, and `top-fraction`. It can
therefore leak the number of gate-passing candidates into the endpoint score.

The corrected contract aggregates over \(U_j\):

- a candidate in \(R_j\) is `scoreable` and receives its tensor-derived score;
- a candidate in \(U_j\setminus R_j\) is `floor` and receives \(\log(10^{-12})\);
- a candidate outside the measured endpoint's \(U_j\) is not part of the analysis.

This last distinction matters. For COVID SPIKE, the corrected roster has 104,815 distinct
`(patient, n-mer, HLA)` candidates, not all 117,250 candidates in the raw Stage-2 enumeration.
The omitted candidates belong to patient/peptide combinations that were never measured as a
long-peptide endpoint and therefore do not belong to any endpoint-specific \(U_j\).

---

## 2. Scientific contracts that are now fixed

### 2.1 Candidate identity and duplicate removal

Within an endpoint, a biological candidate is identified by

```text
(endpoint identity, nmer, normalized HLA)
```

where endpoint identity is:

- COVID SPIKE: `(patient_id, mutation, long_peptide)`;
- COVID NONSPIKE: `(patient_id, long_peptide)`;
- PDAC: `(patient_id, long_peptide)`.

The builder removes exact duplicate rows. The fixed scorer and adaptive selector then count each
biological candidate key once before L2 aggregation. Conflicting endpoint labels,
scoreability declarations, or tensor joins fail closed rather than being reconciled implicitly.

### 2.2 HLA and mapping status

Every 9--12-mer mapping row must have:

- a populated, normalized `HLA-RE`;
- `mapping_status` equal to exactly `scoreable` or `floor`;
- a representation-appropriate `env_id`.

A `scoreable` row must resolve to one tensor/tau observation. A `floor` row must not resolve to
one. Either disagreement indicates a mapping/tensor mismatch and aborts the run.

### 2.3 Floor semantics

The selected floor is \(\log(10^{-12})\). It is a modeling choice representing a plausible
log-\(F\) value for an uncomputed observation, not a numerical approximation to negative
infinity. Full-roster means and medians are expected to be dominated by floors.

The input builder records the categorical status `floor`; it deliberately does **not** assign a
numeric score. The transfer and adaptive scorers own the numerical floor and must both enforce
the same value.

An empty endpoint receives the floor. A nonempty all-floor adaptive roster follows the
authoritative self-gated equation; with multiple candidates its corroboration term can make its
score greater than the bare floor.

### 2.4 Terminology

The long-peptide aggregation layer is **L2**. “L3” is deprecated terminology. Historical files
may contain fields such as `l3_variant`; the Rust-native production path rejects them and uses
`l2_variant` and `l2_aggregation_*` names exclusively.

---

## 3. Repository and ownership boundaries

| Location | Contents | Production role |
|---|---|---|
| `/Users/thomm15/Work_Data` | immutable cohort sources, existing Stage-2 tensors, NCI summaries, and the current Rust fixed-L2 scorer | Data and existing heavy-compute artifacts |
| `supported_ap_code/crates/external_validation_inputs` | deterministic full-roster input construction | Rust-native input package |
| `supported_ap_code/crates/select_adaptive_hillq` | adaptive Hill-\(q\) surfaces, selection, and endpoint scoring | Rust-native adaptive L2 |
| `supported_ap_code/crates/directed_round_robin_organizer` | tournament planning, judging, reduction, and induced views | Rust-native tournament engine |
| `IRIS_scripts/` | configurations, documentation, and cluster wrappers | Production configuration and operational entry points |

Data do not need to move into the git repository. The goal is to centralize the executable
scientific logic and contracts in Rust while retaining large or sensitive source data under
`Work_Data`.

Generated input packages and transfer roots are immutable run artifacts. A production run must
use new output directories rather than overwrite or partially reuse stale scoreable-only trees.

---

## 4. Rust-native full-roster input construction

### 4.1 What was added

The new workspace crate is:

```text
supported_ap_code/crates/external_validation_inputs
```

It replaces `build_mono_inputs_deprecated.py` as the production implementation. Legacy interfaces are not
part of the Rust-native contract.

The crate exposes two commands:

```text
external_validation_inputs build
external_validation_inputs validate
```

`build` reads an explicit JSON configuration and input root, constructs the complete package in
an adjacent staging directory, validates it, and atomically renames it into place. It refuses to
run if the requested output already exists. `validate` independently rechecks an existing
Rust-native package without modifying it.

### 4.2 Authoritative input boundary

The configuration is
`IRIS_scripts/configs/input_packaging/external_validation_inputs.example.json`. For each cohort it identifies:

- the filtered Stage-2 query roster;
- the full HLA environment dictionary;
- the source long-peptide-to-n-mer mapping;
- the binding-simulator TOML template;
- endpoint identity fields;
- the response or committed label column;
- any prespecified label threshold;
- frozen expected counts and SHA-256 hashes.

This crate begins from the authoritative query, environment dictionary, and source long-peptide
mapping. It does not claim to regenerate every source mapping from raw clinical records. In
particular, the expanded PDAC mapping and its decision log are explicit immutable inputs; the
clinical workbook and historical generator are retained as hashed provenance. Porting the raw
clinical preprocessing would be a separate project and is not necessary for the corrected
round-robin roster.

### 4.3 Construction algorithm

For each cohort, `build` performs the following deterministic operations:

1. Normalize HLA names by uppercasing, removing an optional `HLA-` prefix, `*`, and `:`.
2. Read the full environment dictionary and reject duplicate or empty environments.
3. Validate that every query HLA belongs to its declared full environment.
4. Remove exact duplicate computational query rows. Conflicting rows with the same computational
   identity fail closed.
5. Assign deterministic contiguous mono environment IDs to sorted `(full_env_id, HLA)` pairs.
6. Verify that full and mono query representations preserve the same biological identities.
7. For every source endpoint/n-mer link, enumerate every distinct allele in the endpoint's full
   HLA environment.
8. Mark an `(n-mer, HLA)` pair `scoreable` when it exists in the filtered query roster and
   `floor` otherwise.
9. Give mono scoreable rows their true mono environment ID. Give mono floor rows a reserved ID
   outside the valid mono range, preventing accidental tensor joins.
10. Remove exact duplicate mapping rows and validate all scoreable mappings against the query.
11. Require every scoreable n-mer to be contained in its linked long peptide.
12. Verify that roster expansion does not change the endpoint population or endpoint labels.
13. Materialize the prespecified COVID SPIKE binary label columns.
14. Write all CSV, TOML, audit, provenance, and manifest artifacts deterministically.

### 4.4 Output package

The primary contract is the package-level `manifest.json`. It records:

- crate and schema identity;
- the configuration hash;
- hashes and byte counts for authoritative sources;
- per-cohort audits;
- provenance inputs;
- hashes and byte counts for every generated output except the manifest itself.

For each cohort prefix (`pdac`, `covid_spike`, or `covid_nonspike`), the package contains:

| Artifact | Meaning |
|---|---|
| `*_candidate_roster.csv` | representation-neutral roster with `full_env_id`, `mono_env_id`, HLA, and status |
| `*_full_longpep_mapping.csv` | ready for a full/focal primary tensor; `env_id = full_env_id` |
| `*_mono_longpep_mapping.csv` | ready for a mono primary tensor; scoreable rows use mono IDs and floor rows use reserved IDs |
| `*_mono_query.csv` | Stage-2-compatible mono query roster |
| `*_mono_hla_env_dict.csv` | one HLA allele per mono environment |
| `*_full_to_mono_env_crosswalk.csv` | deterministic environment crosswalk |
| `*_full_to_mono_observation_crosswalk.csv` | biological observation crosswalk |
| `*_inputs_audit.json` | cohort-level construction and label audit |

PDAC additionally receives the deduplicated full query and its generated TOML. The package also
contains generated mono TOMLs, the frozen parameter/MN tables, and explicitly configured
provenance copies.

No `mono_inputs_manifest.json` or legacy audit aliases are emitted. Downstream Rust code must
consume `manifest.json` directly.

### 4.5 Representation routing

The mapping is selected from the representation of the **primary P/N tensor**, not from an
external Q source:

| Component model | Primary tensor representation | Mapping |
|---|---|---|
| Full HLA | full | `*_full_longpep_mapping.csv` |
| Focal HLA | focal/full environment | `*_full_longpep_mapping.csv` |
| Mono-Q / full-PN | full | `*_full_longpep_mapping.csv` |
| Old monoallelic | mono | `*_mono_longpep_mapping.csv` |
| Full-Q / mono-PN | mono | `*_mono_longpep_mapping.csv` |

Because both mappings are generated from the same canonical roster, they differ only in
representation-specific `env_id` values.

### 4.6 Fail-closed package validation

The Rust validator:

- recomputes every recorded output hash;
- requires populated HLA and `scoreable|floor` status on every mapping row;
- verifies canonical, full, and mono row counts and column contracts;
- verifies that full and mono mappings differ only in `env_id`;
- requires `env_id == full_env_id` in every full mapping row;
- verifies that canonical `mono_env_id` values reproduce the mono mapping;
- recomputes scoreable/floor counts and compares them with each cohort audit.

The build configuration separately freezes expected source hashes and cohort counts. A changed
query, environment dictionary, source mapping, TOML template, PDAC provenance file, or frozen
Stage-2 parameter table fails before publication.

### 4.7 Validation evidence

The crate has unit and end-to-end tests covering:

- HLA normalization and deterministic environment chunking;
- TOML path replacement while preserving comments;
- full-roster allele expansion and `scoreable|floor` assignment;
- canonical/full/mono representation consistency;
- exact query duplicate removal;
- refusal to overwrite an existing output;
- atomic cleanup after a failed build;
- failure on a non-contained scoreable n-mer;
- detection of post-build mutation through hash validation;
- absence of backward-compatibility manifests and audit aliases.

A real three-cohort build also passed. All reference CSV outputs produced by the old builder
were reproduced byte-for-byte, including the three authoritative mono mapping hashes:

| Cohort | rows | scoreable | floor | authoritative SHA-256 |
|---|---:|---:|---:|---|
| COVID SPIKE | 137,175 | 3,658 | 133,517 | `4cda7da9fb6600f8dd163effe5f21f75296dd5391ed947dfa3c9d1de35eddda` |
| COVID NONSPIKE | 199,124 | 5,553 | 193,571 | `258ef6a15e01a9054c83213e0e3ed3a1a8c1b5ba697d3cd82682daccc139c645` |
| PDAC | 9,315 | 2,309 | 7,006 | `82f1e51305c02160baef917bb08c5158085f883a4d3121d211e2d083663e6567` |

All workspace tests and strict workspace Clippy checks pass. The release binary builds
successfully.

The mono queries themselves are byte-identical to the prior audited outputs. Therefore the
Stage-2 compute roster has not changed, and the existing F tensors do not need to be recomputed.
Only the downstream transfer aggregation must be rerun over the expanded candidate rosters.

---

## 5. Fixed-L2 transfer scoring

The production scorer is the `transfer-batch` command in
`supported_ap_code/crates/iris_fullroster_pipeline`. Its transfer path is mapping-driven:

- observations are indexed by `(patient, env_id, nmer, normalized HLA)`;
- a scoreable mapping must resolve uniquely to an observation;
- a floor mapping must not resolve to an observation;
- exact biological candidates are counted once per endpoint;
- E-vac correction applies only to scoreable candidates;
- all twelve fixed L2 operators run over the full roster.

The 30 required transfer outputs are:

```text
3 cohorts x 5 component models x 2 metric-selected NCI branches
```

Each output must include at least:

- `long_peptide_predictions.csv`;
- `target_tau_selection_by_observation.csv`;
- `summary.json` with the exact mapping path and floor contract.

The typed configuration freezes `log_epsilon = 1e-12`; any other value fails before scoring.
The package records the configuration and input-package manifest hashes, validates all 30 job
scopes and representation routes, emits `l2_variant`, and atomically publishes an immutable
transfer tree.

---

## 6. Complete-F self-gated power-mean-anchor Hill-q L2

The omission-floor full-roster outputs are provisional and must not be used to
freeze the adaptive aggregator. First rerun the Level-1 F tensor for every
declared COVID and PDAC 9--12-mer/HLA tuple using the frozen Level-1 regime.
After that rerun, every retained mapping row must resolve to one computed tau
score; an omitted value is missing data, not a numerical floor.

For computed log scores \(z_i=\log(F_i+10^{-12})\), the replacement rule uses
the generalized power-mean anchor

\[
A_\alpha=\alpha^{-1}\log\left(K^{-1}\sum_i e^{\alpha z_i}\right),
\]

with continuous limits \(A_0=\operatorname{mean}(z)\) and
\(A_\infty=\max(z)\). Thus alpha 0, 1, and infinity recover mean,
log-mean-exp, and maximum anchors. Define floor-free signal
\(x_i=\max(e^{z_i}-10^{-12},0)\), Hill breadth \(D_q=1-1/N_q\), and

\[
U=\log(10^{-12}+\sum_i x_i),\qquad
C_{\alpha,q}=D_q\max(U-A_\alpha,0).
\]

The endpoint score is the unique solution of

\[
S=A_\alpha+C_{\alpha,q}\,\sigma((c-S)/\kappa)
\]

on \([A_\alpha,A_\alpha+C_{\alpha,q}]\). An all-zero completed roster has
score \(\log(10^{-12})\) and zero corroboration.

`supported_ap_code/crates/select_adaptive_hillq` now implements one replacement
contract with no legacy mode:

1. complete-F loading with omission-floor rows rejected;
2. exact candidate-set deduplication;
3. independent reconstruction of fixed `max` and `logsumexp` baselines;
4. explicit alpha and Hill-order grids;
5. certified bisection for the self-gated score;
6. the joint `(alpha,q,c,kappa)` surface per model/metric/cohort;
7. directional staged paired-CNAP comparison with each component model's fixed `max`;
8. PDAC-only and equal-context selection restricted to staged-supported tuples;
9. worst-cohort fractional rank as the primary all-context objective;
10. leave-one-cohort-out transport diagnostics and hashed provenance.

The two policies have different meanings and remain separate:

- **PDAC-only:** PDAC selects `(alpha,q,c,kappa)`; COVID cohorts evaluate the frozen selection.
- **All-context:** PDAC, SPIKE, and NONSPIKE select jointly, worst-cohort first.

Both families enter one tournament with distinct identifiers. The Rust loader fails before the
surface sweep if roster status, observation joins, endpoint labels, or reconstructed baselines
disagree.

The parameter-selection challenge uses a separately declared 200-replication finite roster
with computational order two. All other PR settings are inherited from the validated fixed-L2
bundle. This does not modify the later 600-replication replacement tournament: selected systems
must still survive that complete tournament, and failure there is reported rather than repaired
by silently selecting another parameter triple.

The dense PR calculation uses the same operational structure as the tournament engine. An
immutable plan deduplicates exact ranking/tie signatures; restartable Slurm shards compute the
paired-CNAP outcomes; `status` and `audit` verify completeness and provenance; and the final
selector reconstructs the complete surfaces from the audited cache. The production wrapper is
`IRIS_scripts/adaptive_selection/submit_adaptive_hillq_selection_slurm.sh`.

No production alpha grid or transport threshold is frozen in this plan. Those
must be preregistered after the complete-F rerun and clean-data diagnosis. The
earlier max-anchored adaptive path and every omission-floor surface are invalid
inputs to the replacement selector.

---

## 7. Why one 70-system tournament

Per metric, the intended system roster is:

- 60 fixed systems: five component models times twelve fixed L2 operators;
- five PDAC-selected self-gated systems;
- five all-context-selected self-gated systems.

The union has 70 systems. Across three cohorts it requires

\[
\binom{70}{2}\times3=7{,}245
\]

pair-cohort matches. Two separate 65-system tournaments would require

\[
2\times\binom{65}{2}\times3=12{,}480.
\]

The union therefore saves 5,235 comparisons while adding only the 75 comparisons between the
two adaptive five-system families.

After the common atomic verdict table is complete, graph reduction is performed three ways:

1. the complete 70-system graph;
2. a 65-system induced graph excluding the five all-context systems;
3. a 65-system induced graph excluding the five PDAC-only systems.

`directed_round_robin_organizer induce-view` recomputes strongly connected components, source
components, and graph-maximal systems inside each declared node set. Maximality is not inherited
from the 70-node graph.

---

## 8. End-to-end architecture

```text
immutable cohort sources + complete endpoint candidate roster
                    |
                    v
complete-F query/mapping build                           [update required]
    -> manifest.json
    -> all 9--12-mer/HLA tuples, queries, mappings, audits
                    |
                    v
Level-1 F tensor rerun under frozen L1 settings          [compute required]
                    |
                    v
iris_fullroster_pipeline transfer-batch                  [input update required]
    -> 30 transfer directories
                    |
                    +----------------------+
                    |                      |
                    v                      v
iris_fullroster_pipeline           select_adaptive_hillq  [selector complete]
    build-fixed-bundles                -> both policies
                    |                      |
                    +----------+-----------+
                               v
iris_fullroster_pipeline build-combined-bundles         [Rust, complete]
    -> combined 70-system bundle
                               |
                               v
directed_round_robin_organizer plan/run/reduce         [Rust, complete]
                               |
                 +-------------+-------------+
                 v                           v
          PDAC-only induced view       all-context induced view
```

Legacy builders and providers are not production dependencies and are outside the acceptance
contract.

---

## 9. What is stale and must not be reused

- Existing `prebuilt_bundles/` were built from scoreable-only transfer outputs.
- Existing adaptive reports are scoreable-only and/or use the deprecated fixed-\(q=2\) contract.
- The default transfer root contains stale normalized mappings, including historical statuses
  other than `scoreable|floor` and blank HLA values.
- A `summary.json` can contain an absolute mapping path. Copying an old transfer tree does not
  redirect that provenance and can silently retain the stale mapping.
- Existing fixed-L2 mean, median, top-fraction, log-sum-exp, and adaptive scores cannot be
  repaired by swapping a mapping file after scoring.

Production regeneration must use new complete-F input, tensor, transfer,
selection, base-bundle, and combined-bundle directories. The earlier Stage-2
tensors are incomplete for this roster and must not be reused as production
complete-F inputs.

---

## 10. Implemented all-Rust production contract

### Complete in code

- Deterministic complete-F input construction from measured endpoint mappings, including
  generated Stage-2 TOMLs and a nested PDAC Rojas/Sethna analysis view.
- Workspace-native transfer batch with 30 primary and ten explicitly non-selectable,
  non-bundle-eligible secondary jobs in a typed package manifest.
- Rust complete-F power-anchor Hill-\(q\) loader, scoring, surface sweep, and both policies.
- Rust fixed- and combined-bundle builders with 60/70-system dimension checks.
- Rust tournament planning, deterministic scientific seeds, judging, reduction, and induced
  views.

The handoffs fail closed unless the same configuration hash, input-package manifest hash,
transfer-manifest hash, and PR/ROC base-bundle content hashes are preserved. The selector accepts
only a validated Rust transfer package and L2 terminology. A compact integration fixture covers
the input package, 40 transfers, 60-system bundles, selection artifact, 70-system bundles, and
7,245-match tournament plans, together with wrong-floor, stale-mapping, configuration-mismatch,
and base-bundle-mismatch failures.

---

## 11. Commands available now

### 11.1 Build the implemented Rust crates

```bash
cargo build --release --manifest-path supported_ap_code/Cargo.toml \
  --package external_validation_inputs \
  --package iris_fullroster_pipeline \
  --package select_adaptive_hillq \
  --package directed_round_robin_organizer
```

### 11.2 Build and validate a full-roster input package

The output must not already exist:

```bash
supported_ap_code/target/release/external_validation_inputs build \
  --config IRIS_scripts/configs/input_packaging/external_validation_inputs.example.json \
  --input-root /Users/thomm15/Work_Data \
  --output /path/to/full_roster_inputs_v1

supported_ap_code/target/release/external_validation_inputs validate \
  --bundle /path/to/full_roster_inputs_v1
```

### 11.3 Run the transfer batch and build fixed bundles

Start from `IRIS_scripts/configs/pipeline/iris_fullroster_pipeline.example.json`, replace every path and hash,
and validate the frozen configuration before production use.

```bash
supported_ap_code/target/release/iris_fullroster_pipeline validate-config \
  --config /path/to/iris_fullroster_pipeline.json

supported_ap_code/target/release/iris_fullroster_pipeline transfer-batch \
  --config /path/to/iris_fullroster_pipeline.json \
  --output /path/to/full_roster_transfers

supported_ap_code/target/release/iris_fullroster_pipeline build-fixed-bundles \
  --config /path/to/iris_fullroster_pipeline.json \
  --transfers /path/to/full_roster_transfers \
  --output /path/to/full_roster_fixed_l2
```

### 11.4 Run adaptive selection and build combined bundles

The dense production selection should use the restartable Slurm workflow. Begin with a measured
pilot, then submit the complete plan using the same run root:

```bash
bash IRIS_scripts/adaptive_selection/submit_adaptive_hillq_selection_slurm.sh \
  --mode pilot \
  --source-root /path/to/full_roster_transfers \
  --bundle-root /path/to/full_roster_fixed_l2 \
  --run-root /path/to/adaptive_hillq_selection_run \
  --alpha-values "<preregistered orders>"

bash IRIS_scripts/adaptive_selection/submit_adaptive_hillq_selection_slurm.sh \
  --mode full \
  --source-root /path/to/full_roster_transfers \
  --bundle-root /path/to/full_roster_fixed_l2 \
  --run-root /path/to/adaptive_hillq_selection_run \
  --alpha-values "<same preregistered orders>"

supported_ap_code/target/release/iris_fullroster_pipeline build-combined-bundles \
  --base-bundles /path/to/full_roster_fixed_l2 \
  --selection /path/to/adaptive_hillq_selection_run/full/selection \
  --output /path/to/full_roster_70_systems
```

### 11.5 Derive policy-specific views after a complete tournament

```bash
supported_ap_code/target/release/directed_round_robin_organizer induce-view \
  --bundle <70-system-bundle> \
  --plan <complete-plan> \
  --reduction <complete-70-system-reduction> \
  --exclude-system <policy-not-in-this-view-1> \
                   <policy-not-in-this-view-2> \
                   <policy-not-in-this-view-3> \
                   <policy-not-in-this-view-4> \
                   <policy-not-in-this-view-5> \
  --output <65-system-induced-view>
```

The PDAC-only view excludes the five `*__self_gated_power_hillq_all_contexts_selected` systems.
The all-context view excludes the five `*__self_gated_power_hillq_pdac_selected` systems.

---

## 12. Acceptance checks for the regenerated production run

### Input package

- Source hashes and expected cohort counts match the frozen configuration.
- Every retained 9--12-mer/HLA mapping row has populated HLA and status `scoreable`.
- Canonical, full, and mono mappings pass the Rust validator.
- Query artifacts match the complete-F tensor roster exactly.

### Transfers

- All 30 cohort/model/metric directories exist and point to mappings in the new package.
- Every summary records \(10^{-12}\) and the new input-manifest hash.
- Every candidate resolves uniquely to a completed tensor score.
- Endpoint counts and positive/negative counts are unchanged.
- Full-roster candidate counts and genuine zero-signal prevalence match the input audits.
- Changes in fixed `max` and every coverage-sensitive operator are quantified.
- Mean, median, and top-fraction changes are expected and quantified.

### Adaptive selection

- Independent `max` and `logsumexp` reconstruction checks pass.
- Both selection policies contain one `(alpha,q,c,kappa)` tuple per model and metric.
- No selected `c` or `kappa` lies on a search-grid boundary.
- Selected endpoint scores use the exact frozen tuple and complete-F contract.
- Report hashes are frozen before bundle construction.

### Bundles and tournament

- The base bundle contains exactly 60 fixed systems.
- The combined bundle contains exactly 70 systems: 60 fixed, five PDAC-selected, and five
  all-context-selected.
- Each metric plans exactly 7,245 pair-cohort matches.
- The complete reduction uses every planned atomic verdict exactly once.
- Each 65-system induced reduction contains the correct node set and recomputes graph
  maximality from its induced edges.

### Interpretation

Changes relative to omission-floor results must be attributed separately to:

1. completion of the formerly omitted Level-1 computations;
2. exclusion of zero-signal candidates from adaptive Hill mass;
3. refitting `(alpha,q,c,kappa)` under worst-cohort-first selection;
4. the declared adaptive selection policy.

The omission-floor run is diagnostic only and is not a gate on the complete-F
production result.

---

## 13. Locked decisions

- The candidate universe is the endpoint-specific set of fully computed 9--12-mer/HLA tuples.
- The numeric zero-signal floor is \(\log(10^{-12})\); missing computations have no score.
- Omission-floor dominance of mean and median is an artifact, not an expected production result.
- Exact duplicate biological candidates are removed before aggregation.
- “L2” is the production terminology; “L3” is deprecated.
- PDAC-only and worst-cohort-first all-context adaptive selections are both retained.
- Both adaptive families enter one 70-system tournament.
- The two policy-specific 65-system results are induced reductions of the common tournament.
- Legacy parity and backward-compatible production interfaces are out of scope.
- New production artifacts are immutable and must not overwrite the audited historical runs.
