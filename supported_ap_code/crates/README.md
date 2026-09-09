# IRIS Rust workflow

The production tournament path is a fail-closed sequence. Each stage validates
the previous manifest, and every output directory must be new.

```text
cohort sources
  -> external_validation_inputs
  -> complete-F Level-1 tensors
  -> iris_fullroster_pipeline (transfers and 65-system bundles)
  -> directed_round_robin_organizer
```

`select_adaptive_hillq` remains the development and audit implementation for
adaptive-L2 investigations. It is not a production input stage for the frozen
hybrid tournament.

Build the production binaries from the repository root:

```bash
cargo build --release --manifest-path supported_ap_code/Cargo.toml \
  --package external_validation_inputs \
  --package iris_fullroster_pipeline \
  --package directed_round_robin_organizer
```

## Prepare a tournament locally

### 1. Build the full-roster input package

```bash
supported_ap_code/target/release/external_validation_inputs build \
  --config IRIS_scripts/configs/input_packaging/external_validation_inputs.example.json \
  --input-root /Users/thomm15/Work_Data \
  --output /new/path/full_roster_inputs
```

The output contains the authoritative endpoint candidate rosters, mappings,
crosswalks, generated tensor-query inputs, audits, and `manifest.json`.

### 2. Score transfers and build the 65-system bundles

Prepare a schema-3 production configuration from
`IRIS_scripts/configs/pipeline/iris_fullroster_pipeline.example.json`. It must
pin the complete-F input manifest, tensors, NCI summaries, twelve fixed L2
operators, and the frozen hybrid parameters.

```bash
PIPELINE=supported_ap_code/target/release/iris_fullroster_pipeline

$PIPELINE validate-config --config /path/pipeline.json
$PIPELINE transfer-batch --config /path/pipeline.json \
  --output /new/path/transfers
$PIPELINE validate-transfers --package /new/path/transfers
$PIPELINE build-tournament-bundles --config /path/pipeline.json \
  --transfers /new/path/transfers \
  --output /new/path/bundles
```

The transfer stage evaluates the same frozen adaptive function for every
cohort, metric branch, and component model. The bundle roster is exactly five
models times thirteen L2 operators: 60 fixed systems plus five frozen hybrid
systems.

### 3. Validate and plan on the cluster

```bash
ORGANIZER=supported_ap_code/target/release/directed_round_robin_organizer

$ORGANIZER validate-bundle --bundle /new/path/bundles/pr
$ORGANIZER validate-bundle --bundle /new/path/bundles/roc
$ORGANIZER plan --bundle /new/path/bundles/pr \
  --output /new/path/plans/pr --matches-per-shard 50
$ORGANIZER plan --bundle /new/path/bundles/roc \
  --output /new/path/plans/roc --matches-per-shard 50
```

Each metric bundle has 65 systems, three evaluations, and 6,240 matches. Build
the release organizer and create plans on IRIS so each plan records the Linux
executable identity. Uploading the 65-system PR/ROC bundles is sufficient for
the tournament; the 412-MB transfer tree is provenance and does not need to be
copied for match execution.

The operational wrapper is
`IRIS_scripts/tournament/submit_directed_round_robin_slurm.sh`. A frozen
prebuilt-bundle config is preferred because the wrapper validates the exact
65-system adaptive contract before planning.

## The main rule

Treat every manifest and bundle content hash as part of the scientific result.
Never edit or partially reuse a published output directory; create a new one
and let the next stage validate it.
