# Complete-F input rosters and PDAC cohort views: implementation plan

## Purpose

This document specifies the code and data changes needed to regenerate complete
Level-1 \(F\) tensors for the COVID spike, COVID non-spike, and PDAC cohorts
without using Kd, antigen-processing, or stability filters to decide which
candidate computations exist.

It also specifies how to preserve downstream analysis on the historical
Rojas/Sethna PDAC mapping. The central design is:

> Compute one complete PDAC tensor, then evaluate both the full PDAC cohort and
> the Rojas/Sethna subset as named mappings over that same tensor.

The Rojas/Sethna mapping is therefore retained as an analysis view, not as a
second tensor lineage and not as a filter on the complete computation roster.

This plan is intentionally separate from
`COMPLETE_F_FULL_ROSTER_REGENERATION_PLAN.md`. That document remains the
end-to-end production replacement runbook. This document focuses on the input
construction, Stage-2 query contract, cohort-view model, and the code changes
needed to support them.

## Executive decision

The implementation should create three primary cohort mappings:

```text
covid_nonspike
covid_spike
pdac_full
```

and one explicitly nested secondary mapping:

```text
pdac_rojas_sethna  subset of  pdac_full
```

Only the primary mappings define the complete Stage-2 compute rosters. The
secondary Rojas/Sethna mapping must resolve entirely through the already
computed PDAC full roster.

The default modeling roles are:

| View | Role | Used for joint parameter selection? | Report downstream metrics? |
|---|---|---:|---:|
| `covid_nonspike` | primary | yes | yes |
| `covid_spike` | primary | yes | yes |
| `pdac_full` | primary | yes | yes |
| `pdac_rojas_sethna` | secondary, nested | no | yes |

This prevents the eight-patient Rojas/Sethna subset from being double-counted
as an independent fourth cohort while preserving its PR/CNAP, AUROC,
per-endpoint predictions, selected-tau diagnostics, and fixed/adaptive L2
comparisons.

If the Rojas/Sethna view is ever to influence selection or tournament voting,
that must be a separately preregistered policy change. It must not happen as a
side effect of adding view support.

## Motivation: omission is not a biological floor

The query CSVs currently named by the Stage-2 TOMLs were produced after
presentation and stability filtering. A peptide/HLA tuple absent from those
files can mean any of the following:

- its Kd was above a threshold;
- its antigen-processing score was below a threshold;
- its stability was below a threshold;
- a required lookup result was missing;
- an earlier script never enumerated it;
- it was excluded through a mapping grounded in a filtered query.

None of these is equivalent to a completed Level-1 calculation returning a
biological zero. Treating an uncomputed tuple as `log(1e-12)` therefore lets an
old preprocessing decision masquerade as evidence. It also makes mean-like L2
operators respond to how exhaustively the upstream pipeline happened to score
candidates.

The replacement must make these states distinct:

```text
candidate declared and computed, F > 0       -> ordinary finite evidence
candidate declared and computed, F = 0       -> biological zero; floor on log scale
candidate declared but computation missing   -> hard error
candidate not in an analysis view             -> not part of that view; not a floor
```

## The four identities that must not be conflated

### 1. Assayed endpoint identity

An endpoint is the experimental observation that carries a response label.

- COVID non-spike: `(patient_id, long_peptide)`
- COVID spike: `(patient_id, mutation, long_peptide)`
- PDAC: `(patient_id, long_peptide)`

Condition and other provenance columns remain available, but the chosen
identity must match the downstream label contract and must be validated for
conflicting labels.

### 2. Endpoint-specific candidate identity

This is the biological contribution used by L2 aggregation:

```text
(endpoint identity, minimal epitope, normalized restriction HLA)
```

It is a set, not a multiset. Repeated source rows, homozygous copies, or
overlapping generation paths must not multiply the same contribution.

### 3. Stage-2 computational identity

The existing Stage-2 preflight correctly defines the reusable computation as:

```text
(minimal epitope, normalized restriction HLA, env_id, TCGA_EXPR_TYPE)
```

Two endpoints can legitimately reference the same Stage-2 computation. The
query roster should contain that key once, while a crosswalk maps its result
back to every endpoint-specific candidate that uses it.

Patient, gene, mutation, long-peptide, and response columns are provenance;
they must not force duplicate computation.

### 4. Analysis-view identity

An analysis view is a named set of endpoint-specific candidates. It answers:
"Which endpoints and candidates are aggregated for this reported metric?"

Changing a view must change the mapping and metric hashes. It must not change
the underlying complete query or tensor hash when the view remains a subset of
the complete compute universe.

## Evidence from the current files

### Current filtered query rosters

| Cohort | Current rows | Unique Stage-2 keys | Duplicate excess |
|---|---:|---:|---:|
| COVID non-spike | 4,242 | 4,242 | 0 |
| COVID spike | 2,866 | 2,866 | 0 |
| PDAC | 4,300 | 4,182 | 118 |

The PDAC input has 106 duplicated compute keys and fails
`stage2_run_contract.py validate-representation`.

The relevant generators are:

```text
/Users/thomm15/Work_Data/Cansu_Covid_Nonspike/Marcus_preprocessing/
make_cansu_covid_neoepitope_table_deprecated.py

/Users/thomm15/Work_Data/Cansu_Covid_Spike/Marcus_preprocessing/
make_cansu_covid_neoepitope_table_deprecated.py

/Users/thomm15/Work_Data/Jayon/Marcus_pre_processing/
make_neoepitope_table_with_Kd_Ap_Stability_filters_deprecated.py
```

The two COVID scripts now deduplicate their selected rosters, but selection is
still controlled by Kd/AP/stability. The PDAC script both filters and emits
duplicates.

### Target primary and secondary sizes

Read-only reconstruction from the measured endpoint sources produced:

| Cohort/view | Endpoint+n-mer rows | Endpoint+n-mer+HLA tuples | Unique full-env Stage-2 keys |
|---|---:|---:|---:|
| COVID non-spike primary | 35,446 | 199,124 | 167,864 |
| COVID spike primary | 25,740 | 137,175 | 104,815 |
| PDAC full primary | 9,200 | 52,458 | 51,756 |
| PDAC Rojas/Sethna secondary | 231 | 1,386 | 1,368 |

These are initial acceptance fixtures. A code implementation must reproduce
them or stop with an explained source-contract discrepancy.

### The PDAC naming trap

The existing file:

```text
/Users/thomm15/Work_Data/IRIS_scripts/
Run_Scripts_Selected_ParamSets_for_PDAC_COVID_Transfer_deprecated/
pdac_full_mapping_noResponderFilter_noKdFilter_9to12mer.csv
```

contains 1,573 endpoint+n-mer rows across 213 endpoints. Despite its name, it
is not fully ungated. Its generator disables the explicit Kd gate but later
intersects every candidate with the Kd/AP/stability-filtered evaluation CSV.
Only 17.1% of the 9,200 candidates generated by the declared full-cohort rules
survive that intersection.

The historical narrow Rojas/Sethna file is:

```text
/Users/thomm15/Work_Data/Jayon/Marcus_pre_processing/PDAC_eval_mapping/
longpep_nmer_mapping_RojasSethna_grounded.csv
```

It contains 231 endpoint+n-mer rows, 76 endpoints, and 8 patients. Its current
SHA-256 is:

```text
01c05820a9349399ae710119ae1e4c994138ca13d476e78b256bb3f44702a774
```

Its associated decision file currently hashes to:

```text
bf2807fec0ff6487bf99bfc0ab5eff279a94890b8c2637d7fcdc157bfbd13156
```

These hashes should be recorded as source provenance. They are not eternal
expected values: if an intentional source correction occurs, the new hash and
reason must be reviewed and frozen in a new immutable run configuration.

## Target architecture

```text
measured endpoint sources + HLA environment dictionaries
                         |
                         v
             normalized endpoint mappings
                         |
              +----------+--------------------+
              |                               |
              v                               v
       primary full mappings          named secondary views
              |                       (PDAC Rojas/Sethna)
              v                               |
 endpoint+nmer+distinct-HLA set                |
              |                               |
              +---------- subset audit -------+
              |
              v
   unique Stage-2 computational roster
              |
              v
       complete Level-1 tensors
              |
              v
 computation-to-endpoint crosswalk
              |
        +-----+-------------------+
        |                         |
        v                         v
 full-cohort aggregation    Rojas/Sethna aggregation
        |                         |
        v                         v
 primary metrics/selection  secondary comparison metrics
```

## Required output package

The complete input builder should emit:

```text
full_roster_inputs/
├── manifest.json
├── covid_nonspike_full_deduplicated_query.csv
├── covid_nonspike_primary_mapping.csv
├── covid_nonspike_compute_endpoint_crosswalk.csv
├── covid_nonspike_inputs_audit.json
├── covid_spike_full_deduplicated_query.csv
├── covid_spike_primary_mapping.csv
├── covid_spike_compute_endpoint_crosswalk.csv
├── covid_spike_inputs_audit.json
├── pdac_full_deduplicated_query.csv
├── pdac_full_primary_mapping.csv
├── pdac_rojas_sethna_mapping.csv
├── pdac_compute_endpoint_crosswalk.csv
├── pdac_view_membership.csv
├── pdac_inputs_audit.json
├── *_mono_query.csv
├── *_mono_hla_env_dict.csv
├── *_full_to_mono_env_crosswalk.csv
├── binding_sim_updated_*_Full_Deduplicated.toml
└── binding_sim_updated_*_Mono.toml
```

The exact names may follow current crate conventions, but the semantic roles
must be explicit in the manifest.

### Query schema

The Stage-2 query should retain the established columns:

```text
peptide,HLA-RE,SetNeoepitopeSampleID,PatientID,
TCGA_EXPR_TYPE,env_id,gene,count
```

The first, second, fifth, and sixth columns define computation. The remaining
columns are deterministic representative metadata only. The complete
computation-to-endpoint relationship belongs in a separate crosswalk, not in
duplicated query rows.

### Mapping schema

Each primary or secondary mapping needs at least:

```text
view_id
view_role
patient_id
endpoint identity fields
nmer
HLA-RE
full_env_id
representation env_id
response value/label
gene
mapping_status
compute_key_id
```

For production complete-F inputs, every mapping row must have
`mapping_status=scoreable`. Missing computation is represented by validation
failure, never by a `floor` status.

### View manifest schema

For every view record:

```text
view_id
parent_view_id or null
role = primary|secondary
source path and SHA-256
endpoint identity fields
endpoint count
endpoint+nmer count
endpoint+nmer+HLA count
unique compute-key count
label counts/range
subset audit result
selection_eligible
bundle_eligible
```

## Detailed implementation changes

### Change 1 — Add a deterministic PDAC endpoint-view generator

Add:

```text
/Users/thomm15/Work_Data/Jayon/Marcus_pre_processing/build_complete_f_endpoint_views.py
```

This should replace the current full-cohort mapping-generation path; it is not
a compatibility wrapper around the filtered CSV.

Inputs:

- clinical workbook:
  `Jayon/PDAC_vax_clinical_trial/20230224 Neoantigen target list ML41081 with ELISpot.xlsx`;
- patient HLA typing: `Jayon/PDAC_vax_clinical_trial/HLA_typing.txt`;
- full environment dictionary: `Jayon/hla_env_dict.csv`;
- historical Rojas/Sethna mapping and decision log for the secondary view.

Full-cohort construction rules:

1. Normalize patient IDs, response strings, genes, and peptide sequences.
2. Require patient ID, WT peptide, and mutant peptide.
3. Map `De novo response` to 1 and `No response` to 0.
4. Drop `No data` and `De novo response in pool`.
5. Do not apply responder-only, Kd, AP, or stability filtering.
6. Generate 9--12-mers using the existing mutation-aware rules:
   - one difference: retain windows spanning that position;
   - multiple differences: retain windows spanning at least one difference;
   - unequal WT/MUT lengths: retain all contiguous mutant windows;
   - identical WT/MUT sequences: reject and record the reason.
7. Resolve `env_id` from HLA typing and the environment dictionary.
8. Deduplicate exact endpoint+n-mer identities after deterministic sorting.
9. Fail if the same endpoint has conflicting labels or incompatible metadata.
10. Emit a decision row for every clinical workbook row, including rejected
    rows.

Rojas/Sethna view rules:

- Preserve the historical mapping membership and labels after normalization.
- Preserve its decision file as provenance.
- Do not broaden or reinterpret its historical 9-mer/Kd/responder policy.
- Verify that every Rojas/Sethna endpoint+n-mer occurs in the new full mapping.
- Report any source row that cannot be resolved; do not silently drop it.

Proposed command after this script exists:

```bash
REPO_ROOT=/Users/thomm15/Documents/Supported_Model_Replacement
WORK_DATA_ROOT=/Users/thomm15/Work_Data
PDAC_VIEW_ROOT=${WORK_DATA_ROOT}/Jayon/complete_f_endpoint_views

cd "${REPO_ROOT}"

python3 /Users/thomm15/Work_Data/Jayon/Marcus_pre_processing/build_complete_f_endpoint_views.py \
  --clinical-workbook \
    "${WORK_DATA_ROOT}/Jayon/PDAC_vax_clinical_trial/20230224 Neoantigen target list ML41081 with ELISpot.xlsx" \
  --hla-typing \
    "${WORK_DATA_ROOT}/Jayon/PDAC_vax_clinical_trial/HLA_typing.txt" \
  --env-dict "${WORK_DATA_ROOT}/Jayon/hla_env_dict.csv" \
  --rojas-sethna-mapping \
    "${WORK_DATA_ROOT}/Jayon/Marcus_pre_processing/PDAC_eval_mapping/longpep_nmer_mapping_RojasSethna_grounded.csv" \
  --rojas-sethna-decisions \
    "${WORK_DATA_ROOT}/Jayon/Marcus_pre_processing/PDAC_eval_mapping/longpep_nmer_mapping_RojasSethna_grounded_decisions.csv" \
  --output "${PDAC_VIEW_ROOT}"
```

The command must refuse to overwrite a nonempty output directory.

Tests to add:

```text
/Users/thomm15/Work_Data/Jayon/Marcus_pre_processing/tests/test_build_complete_f_endpoint_views.py
```

Minimum fixtures:

- canonical single substitution;
- multiple substitutions;
- unequal-length WT/MUT peptides;
- identical WT/MUT rejection;
- `No data` and pooled-response removal;
- a non-responder patient retained;
- duplicate clinical rows collapsed;
- conflicting labels rejected;
- HLA profile with a homozygous duplicate;
- Rojas/Sethna row missing from full mapping rejected.

### Change 2 — Make `external_validation_inputs` construct complete queries

Update:

```text
supported_ap_code/crates/external_validation_inputs/src/builder.rs
supported_ap_code/crates/external_validation_inputs/src/config.rs
supported_ap_code/crates/external_validation_inputs/tests/end_to_end.rs
supported_ap_code/crates/external_validation_inputs/README.md
IRIS_scripts/configs/input_packaging/external_validation_inputs.example.json
```

Bundle validation currently lives in `builder.rs`; keep it there unless the
implementation creates a substantive reason to extract a module.

Required configuration changes:

- remove the filtered query as the production membership authority;
- add an explicit dataset-level expression value;
- rename the input mapping concept to `primary_mapping`;
- add a list of named secondary views;
- add `role`, `parent_view_id`, `selection_eligible`, and `bundle_eligible`;
- record expected counts separately for primary mappings, compute rosters, and
  secondary views.

Conceptual PDAC configuration:

```json
{
  "id": "PDAC",
  "prefix": "pdac",
  "expression": "pancreatic adenocarcinoma",
  "env_dict": "Jayon/hla_env_dict.csv",
  "primary_mapping": "/Users/thomm15/Work_Data/Jayon/complete_f_endpoint_views/pdac_full_endpoint_mapping_9to12mer.csv",
  "response_column": "long_peptide_label",
  "response_kind": "binary",
  "endpoint_fields": ["patient_id", "long_peptide"],
  "views": [
    {
      "id": "pdac_rojas_sethna",
      "role": "secondary",
      "mapping": "/Users/thomm15/Work_Data/Jayon/complete_f_endpoint_views/pdac_rojas_sethna_endpoint_mapping.csv",
      "require_subset_of": "pdac_full",
      "selection_eligible": false,
      "bundle_eligible": false
    }
  ]
}
```

Required construction algorithm:

1. Load and normalize the primary endpoint mapping.
2. Load each full environment as an ordered set of distinct normalized HLAs.
3. Expand each endpoint+n-mer over that distinct HLA set.
4. Deduplicate the endpoint-specific candidate identity.
5. Project candidates to the Stage-2 compute key and write one query row per
   key.
6. Create a stable `compute_key_id` and crosswalk every endpoint candidate to
   it.
7. Build mono-environment queries and crosswalks from the same canonical
   candidate set.
8. Materialize secondary view mappings through the canonical crosswalk.
9. Require every secondary compute key to exist in its parent primary query.
10. Emit only `mapping_status=scoreable`; remove production omission-floor
    branches.

Deduplication must occur at two different levels:

```text
endpoint mapping:
  unique(endpoint identity, nmer, normalized HLA)

Stage-2 query:
  unique(nmer, normalized HLA, env_id, expression)
```

Do not deduplicate endpoint candidates on the compute key; doing so would erase
the legitimate fact that several endpoints can use the same calculation.

Required Rust tests:

- a shared compute key appears once in the query but maps to two endpoints;
- a homozygous allele appears once per endpoint and once in the query;
- two different HLA restrictions for the same peptide remain distinct;
- two different environments remain distinct;
- a secondary view is a strict subset and does not change the query hash;
- an out-of-universe secondary row fails validation;
- a primary mapping with a conflicting endpoint label fails;
- any `mapping_status=floor` production row fails;
- deterministic builds from identical inputs have identical hashes.

### Change 3 — Generate Stage-2 TOMLs from the complete query package

The existing dataset TOMLs point to the filtered CSVs:

```text
/Users/thomm15/Work_Data/New_Approaches/MHC_competition_solver/src/
binding_sim_updated_COVID_Nonspike.toml

/Users/thomm15/Work_Data/New_Approaches/MHC_competition_solver/src/
binding_sim_updated_COVID_Spike.toml

/Users/thomm15/Work_Data/New_Approaches/MHC_competition_solver/src/
binding_sim_updated_PDAC.toml
```

Do not overwrite these historical inputs during development. The input builder
should generate immutable run-specific full and mono TOMLs whose
`query_peptide_input_tuples_file` points at the complete deduplicated query.

The TOML audit must verify:

- the resolved query path is inside the new input bundle;
- the query hash matches the bundle manifest;
- the environment dictionary and representation match the query;
- the frozen Level-1 parameter, decision, and exposure settings are unchanged;
- no Kd/AP/stability threshold is being used to remove query rows.

### Change 4 — Keep and strengthen the Stage-2 preflight

The existing preflight in:

```text
/Users/thomm15/Work_Data/IRIS_scripts/stage2_run_contract.py
```

already enforces uniqueness on:

```text
(peptide, normalized HLA-RE, env_id, expression)
```

Keep that contract. Add or extend audit output so that a dry run reports:

- total query rows;
- unique compute keys;
- active environment IDs;
- peptide lengths and counts;
- distinct HLA count;
- query SHA-256;
- expected count from the complete-input manifest;
- exact manifest match status.

The preflight should reject a query that is internally unique but does not
match the manifest's expected compute-key set.

The repository-side bridge is now implemented as:

```text
/Users/thomm15/Work_Data/IRIS_scripts/validate_complete_f_stage2_inputs.py
```

Run it before the existing cluster preflight. It binds the generated TOML and
query to the complete-input manifest, verifies the recorded hash and row count,
and reports active environments, HLA count, and the 9--12-mer length
distribution. The existing `stage2_run_contract.py` remains authoritative for
Runner-specific representation and task-output validation.

### Change 5 — Add named cohort views to `iris_fullroster_pipeline`

Update the typed configuration and transfer path in:

```text
supported_ap_code/crates/iris_fullroster_pipeline/src/contract.rs
supported_ap_code/crates/iris_fullroster_pipeline/src/transfer.rs
supported_ap_code/crates/iris_fullroster_pipeline/src/bundle.rs
supported_ap_code/crates/iris_fullroster_pipeline/tests/compact_pipeline.rs
supported_ap_code/crates/iris_fullroster_pipeline/README.md
```

Required behavior:

1. Load primary and secondary mappings from the validated input-package
   manifest rather than accepting arbitrary untracked mapping paths.
   The typed cohort config must name `primary_view_id`; PDAC uses
   `primary_view_id=pdac_full` even though its stable selector directory remains
   `pdac/`.
2. Resolve every mapping row uniquely to a completed tensor observation and
   every selected tau.
3. Aggregate endpoint predictions independently for each `view_id`.
4. Write the same family of downstream diagnostics for primary and secondary
   views.
5. Keep selection eligibility and bundle eligibility explicit.
6. By default, build the official fixed/adaptive bundles from the three primary
   views only.
7. Emit Rojas/Sethna metrics under a separate secondary-view output tree.
8. Never treat the nested Rojas/Sethna view as an independent worst-cohort
   constraint unless the configuration explicitly changes its role.

Suggested output structure:

```text
full_roster_transfers/
├── primary/
│   ├── covid_nonspike/
│   ├── covid_spike/
│   └── pdac_full/
└── secondary/
    └── pdac_rojas_sethna/
```

For every component model and metric branch, the Rojas/Sethna directory should
contain at least:

```text
summary.json
transfer_metrics.csv
long_peptide_predictions.csv
target_tau_selection_by_observation.csv
view_manifest.json
```

Where a component uses a separate Q source, retain the corresponding joined-Q
diagnostic already used by the full-roster pipeline.

The secondary `view_manifest.json` must record the parent PDAC tensor hashes,
the Rojas/Sethna mapping hash, endpoint count, candidate count, completed-zero
count, and zero missing computations.

### Change 6 — Keep adaptive selection and tournament weighting explicit

The selector should consume only views with `selection_eligible=true`. The
initial complete-F production configuration should set this flag only for:

```text
covid_nonspike
covid_spike
pdac_full
```

The Rojas/Sethna view should still receive predictions for every fixed and
adaptive system after parameters are selected on the primary cohorts. This
permits direct comparison without letting the nested subset tune its own rule.

Likewise, the official combined 70-system tournament bundle should contain the
three primary evaluation cohorts unless an explicit secondary diagnostic bundle
is requested. If a diagnostic Rojas/Sethna bundle is built, name it clearly and
do not merge its verdicts into the primary three-cohort reduction.

### Change 7 — Documentation and stale-semantics audit

Update after code behavior is implemented and tested:

```text
IRIS_scripts/configs/input_packaging/external_validation_inputs.example.json
IRIS_scripts/README.md
IRIS_scripts/docs/COMPLETE_F_CLUSTER_RUNBOOK.md
IRIS_scripts/iris_fullroster_round_robin_plan.md
supported_ap_code/crates/external_validation_inputs/README.md
supported_ap_code/crates/iris_fullroster_pipeline/README.md
supported_ap_code/crates/README.md
```

Pedagogically distinguish:

- computed zero from missing computation;
- endpoint tuple from compute key;
- primary cohort from nested analysis view;
- historical Rojas/Sethna membership from full PDAC membership;
- complete query construction from downstream view aggregation.

Audit stale production language with:

```bash
rg -n \
  'mapping_status.*floor|floor rows|omission-floor|filtered.*query|noKdFilter|RojasSethna' \
  IRIS_scripts supported_ap_code/crates \
  --glob '!supported_ap_code/target/**'
```

## Implementation sequence and commands

Commands labeled **after implementation** depend on code that this plan asks
to add. They are specifications for the completed workflow, not commands that
work in the current checkout.

### Phase 0 — Freeze source provenance

```bash
REPO_ROOT=/Users/thomm15/Documents/Supported_Model_Replacement
WORK_DATA_ROOT=/Users/thomm15/Work_Data
RUN_ID=complete_f_YYYYMMDD
STAGE_ROOT=/path/to/immutable_runs/${RUN_ID}

mkdir -p "${STAGE_ROOT}/source_audit"

shasum -a 256 \
  "${WORK_DATA_ROOT}/Cansu_Covid_Nonspike/Marcus_preprocessing/atlas_patient_peptide_CD8_response_mapping.csv" \
  "${WORK_DATA_ROOT}/Cansu_Covid_Nonspike/Marcus_preprocessing/atlas_hla_env_dict.csv" \
  "${WORK_DATA_ROOT}/Cansu_Covid_Spike/Marcus_preprocessing/spike_patient_peptide_CD8_response_mapping.csv" \
  "${WORK_DATA_ROOT}/Cansu_Covid_Spike/Marcus_preprocessing/covid_spike_hla_env_dict.csv" \
  "${WORK_DATA_ROOT}/Jayon/PDAC_vax_clinical_trial/20230224 Neoantigen target list ML41081 with ELISpot.xlsx" \
  "${WORK_DATA_ROOT}/Jayon/PDAC_vax_clinical_trial/HLA_typing.txt" \
  "${WORK_DATA_ROOT}/Jayon/hla_env_dict.csv" \
  "${WORK_DATA_ROOT}/Jayon/Marcus_pre_processing/PDAC_eval_mapping/longpep_nmer_mapping_RojasSethna_grounded.csv" \
  "${WORK_DATA_ROOT}/Jayon/Marcus_pre_processing/PDAC_eval_mapping/longpep_nmer_mapping_RojasSethna_grounded_decisions.csv" \
  > "${STAGE_ROOT}/source_audit/input_sha256.txt"
```

Gate: all source files exist and hashes are frozen before generator changes.

### Phase 1 — Implement and test the PDAC view generator

After implementation:

```bash
cd "${REPO_ROOT}"

python3 -m unittest discover \
  -s "${WORK_DATA_ROOT}/Jayon/Marcus_pre_processing/tests" \
  -p 'test_build_complete_f_endpoint_views.py'

python3 /Users/thomm15/Work_Data/Jayon/Marcus_pre_processing/build_complete_f_endpoint_views.py \
  --clinical-workbook \
    "${WORK_DATA_ROOT}/Jayon/PDAC_vax_clinical_trial/20230224 Neoantigen target list ML41081 with ELISpot.xlsx" \
  --hla-typing \
    "${WORK_DATA_ROOT}/Jayon/PDAC_vax_clinical_trial/HLA_typing.txt" \
  --env-dict "${WORK_DATA_ROOT}/Jayon/hla_env_dict.csv" \
  --rojas-sethna-mapping \
    "${WORK_DATA_ROOT}/Jayon/Marcus_pre_processing/PDAC_eval_mapping/longpep_nmer_mapping_RojasSethna_grounded.csv" \
  --rojas-sethna-decisions \
    "${WORK_DATA_ROOT}/Jayon/Marcus_pre_processing/PDAC_eval_mapping/longpep_nmer_mapping_RojasSethna_grounded_decisions.csv" \
  --output "${WORK_DATA_ROOT}/Jayon/complete_f_endpoint_views" \
  --expected-full-endpoints 221 \
  --expected-full-rows 9200 \
  --expected-rojas-endpoints 76 \
  --expected-rojas-rows 231
```

Gate:

- full PDAC has 221 endpoints and 9,200 endpoint+n-mer rows;
- Rojas/Sethna has 76 endpoints and 231 endpoint+n-mer rows;
- labels have no conflicts;
- Rojas/Sethna membership is a subset of full PDAC membership;
- no Kd/AP/stability input is read while constructing `pdac_full`.

### Phase 2 — Implement and test complete query construction

```bash
cd "${REPO_ROOT}"

cargo fmt --manifest-path supported_ap_code/Cargo.toml --all -- --check

cargo test --manifest-path supported_ap_code/Cargo.toml \
  --package external_validation_inputs

cargo clippy --manifest-path supported_ap_code/Cargo.toml \
  --package external_validation_inputs --all-targets -- -D warnings

cargo build --release --manifest-path supported_ap_code/Cargo.toml \
  --package external_validation_inputs
```

Gate: the crate has no production floor branch, view-subset tests pass, and two
identical builds produce identical manifest and output hashes.

### Phase 3 — Build and validate the complete input package

Create a new run-specific configuration from the example. Do not update the
historical configuration in place.

```bash
INPUT_CONFIG=${STAGE_ROOT}/external_validation_inputs.complete_f.json
INPUT_BUNDLE=${WORK_DATA_ROOT}/IRIS_scripts/complete_f_input_package

cp IRIS_scripts/configs/input_packaging/external_validation_inputs.example.json "${INPUT_CONFIG}"
```

Edit the copied configuration to use the two generated PDAC view mappings and
the two authoritative COVID endpoint mappings. Then, after implementation:

```bash
supported_ap_code/target/release/external_validation_inputs build \
  --config "${INPUT_CONFIG}" \
  --input-root "${WORK_DATA_ROOT}" \
  --output "${INPUT_BUNDLE}"

supported_ap_code/target/release/external_validation_inputs validate \
  --bundle "${INPUT_BUNDLE}"

rg -n 'floor|missing|duplicate|view_id|subset|query_rows' \
  "${INPUT_BUNDLE}/manifest.json" \
  "${INPUT_BUNDLE}"/*_inputs_audit.json
```

Gate:

- primary compute-key counts are 167,864, 104,815, and 51,756;
- Rojas/Sethna resolves to 1,368 PDAC compute keys;
- every secondary key is in the full PDAC query;
- exact duplicate counts are reported but no duplicate survives;
- all mapping rows are scoreable;
- no missing computation is encoded as a floor.

### Phase 4 — Validate Stage-2 representation before upload

First bind each generated configuration to the complete-input manifest:

```bash
for DATASET in PDAC COVID_SPIKE COVID_NONSPIKE; do
  python3 "${WORK_DATA_ROOT}/IRIS_scripts/validate_complete_f_stage2_inputs.py" \
    validate \
    --bundle "${INPUT_BUNDLE}" \
    --dataset "${DATASET}" \
    --representation full
  python3 "${WORK_DATA_ROOT}/IRIS_scripts/validate_complete_f_stage2_inputs.py" \
    validate \
    --bundle "${INPUT_BUNDLE}" \
    --dataset "${DATASET}" \
    --representation mono
done
```

Install the validated package directly into the three established cohort
roots. These stable paths deliberately have no run-ID directory:

```bash
for DATASET in PDAC COVID_SPIKE COVID_NONSPIKE; do
  python3 "${WORK_DATA_ROOT}/IRIS_scripts/validate_complete_f_stage2_inputs.py" \
    install \
    --bundle "${INPUT_BUNDLE}" \
    --dataset "${DATASET}" \
    --work-data-root "${WORK_DATA_ROOT}"
done
```

The installer refuses to overwrite an existing complete-F target. An earlier
complete-F installation must first be deliberately archived with `_deprecated`
or removed after its provenance has been preserved.

Then run the cluster Runner-specific validation described below.

For each generated full query:

```bash
python3 "${WORK_DATA_ROOT}/IRIS_scripts/stage2_run_contract.py" \
  validate-representation \
  --query "${INPUT_BUNDLE}/covid_nonspike_full_deduplicated_query.csv" \
  --env-dict \
    "${WORK_DATA_ROOT}/Cansu_Covid_Nonspike/Marcus_preprocessing/atlas_hla_env_dict.csv" \
  --hla-environment-representation full \
  --total-env-ids 16

python3 "${WORK_DATA_ROOT}/IRIS_scripts/stage2_run_contract.py" \
  validate-representation \
  --query "${INPUT_BUNDLE}/covid_spike_full_deduplicated_query.csv" \
  --env-dict \
    "${WORK_DATA_ROOT}/Cansu_Covid_Spike/Marcus_preprocessing/covid_spike_hla_env_dict.csv" \
  --hla-environment-representation full \
  --total-env-ids 13

python3 "${WORK_DATA_ROOT}/IRIS_scripts/stage2_run_contract.py" \
  validate-representation \
  --query "${INPUT_BUNDLE}/pdac_full_deduplicated_query.csv" \
  --env-dict "${WORK_DATA_ROOT}/Jayon/hla_env_dict.csv" \
  --hla-environment-representation full \
  --total-env-ids 22
```

Gate: all three queries pass the existing duplicate and representation
contract and match their package manifest exactly.

### Phase 5 — Upload and dry-run complete Stage-2 jobs

Mirror the stable cohort-local inputs into the corresponding established cohort
roots on IRIS. Do not create a run-ID input directory and do not copy the old
filtered query under a new filename.

```bash
CLUSTER_HOST=replace_with_iris_ssh_host
IRIS_WORK_DATA_ROOT=/data1/lukszam/Marcus

for SPEC in \
  "Jayon:pdac" \
  "Cansu_Covid_Spike:covid_spike" \
  "Cansu_Covid_Nonspike:covid_nonspike"; do
  COHORT=${SPEC%%:*}
  PREFIX=${SPEC#*:}
  MANIFEST="${WORK_DATA_ROOT}/${COHORT}/${PREFIX}_complete_f_inputs_manifest.json"
  FILE_LIST="/tmp/${PREFIX}_complete_f_files.txt"
  jq -r '.files | keys[]' "${MANIFEST}" > "${FILE_LIST}"
  rsync -av --files-from="${FILE_LIST}" \
    "${WORK_DATA_ROOT}/${COHORT}/" \
    "${CLUSTER_HOST}:${IRIS_WORK_DATA_ROOT}/${COHORT}/"
  rsync -av "${MANIFEST}" \
    "${CLUSTER_HOST}:${IRIS_WORK_DATA_ROOT}/${COHORT}/"
done
```

On IRIS, dry-run both Stage-2 modes for every primary cohort. Use the generated
TOML for the appropriate full or mono representation. A full command template
is:

```bash
cd /data1/lukszam/Marcus/IRIS_scripts

bash complete_f_stage2/submit_complete_f_stage2.sh \
  --dataset PDAC \
  --run-id "${RUN_ID}" \
  --q-model-config \
    "${IRIS_WORK_DATA_ROOT}/Jayon/binding_sim_updated_PDAC_Full_Deduplicated.toml" \
  --external-validation-input-root "${IRIS_WORK_DATA_ROOT}/Jayon" \
  --hla-environment-representation full \
  --pn-hla-scope all \
  --dry-run
```

Repeat for `COVID_SPIKE` and `COVID_NONSPIKE`, then repeat for every required
mono representation using the generated mono TOML, environment dictionary,
crosswalk, and explicit mono environment count. Do not infer those counts from
the historical package.

Gate: dry runs resolve only new complete queries, display the expected query
hashes and row counts, and submit nothing.

### Phase 6 — Run and validate the complete tensors

After dry-run approval, remove `--dry-run` and submit PN and QPI jobs with the
frozen Level-1 geometry, decision regime, exposure grid, parameter file, and
M/N tuples. Do not reselect Level 1.

For each completed representation, require:

- every expected query key occurs exactly once for every required tau/parameter
  context;
- no undeclared key occurs;
- completed zero values are finite and explicitly counted;
- missing keys are zero;
- the tensor manifest includes query and executable hashes.

The Rojas/Sethna mapping is not submitted separately. Its compute keys are
already covered by the full PDAC job.

### Phase 7 — Implement and test primary/secondary transfer views

```bash
cd "${REPO_ROOT}"

cargo fmt --manifest-path supported_ap_code/Cargo.toml --all -- --check

cargo test --manifest-path supported_ap_code/Cargo.toml \
  --package iris_fullroster_pipeline \
  --package select_adaptive_hillq

cargo clippy --manifest-path supported_ap_code/Cargo.toml \
  --package iris_fullroster_pipeline \
  --package select_adaptive_hillq \
  --all-targets -- -D warnings
```

Gate:

- the same PDAC tensor fixture produces both full and Rojas/Sethna predictions;
- each view has its own endpoint and metric counts;
- Rojas/Sethna is absent from selector cohort weighting by default;
- changing only Rojas/Sethna membership cannot change the full PDAC predictions
  or selected parameters;
- a missing Rojas/Sethna tensor observation fails rather than flooring.

### Phase 8 — Generate full and Rojas/Sethna downstream metrics

After implementation, the normal transfer command should materialize both
roles from one validated configuration:

```bash
PIPELINE=supported_ap_code/target/release/iris_fullroster_pipeline
PIPELINE_CONFIG=${STAGE_ROOT}/iris_fullroster_pipeline.complete_f.json
TRANSFER_ROOT=${STAGE_ROOT}/full_roster_transfers

"${PIPELINE}" transfer-batch \
  --config "${PIPELINE_CONFIG}" \
  --output "${TRANSFER_ROOT}"

"${PIPELINE}" validate-transfers \
  --package "${TRANSFER_ROOT}"
```

The primary branch should still contain 30 transfer jobs:

```text
3 primary cohorts x 5 component models x 2 metric branches = 30
```

The secondary Rojas/Sethna branch should add 10 metric outputs:

```text
1 secondary view x 5 component models x 2 metric branches = 10
```

These ten outputs are comparison products, not extra optimization cohorts.

Required comparison report:

```text
${TRANSFER_ROOT}/secondary/pdac_rojas_sethna/comparison_to_pdac_full.csv
${TRANSFER_ROOT}/secondary/pdac_rojas_sethna/comparison_to_pdac_full.json
```

For each component and metric, report:

- endpoint and label counts;
- PR/CNAP or AUROC;
- fixed-L2 performance;
- adaptive-L2 performance using parameters selected on primary cohorts;
- delta from full PDAC;
- completed-zero prevalence;
- candidate-count distribution;
- tensor coverage, which must be 100%;
- mapping and tensor hashes.

### Phase 9 — Build primary production bundles and optional diagnostics

The official fixed and combined bundles should use only primary views:

```bash
"${PIPELINE}" build-fixed-bundles \
  --config "${PIPELINE_CONFIG}" \
  --transfers "${TRANSFER_ROOT}" \
  --output "${STAGE_ROOT}/full_roster_fixed_l2"
```

If a Rojas/Sethna comparison bundle is useful, add a clearly separate command
or configuration such as:

```text
build-view-bundle --view pdac_rojas_sethna
```

That diagnostic bundle must not be merged into the three-primary-cohort
tournament reduction and must carry `role=secondary` in its manifest.

### Phase 10 — Publish only after all gates pass

Do not remove the old artifact or cluster-input trees until:

- complete input validation passes;
- Stage-2 coverage is exact;
- primary and Rojas/Sethna transfer metrics both validate;
- adaptive selection uses only the declared primary views;
- the replacement primary tournament pilot passes;
- the Rojas/Sethna comparison report is reproducible from the same tensor
  hashes.

Then follow the retirement and publication steps in
`COMPLETE_F_FULL_ROSTER_REGENERATION_PLAN.md`.

## Acceptance checklist

### Input construction

- [ ] No primary query membership depends on Kd, AP, or stability thresholds.
- [ ] Every environment is expanded over distinct normalized HLAs.
- [ ] Endpoint candidates are unique on endpoint+n-mer+HLA.
- [ ] Query rows are unique on the exact Stage-2 compute key.
- [ ] Shared computations retain all endpoint crosswalk entries.
- [ ] PDAC full reproduces 9,200 endpoint+n-mer rows and 51,756 compute keys.
- [ ] Rojas/Sethna reproduces 231 endpoint+n-mer rows and 1,368 compute keys.
- [ ] Every Rojas/Sethna compute key is contained in the full PDAC query.

### Tensor computation

- [ ] The NCI-selected Level-1 settings are unchanged.
- [ ] Every primary compute key has complete tensor coverage.
- [ ] No Rojas/Sethna-only Stage-2 job exists.
- [ ] A computed zero is distinguishable from missing computation.
- [ ] Query, code, parameter, and tensor hashes are recorded.

### Downstream views

- [ ] Full PDAC and Rojas/Sethna predictions come from identical tensor hashes.
- [ ] Each view has an independent mapping hash and endpoint manifest.
- [ ] Both views receive fixed and selected adaptive L2 metrics.
- [ ] Rojas/Sethna is not included in worst-cohort selection by default.
- [ ] Rojas/Sethna is not double-counted in the primary tournament.
- [ ] Missing mapping-to-tensor joins fail closed.

### Reproducibility

- [ ] Rebuilding from the same inputs produces identical output hashes.
- [ ] All expected counts are asserted in tests and manifests.
- [ ] Historical filtered files remain provenance only, not active inputs.
- [ ] Documentation names `pdac_full` and `pdac_rojas_sethna` unambiguously.

## Stop conditions

Stop rather than patching around any of the following:

- the full PDAC generator reads the filtered Kd/AP/stability query;
- a Rojas/Sethna row is absent from the complete PDAC computation universe;
- a query contains duplicate Stage-2 compute keys;
- an endpoint candidate disappears when compute keys are deduplicated;
- a mapping contains conflicting labels for one endpoint;
- a homozygous allele changes an L2 result through multiplicity;
- missing computation is converted into `log(1e-12)`;
- changing a secondary view changes the primary query or tensor hash;
- the secondary view affects adaptive selection or primary tournament weighting
  without an explicit preregistered policy;
- a new stage attempts to overwrite a checksum-linked output directory.

## Completion criterion

This implementation is complete when a single validated complete PDAC tensor
can be joined without missing rows to both `pdac_full` and
`pdac_rojas_sethna`, both views produce independently hashed downstream metric
reports, and only the three declared primary cohorts influence the default
adaptive selection and production tournament.
