# ProteinGym observed-evaluation study

This study compares already-released protein variant-effect scores. It trains no
model and performs no computational resampling. Each retained deep-mutational-
scanning assay is one observed full evaluation, so the evidence list has
`M = 91` assay-level effects.

## Declared regime

- ProteinGym `v1.3` substitution assays and metadata pinned at
  commit `144fe22b07dfaeec2b366f2346203a9838a55b4c`.
- Manual biological binarization cutoffs only.
- Single amino-acid substitutions only, including the single-substitution rows
  of assays that also measured multiple substitutions.
- Experimentally nonfunctional/deleterious variants are the positive class.
- Released ProteinGym fitness-oriented scores are negated so larger values
  favor the positive class.
- Every retained variant has finite EVE ensemble, ESM-1v single, ESM-1v
  ensemble, and ESM-2 650M scores on the same row.
- Target deleterious prevalence: 1%–90%.
- Empirical order `K_E = 2`; magnitude threshold `delta = 0`;
  survival floor `d = 0`; literal-survival requirement
  `gamma = 0.81`.
- Profile: `publication`. All AP, CNAP, prevalence minimization, support, and
  literal-survival calculations are performed by the V17 Rust implementation.

## Data retained

- 91 assays, 80 UniProt proteins, and
  60 publications.
- 277,873 single substitutions:
  96,987 deleterious and
  180,886 functional.
- Assay types: OrganismalFitness 39, Stability 21, Activity 19, Expression 8, Binding 4.
- 20 assay rows share a protein with
  another retained assay; 42
  come from publications contributing more than one assay.
- Assay-specific deleterious fractions range from 0.025
  to 0.869, with median
  0.315 and interquartile range
  [0.244, 0.440].
- 91 assays fall within the declared
  1%–90% target interval,
  0 fall below it, and 0 exceed it. The pooled
  deleterious fraction is 0.349; it weights assays by their
  retained substitution counts and is not the prevalence of a typical assay.

## Results

| Comparison | Order-2 support | Assays above 0 | Subset survival | Verdict |
|---|---|---|---|---|
| ESM-1v ensemble − EVE ensemble | -0.1548 | 49/91 | 0.287 | no verdict |
| ESM-2 650M − ESM-1v ensemble | -0.1175 | 39/91 | 0.181 | no verdict |
| ESM-2 650M − EVE ensemble | -0.1556 | 53/91 | 0.337 | no verdict |

`Order-2 support` is the supported magnitude of the finite list of assay-level
effects after each assay is first challenged throughout the complete declared
prevalence interval. `Subset survival` is the literal fraction of distinct
order-2 assay subsets whose two retained effects both strictly exceed zero.

## Observed-only AUROC companion

| Comparison | Order-2 support | Assays above 0 | Subset survival | Verdict |
|---|---|---|---|---|
| ESM-1v ensemble − EVE ensemble | -0.0663 | 56/91 | 0.376 | verified failure |
| ESM-2 650M − ESM-1v ensemble | -0.0445 | 48/91 | 0.275 | verified failure |
| ESM-2 650M − EVE ensemble | -0.0659 | 56/91 | 0.376 | verified failure |

AUROC is unchanged by the target prevalence. Each assay therefore contributes
its ordinary paired AUROC difference at `Gamma = 1`, after which the same
order-2 empirical support and literal assay-pair survival rule is applied. All
three declared comparisons fail this observed gate, so no within-assay
computational resampling or case-mix breakdown search is performed. This is the
`M = 91, K_E = 2, K_C = 0` boundary of the AUROC
construction.

## Interpretation

The results report observed support across the retained ProteinGym assay
contexts. They do not describe sampling from a population of hypothetical
assays, and repeated proteins or publications are not relabeled as independent
research programs. Publication and protein multiplicities are retained in
`tables/assay_audit.csv`.

The prevalence calculation holds each assay's observed class-conditional paired
score distributions fixed while changing finite library composition. It does
not transport performance to mutation mechanisms, proteins, or assays absent
from the retained collection. Variants within an assay are experimentally
structured, so no exchangeable-label reference law is asserted.

## Figures

1. `01-observed-assay-effects` shows the complete finite list of retained
   assay-level advantages for all three comparisons.
2. `02-two-part-support-decision` separates supported magnitude from literal
   persistence within the observed empirical gate.
3. `03-observed-auroc-decision` gives the corresponding observed-only AUROC
   magnitude and assay-pair survival results.
