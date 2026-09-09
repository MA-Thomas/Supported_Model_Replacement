# `iris_nci_parameter_tournament`

Domain-specific orchestration for NCI parameter-set tournaments. The crate constructs metric-specific diverse candidate rosters, streams the selected regimes from complete-F tensors, emits immutable bundles for `directed_round_robin_organizer`, and applies Appendix A Version 2 to audited reductions.

Scientific responsibilities are deliberately separated:

- this crate owns the `(d_pos, d_neg, M, N)` roster rule, NCI score construction, and Version 2 operational ordering;
- `directed_round_robin_organizer` owns pairwise execution, artifact validation, and fixed-policy graph reduction;
- `supported-ap` owns CNAP, AUROC, literal-survival, and prevalence-range calculations.

Run `iris_nci_parameter_tournament --help` for the preparation, audit, and Version 2 commands. The complete NCI configuration and Slurm runbook live in `IRIS_scripts/nci_parameter_tournament/`.
