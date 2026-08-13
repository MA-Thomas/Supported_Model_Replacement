# Conditional ProteinGym full-nesting illustration

This is a pedagogical conditional calculation, not a substantive ProteinGym
replacement conclusion. It asks what the V17 full nested calculation would
return **if** 40%–50% deleterious prevalence were scientifically applicable,
residue position were the actionable within-assay resampling unit, and the
illustrative policy required more than 60% literal survival.

## Conditional specification

- Comparison: ESM-1v ensemble − ESM-1v single.
- Empirical evaluations: 91 retained ProteinGym assays.
- Target prevalence: 40%–50%.
- Empirical order: K_E=2; computational order: K_C=2.
- Magnitude threshold and survival floor: δ=d=0.
- Illustrative survival requirement: γ=0.60.
- Computational replications: J=200 per assay.
- Resampling: draw the observed number of residue-position clusters with
  replacement within each assay; carry all substitutions at a selected
  position; redraw a complete bootstrap only when it lacks an outcome class.
- Profile: `publication`.

## Stage 1: observed empirical gate

- Mean assay anchor: +0.022877.
- Order-2 supported magnitude: +0.000590.
- Positive anchors: 74/91.
- Surviving assay pairs: 2,701/4,095
  = 0.659585.
- Gate verdict: `supported_replacement`.

## Stage 2: observed-anchored computational challenge

- Full supported magnitude: -0.016395.
- Full literal survival: 0.457419.
- Full conditional verdict: `no verdict`.
- Missing-class complete bootstrap redraws: 112 across
  18,200 retained computational
  evaluations.

Every complete challenge selects two assay rows, retains both observed anchors,
selects two computational effects within each row, and takes the minimum over
all six effects before averaging. Computational multiplicity never creates new
empirical rows.

## Interpretation

The interval, comparison, breadth requirement, and position-bootstrap law are
introduced to display the complete nested arithmetic on real paired assay data.
They were not established prospectively as the scientifically appropriate
ProteinGym deployment regime. Passage or failure therefore describes this
conditional calculation only and must not be reported as evidence for replacing
a ProteinGym model in practice.
