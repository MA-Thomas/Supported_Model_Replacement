# Bank Marketing finite-evidence study

The separate [`proteingym/`](proteingym/) study exercises the V17 observed-`M`
path using released EVE, ESM-1v, and ESM-2 scores across retained
deep-mutational-scanning assays. The banking study below exercises the projected
`J` path from one temporal evaluation.

The separate [`proteingym_contexts/`](proteingym_contexts/) study applies the
primary single-context assessment to each of the 91 retained assays, then
performs candidate-conservative and replacement-conservative graph reduction.
It includes a declared position-cluster computational challenge and saves its
own reports, tables, and figures.

The [`proteingym_auroc_contexts/`](proteingym_auroc_contexts/) companion applies
the same single-context design and graph reductions to ordinary AUROC
(Γ=1), using identical position draws and decision thresholds. It saves a
matched CNAP–AUROC comparison separately from the joint empirical studies.

The separate [`proteingym_nested/`](proteingym_nested/) workflow reuses the
retained ProteinGym scores for a clearly labeled conditional illustration of
the full `M>1`, `K_E=2`, `K_C=2` calculation with residue-position cluster
resampling. It has its own entry point and output directory and does not alter
either study described above.

This study rebuilds six pedagogical figures from the raw UCI Bank Marketing
data and the V17 Rust implementation. Clean runs refit every model and replace
all predictions, reports, tables, and figures.

Source: S. Moro, P. Rita, and P. Cortez, *Bank Marketing*, UCI Machine
Learning Repository, 2014, <https://doi.org/10.24432/C5K306>, CC BY 4.0.

The primary comparison is random forest versus full logistic regression. Full
logistic versus demographic-only logistic regression is a positive control.
The declared AP assessment uses:

- a 1%–50% target-prevalence interval;
- empirical order `K_E = 1` and computational order `K_C = 2`;
- magnitude threshold `delta = 0`;
- literal-survival floor `d = 0`;
- direct distinct-subset survival requirement `gamma = 0.81`;
- one observed temporal evaluation, projected with independent-row,
  class-stratified computational replications;
- no exchangeable-label assertion for the observational holdout.

The one temporal holdout is assessed first as the empirical gate. Only a
direction that clears it receives the computational challenge. Each
computational effect is anchored by the observed effect from which it was
derived; replications are not additional observed evaluations and are not
given a confidence interpretation.

The AUROC companion reuses the same fitted-model predictions and holdout score
pools. It replaces the AP prevalence challenge with a within-class concentration
challenge. Every projected replication keeps the observed holdout design (2,540
subscribers and 5,698 non-subscribers). The adaptive search brackets the
case-mix breakdown, and numerical optimization bounds are retained as solver
certificates rather than statistical intervals.

`duration` is excluded because it is unavailable at the pre-contact targeting
decision. The first 80% of source rows train every model from scratch; the final
20% form the temporal evaluation.

## Reproduce

The copied archive and extracted CSV under `data/raw/` are verified by SHA-256
before every run.

```sh
python3 -m venv --system-site-packages domain_study/.venv
domain_study/.venv/bin/python -m pip install -r domain_study/requirements.txt
sh domain_study/run.sh quick
```

Use `publication` for the larger computational replication counts and denser AP
diagnostic grid. Every clean run replaces `domain_study/outputs/`.

To rerun only AUROC from retained publication predictions and AP reports:

```sh
sh domain_study/run.sh auroc
```

To redraw outputs from retained publication reports without refitting or
resampling:

```sh
sh domain_study/run.sh render
```

Both reuse modes validate the retained V17 schemas and require the AP
replication profiles used by Figure 4.

## Figure sequence

1. **Ranking for contact decisions.** Connects model ranking to limited contact
   capacity.
2. **AP needs a target prevalence.** Holds a ranking fixed while changing the
   target subscriber prevalence.
3. **Advantage across the deployment range.** Shows each comparison throughout
   the declared 1%–50% prevalence range.
4. **Anchored computational challenge.** Shows how the observed anchor
   constrains the positive control's computational effects.
5. **Staged replacement decision.** Shows the empirical gate before the
   anchored computational magnitude and survival criteria.
6. **AUROC empirical case-mix breakdown.** Displays the baseline comparison and
   the positive control's projected breakdown under within-class concentration.

The prevalence interval is a declared deployment range, not uncertainty about
the observed prevalence. Likewise, the AUROC concentration factor challenges
redistributions of cases represented in the holdout; it does not establish
transport to customer types absent from that holdout.
