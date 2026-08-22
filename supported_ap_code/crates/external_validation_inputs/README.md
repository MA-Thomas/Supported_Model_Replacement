# external_validation_inputs

This crate is the deterministic Rust replacement for
`build_mono_inputs_deprecated.py`. It converts immutable cohort inputs into the audited
full-roster package used by IRIS external-validation transfer scoring.

The biological candidate roster for an endpoint contains every 9--12-mer in
the source long-peptide mapping paired with every distinct allele in the
patient's full HLA environment. A pair represented in the filtered Stage 2
query is marked `scoreable`; every other pair is marked `floor`. The numeric
floor is intentionally not chosen here. Transfer scoring applies the frozen
`log(1e-12)` policy.

## Build

From the repository root:

```bash
cargo run --release \
  --manifest-path supported_ap_code/Cargo.toml \
  --package external_validation_inputs -- \
  build \
  --config IRIS_scripts/external_validation_inputs.example.json \
  --input-root /Users/thomm15/Work_Data \
  --output /path/to/full_roster_inputs_v1
```

The output path must not already exist. The crate builds in an adjacent
staging directory, validates the result, and then renames it into place.

For each cohort it emits:

- a representation-neutral `*_candidate_roster.csv`;
- a ready-to-score `*_full_longpep_mapping.csv`;
- a ready-to-score `*_mono_longpep_mapping.csv`;
- mono query, environment dictionary, and full-to-mono crosswalks;
- generated binding-simulator TOMLs;
- an audit JSON.

`manifest.json` records the configuration, source hashes, output hashes,
provenance inputs, and audit results.

## Validate

```bash
supported_ap_code/target/release/external_validation_inputs \
  validate --bundle /path/to/full_roster_inputs_v1
```

Validation recomputes every output hash, checks mapping statuses and HLA
values, verifies that full and mono mappings differ only in `env_id`, and
checks that every full mapping uses `env_id == full_env_id`.

## Representation routing

Use the full mapping when the primary P/N tensor is full or focal. Use the
mono mapping when the primary P/N tensor is mono. In a hybrid, the external Q
source does not change this rule.
