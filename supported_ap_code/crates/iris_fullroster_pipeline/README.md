# IRIS full-roster pipeline

This crate owns the Rust-native boundary from complete-F Level-1 tensors to
the immutable directed-round-robin input bundles. It computes twelve fixed L2
operators and one frozen adaptive L2 operator for each of five component
models. All commands fail if their output already exists and publish through
an adjacent staging directory.

```text
iris_fullroster_pipeline validate-config --config CONFIG.json
iris_fullroster_pipeline transfer-batch --config CONFIG.json --output TRANSFERS
iris_fullroster_pipeline validate-transfers --package TRANSFERS
iris_fullroster_pipeline build-tournament-bundles \
  --config CONFIG.json --transfers TRANSFERS --output BUNDLES
```

Start from
`IRIS_scripts/configs/pipeline/iris_fullroster_pipeline.example.json` and
replace every path and SHA-256 placeholder. The same immutable configuration
must be supplied to both scoring commands; bundle construction rejects a
transfer package produced by any other configuration.

## Frozen scientific contract

The configuration schema is version 3. It freezes complete full, focal, and
mono tensor files for every cohort, all five NCI summary files, the input
package manifest, the twelve fixed operators, and this one adaptive operator:

```text
endpoint_local_epitope_second_hla_hybrid_v1
```

Its parameters are part of the validated schema: `c=-2.2`, `kappa=0.13`,
`t=-6.45`, `delta=0.02`, `B=1`, and `w=0.12`. They were developed using the
Full-HLA component model and are transported unchanged to all five component
models. There is no tournament-time adaptive selection step.

Each PR and ROC bundle therefore contains exactly 65 systems:

```text
5 component models x (12 fixed L2 operators + 1 frozen adaptive L2 operator)
```

Across the three primary evaluations this produces exactly
`C(65,2) x 3 = 6,240` directed pair-evaluation matches per metric bundle.

Primary analyses comprise 30 jobs (`3 cohorts x 5 models x 2 metric
branches`). Configured secondary views add ten diagnostic jobs per view under
`secondary/<view_id>/...`; their manifests record
`selection_eligible=false` and `bundle_eligible=false`, so they cannot enter
the primary bundles. The PDAC Rojas/Sethna mapping is such a secondary view.

Production artifacts use `l2_variant` and `l2_aggregation_*` terminology.
Coverage-by-omission mappings are rejected: every eligible mapping row must
resolve to a completed tensor observation. Treat every recorded content hash
as part of the scientific result and never edit a published output directory.
