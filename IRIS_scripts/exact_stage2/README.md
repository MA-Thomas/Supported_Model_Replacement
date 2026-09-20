# Exact survivor Stage 2 source and deployment

`runtime/` is the reviewed source overlay for
`/Users/thomm15/Work_Data/IRIS_scripts` and its IRIS copy. The entry point and
cluster instructions are in
[runtime/complete_f_stage2/README.md](runtime/complete_f_stage2/README.md).

The refreshed input package is at
`cluster_inputs/complete_f_survivor_v1` in the repository root. Its 40 retained
cohort-input files are byte-identical to the preceding audited package; the
fixed grid tables were replaced by exact survivor manifests and CSVs.

`build_grids.py` derives computational unions from the ten merged certificates
and prepared candidate registries. It retains every branch survivor, includes
all six tau values, and assigns the hybrid models to their full/mono P/N
sources. `input_package_config.json` is the staged configuration used to rebuild
this package with the existing Rust input builder. The canonical packaging
configuration under `IRIS_scripts/configs/input_packaging/` now points to the
installed exact-grid files.

`baseline_hashes.json` supports a fail-closed installation: changed source
files may only replace the versions inspected at the start of this task.
`numerical_source_hashes.json` records the unchanged Runner/P/N/Q/Pi Rust code.
The installer updates only the launcher, grid/input packages, validation,
assembly, and associated documentation/tests. It archives the superseded
package directories before replacing them; those archives are not a supported
compatibility interface.

See `VALIDATION.md` for the verification record and limits.
