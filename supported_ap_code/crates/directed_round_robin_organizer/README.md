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
- two selection strategies — candidate-conservative (default) and
  replacement-conservative — both reported for every reduction (see
  "Selection strategies");
- unique selection only when the selected set is a singleton; and
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

## Selection strategies

The reduction reports **two** selections over the completed round robin, each a
pure deterministic function of the atomic directed verdicts — no match is re-run
and no new mathematics is introduced. `tournament_spec.json` declares which one
is primary via `selection_strategy` (default `candidate_conservative`). The
primary is written to `selection.json` and the other to
`alternate_selection.json`, so a run always carries both.

**`replacement_conservative`.** An edge A->B (a supported claim that A should
replace B) is created only when the same direction is supported in *every*
evaluation (the all-evaluation conjunction). The graph is condensed and every
vertex in a source strongly connected component survives; a system is removed
only when a supported edge enters it. This is conservative about *asserting
replacements*: a defeat in one context does not remove a system unless the same
direction survives the conjunction across all contexts. It returns broad
admissible sets and is never empty.

**`candidate_conservative`** (default). The maximal set is computed separately
for each evaluation (`per_evaluation_maximal.json`) and the survivors are the
*intersection*: a system is retained only if it is maximal in every evaluation.
A single-context defeat is fatal. This is conservative about *retaining
candidates*, and it is the sharper, more eliminative rule — its survivor set is
always a subset of the replacement-conservative set. An empty result is a valid,
informative outcome (`outcome.kind = "no_surviving_candidate"`): no system is
top-tier in every context, i.e. the evidence favors domain-specific tradeoffs
rather than one universal model.

### Which to use

The two rules answer different questions and can disagree on identical verdicts:

```text
evaluation X: B is supported over A
evaluation Y: C is supported over A
```

The replacement-conservative rule *retains* A (neither B->A nor C->A is
supported in every evaluation, so no eliminating edge forms). The
candidate-conservative rule *removes* A (A is maximal in neither X nor Y).

Prefer `candidate_conservative` when you want to drive toward a single best
model: it subjects each candidate to an independent per-context test and
eliminates it on any failure, so it is the more falsificationist choice —
survival requires passing every severe test, and it declines the immunizing move
of shielding a candidate from single-context refutation. Prefer
`replacement_conservative` when you only want to assert a replacement that holds
across every context, or when robustness to a single under-powered evaluation
matters.

That last point is the price of the sharper rule: candidate-conservative is
falsificationist **only to the extent that each per-evaluation test is severe**.
It eliminates a system the instant that system is non-maximal in *any*
evaluation, including the weakest-powered one, so a small or noisy cohort can
discard a candidate that is excellent elsewhere. Where an evaluation is
under-powered, treat an elimination driven by that cohort as provisional and
read `per_evaluation_maximal.json` to see which context did the eliminating; the
replacement-conservative rule is more robust there because it requires one rival
to dominate everywhere before any system is removed.

Both selections carry a `strategy` field and a `survivor_evidence` block giving
each survivor's pairwise conjunctive verdicts against every other system.
`induce-view` and the revision-composition commands honor the same declared
`selection_strategy` and each also emits both selections.

The strategy is a **reduction-time** choice, not part of the tournament's frozen
identity: it is excluded from `bundle_content_hash`, the policy hash, and every
match and plan identity. Changing it therefore never invalidates a bundle, a
plan, or completed match results — only the reduction is recomputed. `reduce` and
`induce-view` accept `--selection-strategy` to override the bundle's declared
default for a single invocation; because both selections are written regardless,
the flag only chooses which one is primary. A reduction records its own strategy
and re-audits against that, so an overridden reduction remains verifiable.

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
  --results <results> --output <reduction> --threads <n> \
  [--selection-strategy <candidate-conservative|replacement-conservative>]
directed_round_robin_organizer audit --bundle <bundle> --plan <plan> \
  --results <results> --reduction <reduction> --threads <n>
directed_round_robin_organizer audit-reduction --bundle <bundle> --plan <plan> \
  --reduction <reduction> --threads <n>
directed_round_robin_organizer induce-view --bundle <bundle> --plan <plan> \
  --reduction <reduction> --exclude-system <id>... --output <view> --threads <n> \
  [--selection-strategy <candidate-conservative|replacement-conservative>]
```

Planning also supports `--matches-per-shard <count>`. `--threads` is the size
of one shared Rayon pool. Both independent matches and the parallel
computational evaluations within each match use that pool, so nested work is
scheduled against one fixed execution budget. Match ordering interleaves
evaluations after deterministic content ordering, so small timing shards cover
every evaluation when the shard size is at least the number of evaluations.

## Generic reduction outputs

```text
reduction_manifest.json
completeness_audit.json
atomic_directed_verdicts.csv
evaluation_conjunctive_verdicts.csv
system_graph.json
system_components.json
selection.json
alternate_selection.json
per_evaluation_maximal.json
descriptive_metrics.csv
score_vector_comparisons.csv
```

`selection.json` holds the primary selection (the strategy named by
`selection_strategy`, default `candidate_conservative`) and
`alternate_selection.json` the other strategy; `per_evaluation_maximal.json`
records each evaluation's maximal set. Multiple survivors remain an unresolved
candidate set and the organizer applies no domain-specific refinements. The two
strategies are defined and contrasted in "Selection strategies" below.

`induce-view` derives a smaller graph from one audited complete reduction. It
filters the excluded vertices and their pairwise rows, then recomputes the
all-evaluation conjunction, SCC graph, maximal set, and survivor evidence. The
view records the full reduction-manifest hash and never represents itself as a
second tournament. This supports, for example, two 65-node policy views of one
70-system run by excluding the five adaptive systems from the other policy.

## Context revisions

`compose-revision` supports an audited replacement of one evaluation directly
from raw result sets. Production finalization uses
`compose-revision-from-reductions`: it loads the much smaller base and revision
reductions, verifies their manifests and hashes, reconstructs and checks their
conjunctions, graphs, selections, match directions, and score comparisons, and
then performs the same composition. Both paths verify identical system
registries, scientific policies, and pairwise resampling seeds before
substituting the revision's atomic verdicts and rerunning the declared
conjunction, graph, and selection rules. Regression tests require the two paths
to produce identical compositions. `audit-revision` verifies the composition
manifest, every output hash, both source plan identities, and the composition
content hash.

A composition is written separately from ordinary reductions and records the
source bundle and plan for every atomic verdict. Its `graph_delta.json` reports
supported edges and maximal systems gained, lost, or retained relative to the
base tournament. Reductions and compositions are built in hidden staging
directories and renamed into place only after their final manifests are
written, so an interrupted finalizer cannot publish a partial checkpoint.

## Development

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The optional root-crate `highs-reference` feature additionally requires CMake
and a C++ toolchain; organizer production judgments do not use it.
