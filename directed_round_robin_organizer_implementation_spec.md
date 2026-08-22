# `directed_round_robin_organizer`: Domain-Agnostic Implementation Specification

**Status:** implemented contract, schema version 2  
**Repository:** `Supported_Model_Replacement`  
**Primary mathematical crate:** `supported-ap`  
**Organizer crate:** `supported_ap_code/crates/directed_round_robin_organizer`

## 1. Purpose

`directed_round_robin_organizer` runs complete, deterministic, directed
supported-evidence tournaments over opaque systems.

The organizer has no knowledge of the scientific domain that produced a
system. In particular, it does not know about biological models, L3
aggregation, aggregation families, NCI regimes, disease cohorts, mutations,
response thresholds, or component composition.

Its responsibilities are limited to:

1. validating a portable registry of opaque contestants and aligned score
   vectors;
2. enumerating every unordered system pair in every evaluation;
3. deriving stable match identities and seeds;
4. calling `supported-ap` for both directed questions in each match;
5. publishing complete, atomic, auditable match artifacts;
6. combining directed verdicts across declared evaluations by conjunction;
7. constructing the directed support graph and its strongly connected
   components; and
8. returning every vertex in a source strongly connected component as the
   graph-maximal system set.

Domain-specific interpretation is performed before or after the organizer by
separate software.

## 2. Architecture and responsibility boundary

```text
domain-specific score provider
  discovers systems, constructs scores, aligns endpoints, freezes labels,
  records scientific provenance, and emits an opaque portable bundle
                            |
                            v
directed_round_robin_organizer
  validates, schedules, shards, executes, audits, conjoins, and analyzes
  a directed graph over opaque system IDs
                            |
                            v
supported-ap
  judges one paired comparison under a declared metric-specific policy
                            |
                            v
optional domain-specific reporter
  interprets system annotations and performs any domain-specific grouping
```

### 2.1 Score provider

A provider owns all domain and environment knowledge. It must:

- decide what constitutes a contestant;
- construct or locate each contestant's score vector;
- define endpoint identity and labels;
- align every contestant to an identical endpoint roster within an evaluation;
- preserve source checksums and domain provenance;
- freeze the metric-specific judgment policy; and
- emit the schema defined here.

A provider may store domain metadata in opaque `annotations` objects and
`source_provenance.json`. The organizer validates only annotation-key syntax;
it never interprets annotation values.

The provider must not calculate a supported-evidence verdict.

### 2.2 `supported-ap`

`supported-ap` exclusively owns AP, CNAP, AUROC, resampling, supported
magnitude, literal survival, finite-evidence verdicts, and optimization
certificates.

### 2.3 Organizer

The organizer owns generic tournament mechanics only. It must never:

- infer relationships among systems from their names or annotations;
- require a rectangular domain-specific factorization of systems;
- reject a contestant because its name resembles a domain-specific category;
- group systems by provider annotations;
- use raw metric values to break a graph tie; or
- repair, impute, clip, reorder without keyed validation, or otherwise alter
  provider scores or labels.

## 3. Tournament semantics

One bundle is single-metric. Schema version 2 supports:

- `pr_cnap`, using the official forward staged supported-superiority verdict;
- `auroc`, using the official supported-AUROC verdict.

Let `K` be the number of opaque systems and `C` the number of evaluations.
The tournament contains:

```text
C * choose(K, 2)
```

unordered match artifacts. Each artifact contains both directed results:

```text
A over B
B over A
```

Failure to support `A over B` is not support for `B over A`.

## 4. Portable bundle schema

Schema version 2 uses:

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
      ... optional provider files not referenced by the organizer
```

All paths referenced by the manifest are relative to the bundle root and may
not escape it. Absolute source paths may occur only inside provenance data.

### 4.1 `systems.json`

```json
{
  "schema_version": 2,
  "systems": [
    {
      "system_id": "opaque_stable_id",
      "display_label": "Human-readable name",
      "score_column": "score_opaque_stable_id",
      "annotations": {}
    }
  ]
}
```

Required organizer semantics:

- `system_id` is a unique stable machine identifier;
- `display_label` is nonempty and otherwise uninterpreted;
- `score_column` is a unique stable column identifier; and
- `annotations` is an optional JSON object whose keys are stable identifiers.

There are no model, branch, regime, aggregation, family, or domain fields in
the organizer schema. A provider may place such information inside
`annotations`, where it is opaque to the organizer.

The system registry need not form any Cartesian product or grouped design.

### 4.2 `endpoints.csv`

Required columns:

```text
endpoint_id,label
```

Rules:

- `endpoint_id` is unique and nonempty within the evaluation;
- `label` is binary;
- both label classes must be present; and
- row order has no scientific meaning.

Original domain identity columns may be retained in a provider-owned sidecar
referenced by provenance. The organizer sees only opaque endpoint IDs.

### 4.3 `scores.csv`

Required columns:

```text
endpoint_id,<exactly one score column per registered system>
```

Rules:

- the endpoint roster must exactly equal `endpoints.csv`;
- there are no additional or missing score columns;
- every score is finite; and
- rows are joined by `endpoint_id`, then canonicalized by sorted ID.

### 4.4 `source_provenance.json`

This must be a JSON object. Its content is provider-owned and may be
domain-specific. It is byte-hashed for audit but does not participate in
portable scientific identity.

### 4.5 `tournament_spec.json`

The schema is:

```text
schema_version = 2
metric
verdict_field
evidence_policy
computational_design
reference_assessment
master_seed
seed_derivation_version
evaluations
conjunction_rule = all_evaluations
graph_maximality_rule = source_strongly_connected_components
selection_rule = source_scc_maximal_vertices
pr_cnap or auroc
annotations = optional opaque object
operational_tie_break = optional complete system-ID permutation
```

Exactly one metric-specific judge specification must be present. The
organizer does not contain label or endpoint rules for any named domain.

`annotations` participates in the frozen specification hash but cannot affect
generic validation or reduction semantics.

### 4.6 `bundle_manifest.json`

The manifest contains:

- schema name `directed_round_robin_input_bundle`;
- schema version `2`;
- metric;
- creation time;
- relative paths and SHA-256 hashes for systems, specification, and each
  evaluation's endpoints, scores, and provenance;
- provider identity;
- minimum organizer schema version; and
- portable bundle content hash.

The portable content hash is computed from canonical scientific content:

- frozen tournament specification;
- sorted opaque system registry;
- per-evaluation canonical label-vector hash; and
- per-evaluation per-system canonical score-vector hashes.

Paths, timestamps, provenance text, and CSV row order do not change portable
scientific identity.

## 5. Validation

Bundle validation fails closed on:

- unsupported or mixed schema versions;
- unsafe paths or hash mismatches;
- duplicate system IDs or score columns;
- missing, additional, duplicate, or nonfinite scores;
- endpoint-roster differences;
- missing label classes;
- invalid metric-specific policy;
- an evaluation registry inconsistent with the manifest;
- an invalid operational tie-break; or
- a portable content-hash mismatch.

The organizer does not validate the scientific meaning of annotations. That
is the provider's responsibility.

## 6. Match enumeration and identity

Systems and evaluations are sorted by their stable IDs. For every evaluation,
all unordered pairs are enumerated in canonical `(system_low, system_high)`
order.

`MatchId` is a digest of canonical content including:

- schema version;
- tournament content hash;
- metric;
- evaluation ID;
- canonical system pair;
- both score-vector hashes;
- assessment-policy hash; and
- seed-derivation version.

It excludes paths, timestamps, shards, retries, process IDs, and scheduler
metadata.

Version-1 seed derivation hashes:

- master seed;
- metric;
- evaluation ID;
- canonical system pair; and
- seed-derivation version.

Both directions in a match share the paired computational design.

## 7. Deterministic sharding

The plan supports:

- requested total shard count; or
- target matches per shard.

Shards are execution containers, not scientific objects. Changing sharding
may change `plan_id`, but never match IDs, seeds, or scientific results.

Before assigning shards, matches are deterministically ordered by interleaving
evaluations and content-ordering the system pairs within each evaluation. This
makes small timing shards evaluation-stratified when the shard size is at
least the evaluation count, while remaining agnostic to evaluation meaning.

The plan records every assignment and validates the expected count:

```text
evaluation_count * choose(system_count, 2)
```

## 8. Judgment

The organizer calls `supported-ap` as a Rust library. It does not invoke a
CLI per match and does not implement metric mathematics.

For each match it supplies the shared labels and two aligned score vectors,
retains the complete official reports, and normalizes each directed result to:

- `supported`;
- `not_supported`; or
- `unresolved`.

Only `supported` creates an evidential edge. A scientific `unresolved` result
is a completed match, distinct from operational failure.

`run-shard --threads N` constructs one local Rayon pool with exactly `N`
workers. Shard-level match scheduling and the independent computational
evaluations exposed by `supported-ap` both use this pool. Nested Rayon work is
therefore work-stealed within one execution budget rather than assigned a
separate pool per match. Thread count and sharding are operational choices;
they do not enter match identity, seed derivation, or scientific reduction.

## 9. Atomic publication and recovery

Match artifacts are written to unique temporary files, flushed, reopened,
validated, atomically renamed, and accompanied by SHA-256 sidecars.

A valid existing artifact is reused. It is never silently overwritten. An
invalid or conflicting artifact stops or fails the relevant task and remains
visible to audit.

## 10. Completeness audit

Reduction requires exactly one compatible completed artifact for every
planned match. Audit verifies tournament, plan, match, metric, evaluation,
system pair, input-vector hashes, policy hash, seed, and judge identity.

Missing, corrupt, duplicated, extra, or incompatible artifacts prevent final
reduction. Scientific unresolved verdicts do not make a tournament
operationally incomplete.

## 11. Generic reduction

### 11.1 Atomic directed table

For every evaluation and ordered system pair, retain one three-state verdict.

### 11.2 Conjunction across evaluations

For an ordered pair `(A,B)`:

- `supported` iff every evaluation supports `A over B`;
- `unresolved` iff at least one evaluation is unresolved and all remaining
  evaluations support `A over B`; and
- `not_supported` otherwise.

### 11.3 Directed support graph

Vertices are opaque system IDs. Edge `A -> B` exists exactly when the
all-evaluation directed result is `supported`.

The graph implementation computes:

1. strongly connected components;
2. the condensation DAG;
3. source SCCs, meaning components with no incoming edge from another SCC;
4. the union of vertices in source SCCs; and
5. per-vertex status, including supported wins, losses, cycles, and unresolved
   comparisons.

No transitivity or acyclicity is assumed.

### 11.4 Selection

The organizer's only scientific selection rule is:

```text
retain every vertex in a source strongly connected component
```

If one vertex remains, report a unique conjectured system. If multiple
vertices remain, report the complete unresolved candidate set.

No annotation, raw metric value, supported-difference magnitude, domain group,
or family may refine this set.

An optional provider-prespecified operational ordering may identify a separate
non-evidential operational choice among surviving systems. It must never be
reported as scientific selection.

### 11.5 Descriptive outputs

Empirical metric values, observed directed differences, exact score-vector
identity, and rank equivalence are retained descriptively. They never affect
the support graph or selection.

## 12. Reduction outputs

```text
reduction/
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

The organizer emits no domain-specific grouped graphs, stability analyses, or
claims. Such reports belong in an external domain-specific consumer that joins
these generic outputs to provider annotations.

## 13. IRIS provider conformance

The initial IRIS provider is a separate Python program under
`Supported_Model_Replacement/IRIS_scripts/`.

It owns:

- the biological component-model definitions;
- metric-specific NCI branch provenance;
- the L3 aggregation menu;
- target tensor and mapping locations;
- cohort-specific endpoint identities and label contracts;
- full/mono representation joins;
- the accepted COVID SPIKE measurement-error statement; and
- source-artifact checksums.

Each IRIS `(component model, metric branch, L3 aggregation)` combination is
registered as one opaque organizer system. The IRIS provider stores its
structure inside system `annotations`; the organizer neither requires nor
interprets those fields.

Aggregation families are not part of the IRIS provider or organizer contract.

Any IRIS analysis such as fixed-L3 model contrasts, fixed-model L3 contrasts,
or cross-L3 robustness must be produced after generic reduction by an
IRIS-specific reporting program. It cannot alter the organizer's graph-maximal
set.

## 14. Testing requirements

The implementation must test:

- schema and hash validation;
- opaque systems with arbitrary annotations;
- rejection of duplicate systems and score columns;
- row-order invariance;
- complete pair enumeration;
- stable match IDs and seeds across sharding plans;
- both directed outcomes per match;
- three-state conjunction;
- SCCs, cycles, source components, and graph-maximal selection;
- atomic restart and recovery;
- seed-identical scientific results across one-thread and multithreaded Rayon
  execution;
- fail-closed completeness and reduction audits;
- PR/CNAP and AUROC judge adapters; and
- end-to-end provider bundles against the organizer's own validator.

Domain-specific tests belong to their providers. For IRIS these include NCI
branch consistency, tensor/mapping provenance, cohort endpoint identity,
transfer roster equality, L3 availability, and label consistency.

## 15. Final implementation rule

The score provider supplies official opaque contestants. `supported-ap`
judges paired vectors. `directed_round_robin_organizer` runs and reduces the
complete generic tournament. Domain meaning remains outside the organizer.
