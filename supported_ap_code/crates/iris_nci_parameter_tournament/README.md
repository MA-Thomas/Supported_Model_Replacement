# `iris_nci_parameter_tournament`

Domain-specific orchestration for NCI parameter-set tournaments. The crate constructs either complete regime rosters or metric-specific diverse candidate rosters, streams their scores from complete-F tensors, emits immutable bundles for `directed_round_robin_organizer`, and applies Appendix A Version 2 to audited exhaustive or accelerated selections.

Scientific responsibilities are deliberately separated:

- this crate owns roster construction, NCI score construction, and Version 2 operational ordering;
- `directed_round_robin_organizer` owns pairwise execution, artifact validation, and fixed-policy graph reduction;
- `supported-ap` owns CNAP, AUROC, literal-survival, and prevalence-range calculations.

Run `iris_nci_parameter_tournament --help` for the preparation, audit, and Version 2 commands. The complete NCI configuration and Slurm runbook live in `IRIS_scripts/nci_parameter_tournament/`.

## Roster modes

Existing configurations retain the original ranked selection procedure. The
optional `roster_mode` defaults to `ranked_unique_biological_key`, which ranks by
empirical metric and accepts the first regime for each exact `(d_pos, d_neg, M, N)`
tuple until `candidate_count` is reached.

To include the complete grid, set:

```json
"roster_mode": "all_regimes",
"candidate_count": 4200
```

In this mode, `candidate_count` is an **exact expected count**, not a selection
limit. Every regime is retained, including steepness variants sharing the same
tuple and regimes with identical scores. Both metric branches include the same
parameter sets, in ascending `regime_idx` order. Empirical metrics remain
descriptive data and do not determine inclusion or roster order.

Preparation verifies a complete, unique, zero-based regime index sequence and
checks every parameter tuple against the tensor's full geometry × M/N grid.
A shortened metrics table cannot silently become an "all-regimes" roster, even
if the configured count is shortened to match it. Scores are computed once per
distinct regime and reused between PR and ROC branches, with the existing
per-observation tau maximization and reciprocal Q-source rules.

Roster artifacts declare their mode, ordering rule, and diversity key (empty in
all-regimes mode). The existing `selected_rank` and scan `ranked_position` fields
mean roster position in this mode, not empirical metric rank. System IDs still
identify the component model, metric branch, and original regime index.

`audit-prepared --package PACKAGE --config CONFIG` also checks that a reused
package belongs to the exact requested configuration. All-regimes audits
reconstruct the full roster from the bound metrics table and tensor metadata.

Roster mode is separate from organizer execution mode. The new checked-in
`config.nci.all_regimes.v1.json` selects all 4,200 regimes; use the organizer's
`accelerated` commands (or the runbook's `--execution-mode accelerated`) to
execute them with the audited fast path. The original `config.nci.v1.json` and
the ranked candidate procedure remain available.

## Metric-specific Version 2 tertiary selection

After the fixed-policy survivor set `S0` and incoming-survival set `T`, only a
remaining tie invokes tertiary regret. PR tournaments retain worst-target CNAP
regret over the configured operational prevalence region. AUROC tournaments use
empirical AUROC regret with uniform within-class weights (`Gamma_op = 1`).
The optional `tournament.operational_auroc_gamma` defaults to 1; larger values
are not yet implemented and are rejected. Neither rule changes `S0` or `T`.

Every member of `S0` remains a benchmark, including candidates outside `T`.
Within each context, AUROC regret is the largest empirical AUROC in `S0` minus
the candidate's empirical AUROC. The implementation subtracts exact doubled
Mann–Whitney credits before normalization and gives score ties half credit.
It computes each benchmark once per context, requiring no new pairwise
computational evaluations. With multiple contexts, descending regret vectors
are minimized lexicographically; identical vectors remain tied. For NCI's
single context, this selects the highest empirical AUROC within `T`. Under the
current nonnegative magnitude threshold and strict observed AUROC gate, every
full-roster empirical AUROC maximizer already survives in `S0` and has an
all-zero incoming profile in `T`. Consequently this default returns the
full-roster empirical AUROC maximizers. Severe testing still determines the
supported directions and survivor sets; this operational rule does not choose
a lower empirical AUROC.

Selection schema 2 declares the tertiary rule even when it is not invoked.
CNAP rows remain in `range_regret_profiles`; AUROC rows are in
`auroc_regret_profiles`, recording both empirical AUROCs and the binding
comparator. AUROC reports omit the CNAP prevalence/search fields. The auditor
continues to read historical schema 1 selections as CNAP tertiary selections,
including those with AUROC as the primary metric. Saved results are not
rewritten; re-finalization requires a fresh output directory. Both exhaustive
and accelerated execution use the same metric-specific finalizer.

The historical `config.nci.v1.json` is preserved byte for byte because completed
N=20 artifacts bind its hash. Its old AUROC/CNAP scope wording describes those
historical results; new finalizations use the metric-specific rule above and
record it explicitly in selection schema 2. The all-regimes configuration
declares the updated scope and `operational_auroc_gamma: 1.0`.


## Portable cluster inputs and component finalists

`export-inputs --config CONFIG --output PACKAGE` copies each distinct input into
a closed, hash-verified package. `audit-inputs --package PACKAGE` validates the
package and rejects references outside it. Configuration input paths now resolve
relative to the configuration file. New prepared-package and Version 2 manifests
use relative source references; historical absolute-path manifests remain
readable. Transfer the whole run tree to preserve result auditability.

`prepare-finalists --config CONFIG --prepared PREPARED --version2 VERSION2_ROOT
--output FINALISTS` validates the ten parent selections, advances all tied
`s_op` members, aligns biological identities and labels, and builds separate PR
and ROC bundles. `audit-finalists --package FINALISTS` checks the package and
parent bindings. Finalist tournaments reuse the organizer and Version 2; empty
and singleton finalist sets have explicit terminal metadata.

`summarize-components --config CONFIG --finalists FINALISTS --selections
FINAL_VERSION2_ROOT --output SUMMARY_JSON` validates the final selections and
reports winning regime IDs and component sets. It declares a unique component
only when exactly one component remains. `audit-version2` also accepts paired
`--config` and `--bundle` arguments to verify the expected tournament/source
binding when resuming or consuming a parent selection.

The Slurm entry point is `IRIS_scripts/nci_parameter_tournament/
submit_nci_cluster_pipeline.sh` in the repository root. Its documentation covers
packaging, Linux builds, CPU/task allocation, dependency stages, and resumption.
