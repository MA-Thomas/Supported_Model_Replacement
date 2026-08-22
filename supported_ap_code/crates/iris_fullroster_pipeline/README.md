# IRIS full-roster pipeline

This crate owns the Rust-native production boundary from frozen Stage-2 tensors through the
fixed and adaptive tournament input bundles. All commands fail if their output already exists
and publish through an adjacent staging directory.

```text
iris_fullroster_pipeline validate-config --config CONFIG.json
iris_fullroster_pipeline transfer-batch --config CONFIG.json --output TRANSFERS
iris_fullroster_pipeline validate-transfers --package TRANSFERS
iris_fullroster_pipeline build-fixed-bundles \
  --config CONFIG.json --transfers TRANSFERS --output FIXED_BUNDLES
select_adaptive_hillq \
  --source-root TRANSFERS --bundle-root FIXED_BUNDLES --output SELECTION
iris_fullroster_pipeline build-combined-bundles \
  --base-bundles FIXED_BUNDLES --selection SELECTION --output COMBINED_BUNDLES
```

Start from `IRIS_scripts/iris_fullroster_pipeline.example.json`. Replace every
path and SHA-256 placeholder before validation. The same immutable configuration
must be supplied to `transfer-batch` and `build-fixed-bundles`; the latter
rejects a transfer package produced by any other configuration.

The configuration freezes the full, focal, and mono tensor files for every cohort, all five NCI
summary files, the Rust full-roster input-package manifest, the exact twelve fixed L2 operators,
and the tournament policy. The transfer package contains exactly 30 jobs (`3 x 5 x 2`) and is
the direct input to `select_adaptive_hillq`; no legacy `transfers/` wrapper is required.

Production artifacts use `l2_variant` and `l2_aggregation_*` terminology. The fixed bundles have
60 systems and the combined bundles have 70 systems / 7,245 directed-round-robin matches across
the three evaluations. Adaptive selection is accepted only when its recorded
transfer-manifest and PR/ROC base-bundle content hashes match the supplied fixed
bundles.
