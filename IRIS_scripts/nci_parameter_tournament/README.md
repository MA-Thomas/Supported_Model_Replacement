# NCI parameter tournaments

This workflow declares one parameter set as one tournament system. For each of five component models and each metric, it:

1. ranks all 4,200 regimes by empirical AP for the CNAP branch or empirical AUROC for the AUROC branch (`regime_idx` breaks exact metric ties deterministically);
2. scans downward, accepting the first regime with a new exact `(d_pos, d_neg, M, N)` tuple, until `candidate_count` regimes have been accepted;
3. constructs an immutable organizer bundle and runs the existing complete directed round robin;
4. uses the organizer's fixed-policy graph reduction to obtain `S0`;
5. applies Appendix A Version 2: lexicographic incoming survival with challengers frozen to `S0`, followed only if needed by lexicographic worst-target CNAP regret against `S0`.

Exact final vectors remain tied. The workflow never substitutes an empirical-metric tie-break for Version 2.

The frozen configuration currently sets `candidate_count = 20` and uses the manuscript's `[0.01, 0.30]` CNAP range. Its file hashes bind the three NCI tensors and the five metric tables used to construct the native and reciprocal component models.

Build the two release binaries:

```bash
cargo build --release --manifest-path supported_ap_code/Cargo.toml \
  --package directed_round_robin_organizer \
  --package iris_nci_parameter_tournament
```

Then submit:

```bash
bash IRIS_scripts/nci_parameter_tournament/submit_nci_parameter_tournaments_slurm.sh \
  --run-root /path/to/nci_parameter_tournament_run
```

Use `--prepare-only` to build and audit all ten bundles and plans without submitting jobs. Final selections are written to `RUN_ROOT/version2/MODEL/METRIC/selection.json`; the separately retained `s0`, intermediate `t`, and final `s_op` arrays make unresolved ties explicit. Each incoming profile names its binding admissible challenger. When the tertiary rule is invoked, each regret profile names the binding evaluation, comparator, prevalence, and regret.
