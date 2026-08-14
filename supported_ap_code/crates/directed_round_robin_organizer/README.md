# directed_round_robin_organizer

`directed_round_robin_organizer` runs complete deterministic
supported-evidence tournaments over opaque systems. It contains no domain
model, grouping, or interpretation logic, and no AP, CNAP, AUROC, resampling,
support, survival, or optimization mathematics. Each match is judged by the
workspace's `supported-ap` library.

The crate provides:

- independent `pr_cnap` and `auroc` tournaments;
- every unordered system pair in every evaluation, with both directed results;
- content-derived match IDs and seeds independent of paths, shards, retries,
  and execution order;
- deterministic sharding;
- atomic, checksummed, resumable match publication;
- fail-closed completeness auditing;
- three-state conjunction across evaluations;
- directed support graphs, SCC condensation, and source-SCC maximal systems;
- unique selection only when the graph-maximal set is a singleton; and
- descriptive metric and score-vector comparisons that cannot affect
  selection.

## Domain boundary

The organizer understands only opaque `system_id` values, evaluation IDs,
binary labels, aligned score vectors, and a supported-evidence policy. It does
not know about biological models, L3 aggregation, aggregation families, NCI
regimes, COVID, mutation, or any other provider domain.

Providers may retain domain details in opaque JSON `annotations` and source
provenance. The organizer never uses annotations for validation grouping,
graph construction, or selection. Domain-specific grouped analyses belong in
a separate reporting layer.

## Bundle layout

```text
bundle/
  bundle_manifest.json
  tournament_spec.json
  systems.json
  evaluations/
    <evaluation_id>/
      endpoints.csv
      scores.csv
      source_provenance.json
```

Schema version 2 `systems.json` registers each contestant as:

```json
{
  "system_id": "opaque_id",
  "display_label": "Human-readable label",
  "score_column": "score_opaque_id",
  "annotations": {}
}
```

`endpoints.csv` has `endpoint_id,label`. `scores.csv` has `endpoint_id` and
exactly one registered score column per system. The loader joins by key, sorts
canonically, requires both classes, and rejects missing, extra, duplicate, or
nonfinite values.

The manifest hashes files for audit. Its portable content hash is computed
from the frozen specification, opaque system registry, label-vector hashes,
and score-vector hashes, so paths and CSV row ordering do not change
tournament identity.

## CLI

From `supported_ap_code/`:

```text
directed_round_robin_organizer validate-bundle --bundle <bundle>
directed_round_robin_organizer bundle-content-hash --bundle <bundle>
directed_round_robin_organizer plan --bundle <bundle> --output <plan> --shards <n>
directed_round_robin_organizer run-shard --bundle <bundle> --plan <plan> \
  --shard-id <id> --results <results> --threads <n>
directed_round_robin_organizer status --plan <plan> --results <results>
directed_round_robin_organizer reduce --bundle <bundle> --plan <plan> \
  --results <results> --output <reduction>
directed_round_robin_organizer audit --bundle <bundle> --plan <plan> \
  --results <results> --reduction <reduction>
```

Planning also supports `--matches-per-shard <count>`. `--threads` is
match-level concurrency; judge assessments remain sequential to prevent nested
oversubscription. Match ordering interleaves evaluations after deterministic
content ordering, so small timing shards cover every evaluation when the shard
size is at least the number of evaluations.

## Generic reduction outputs

```text
reduction_manifest.json
completeness_audit.json
atomic_directed_verdicts.csv
evaluation_conjunctive_verdicts.csv
system_graph.json
system_components.json
selection.json
descriptive_metrics.csv
score_vector_comparisons.csv
```

Only an all-evaluation `supported` verdict creates an edge. Every vertex in a
source SCC survives. Multiple survivors remain an unresolved candidate set;
the organizer does not apply domain-specific refinements.

## Development

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The optional root-crate `highs-reference` feature additionally requires CMake
and a C++ toolchain; organizer production judgments do not use it.
