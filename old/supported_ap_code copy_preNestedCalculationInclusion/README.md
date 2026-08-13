# supported-ap

This crate implements the finite-evidence procedures in
`Stats_Paper_Extending_PRAUC_v17_Codex.tex`.

For AP, each complete evaluation is reduced in the manuscript's order:

1. compute the prior-standardized paired CNAP difference over the declared
   target-prevalence set;
2. retain the weakest difference within that complete evaluation;
3. apply the declared empirical or computational support order to the resulting
   finite effect list;
4. compute literal survival as the fraction of distinct subsets of that order
   whose members all strictly exceed the declared floor.

The projected API first assesses its one observed evaluation with empirical
order one. Only a direction that clears that gate receives the additional
computational challenge, and each computational effect is clipped by its
observed anchor. The observed API accepts genuinely observed evaluations
grouped by evaluation ID and assesses them directly at the empirical order.

The library also exposes the full nested `M > 1, K_C > 0` path through
`assess_staged_nested_ap`. It evaluates the observed order-`K_E` gate before
generating any computational effects, supports a different replication count
`J_m` and class-count design in every empirical row, and keeps each observed
anchor mandatory. The underlying `anchored_nested_support` function is
metric-agnostic and computes the exact finite-array magnitude and strict-floor
survival without enumerating complete nested selections. Existing projected and
observed CLI commands remain unchanged; a nested CLI/report surface is reserved
for a separate schema addition.

Replacement is supported only when both direct criteria are strict:

```text
supported magnitude > delta
literal distinct-subset survival fraction > gamma
```

Both model orientations are returned from the same paired evidence. There is no
permutation-calibration, outer-resampling, confidence-bound, or legacy schema
layer.

## AUROC case-mix challenge

The AUROC companion constructs the paired positive–negative gain matrix and
minimizes its weighted difference over capped within-class simplices. It applies
the same order-`K` support and literal-survival policy at each concentration
factor and brackets the first factor at which replacement fails.

The production combinatorial backend returns numerical lower and upper
optimization certificates. These bounds describe solver precision, not
statistical confidence. Unresolved numerical intervals remain explicit.
`Gamma = 1` and empirical-support saturation are handled analytically; each
intermediate solve is single-threaded while independent evaluations may run in
parallel. The optional `highs-reference` feature retains a small-problem
differential-check backend.

The library-level `estimate_nested_auroc_breakdown` API extends this challenge
to `M > 1, K_C > 0`. It runs the order-`K_E` observed gate before drawing any
computational rows, reuses each row's gain matrix and solver workspace throughout
the concentration search, and propagates every optimizer certificate through
the exact anchored nested operator. Lower aggregates certify passes, upper
aggregates certify failures, and intervals that settle neither side remain
`Unresolved`. The nested API supports heterogeneous `J_m` and class-count
designs; the existing AUROC CLI commands and report schemas are unchanged.

## CLI

Projected and observed evidence are separate commands:

```text
supported_ap ap projected ...
supported_ap ap observed --evaluation-id-col evaluation ...
supported_ap auroc projected ...
supported_ap auroc observed --evaluation-id-col evaluation ...
```

Every command requires an explicit empirical or computational order, magnitude
threshold, survival floor, survival requirement, and reference assessment. Projected commands also
require the number of computational replications, a resampling seed, and the
resampling unit. The reference assessment is recorded as scientific metadata
and never gates the finite-evidence verdict.

## Development

```text
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

# Optional small-problem differential checks
cargo test --features highs-reference
```

## Domain studies

`domain_study/` refits three models from checksum-verified UCI Bank Marketing
data, runs the V17 staged AP and AUROC assessments, and regenerates six
figures, tables, JSON reports, and a reproducibility manifest. See
`domain_study/README.md` for the declared design and reproduction commands.

`domain_study/proteingym/` compares already-released EVE, ESM-1v, and ESM-2
variant-effect scores. It retains manual-cutoff, single-substitution DMS assays
as separately identified observed evaluations and exercises the V17 observed
AP path without model fitting or computational resampling.
