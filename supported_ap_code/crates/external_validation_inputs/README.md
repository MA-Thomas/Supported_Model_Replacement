# external_validation_inputs

This crate builds deterministic complete-F external-validation inputs for
COVID spike, COVID non-spike, and PDAC.

The primary endpoint mapping—not a presentation-filtered query—is the roster
authority. For every endpoint n-mer, the builder enumerates every distinct HLA
in the endpoint's full environment. It then separates two identities:

- endpoint candidate:
  `(endpoint identity, nmer, normalized HLA)`;
- Stage-2 computation:
  `(nmer, normalized HLA, env_id, expression)`.

Endpoint candidates are retained for aggregation. A shared Stage-2 computation
is emitted once and linked back to every endpoint through `compute_key_id`.
Homozygous copies therefore do not multiply either computation or L2 evidence.

Production mappings contain only `mapping_status=scoreable`. A missing
computation is a validation failure; it is never represented as an omission
floor. A completed biological zero remains a valid score and is handled by the
downstream locked `log(1e-12)` policy.

## Named analysis views

Schema version 2 supports secondary mappings that must be subsets of a primary
view. The production configuration uses:

```text
pdac_full          primary compute and selection cohort
pdac_rojas_sethna  secondary metrics-only view over pdac_full
```

The Rojas/Sethna view has its own mapping and output hashes but reuses the same
PDAC tensor. It is neither selection-eligible nor bundle-eligible by default.

## Build

First generate the PDAC endpoint views described in
`COMPLETE_F_INPUT_ROSTERS_AND_PDAC_VIEWS_IMPLEMENTATION_PLAN.md`, then copy the
example configuration into the immutable run root and replace its placeholder
paths.

```bash
cargo run --release \
  --manifest-path supported_ap_code/Cargo.toml \
  --package external_validation_inputs -- \
  build \
  --config /path/to/immutable_run/external_validation_inputs.complete_f.json \
  --input-root /Users/thomm15/Work_Data \
  --output /path/to/immutable_run/full_roster_inputs
```

The output path must not exist. Construction occurs in an adjacent staging
directory, validation runs before publication, and the validated staging tree
is renamed atomically.

For each cohort the bundle contains:

- complete full and mono computational query rosters;
- full and mono primary endpoint mappings;
- a representation-neutral candidate roster;
- stable computation/environment crosswalks;
- generated full and mono binding-simulator TOMLs;
- secondary view mappings where configured;
- a per-cohort audit JSON.

`manifest.json` records source hashes, output hashes, expected counts, primary
and secondary view contracts, and generator identity.

Before a generated TOML is passed to the cluster launcher, run
`/Users/thomm15/Work_Data/IRIS_scripts/validate_complete_f_stage2_inputs.py validate`
for its dataset and
representation. This checks the resolved query, hash, row count, uniqueness,
active environments, and peptide-length distribution against this manifest.

## Validate

```bash
supported_ap_code/target/release/external_validation_inputs \
  validate --bundle /path/to/immutable_run/full_roster_inputs
```

Validation recomputes every output hash and checks that:

- full queries are unique on the exact Stage-2 key;
- `compute_key_id` is unique and nonblank;
- full and mono mappings differ only in representation environment ID;
- every mapping row is `scoreable` and resolves to the primary query;
- canonical, full, and mono mappings have identical biological membership;
- every secondary view remains inside the primary compute universe;
- secondary views are not selection- or bundle-eligible.

## Representation routing

Use the full mapping when the primary P/N tensor is full or focal. Use the mono
mapping when the primary P/N tensor is mono. A hybrid's external Q source does
not change this mapping rule.
