# directed_round_robin_organizer

`directed_round_robin_organizer` runs deterministic supported-evidence
tournaments over opaque systems, with exhaustive SCC reduction and a separately
audited accelerated candidate-conservative mode. It contains no domain
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
- unique evidential selection only when the selected set is a singleton; and
- descriptive metric and score-vector comparisons that cannot affect the
  evidential selection.

## Domain boundary

The organizer understands only opaque `system_id` values, evaluation IDs,
binary labels, aligned score vectors, and a supported-evidence policy. It does
not know about biological models, L3 aggregation, aggregation families, NCI
regimes, COVID, mutation, or any other provider domain.

Providers may retain domain details in opaque JSON `annotations` and source
provenance. The organizer never uses annotations for evaluation grouping,
graph construction, or selection. Domain-specific grouped analyses belong in
a separate reporting layer.

## Selection strategies

The reduction reports **two evidential selections** over the completed round
robin, each a pure deterministic function of the atomic directed verdicts — no
match is re-run and no new mathematics is introduced. `tournament_spec.json`
declares which one is primary via `selection_strategy` (default
`candidate_conservative`). The primary is written to `selection.json` and the
other to `alternate_selection.json`, so a run always carries both. The
operational winner strategy described below is a subsequent reduction of a
plural evidential survivor set and does not change either selection.

**`replacement_conservative`.** An edge A->B (a supported claim that A should
replace B) is created only when the same direction is supported in *every*
evaluation (the all-evaluation conjunction). The graph is condensed and every
vertex in a source strongly connected component survives; a system is removed
only when a supported edge enters its SCC from another SCC. This is conservative about *asserting
replacements*: a defeat in one context does not remove a system unless the same
direction survives the conjunction across all contexts. It returns broad
admissible sets and is never empty.

**`candidate_conservative`** (default). The maximal set is computed separately
for each evaluation (`per_evaluation_maximal.json`) and the survivors are the
*intersection*: a system is retained only if it is maximal in every evaluation.
A single context in which the candidate is outside every source SCC excludes
it. If each context graph is acyclic, a single supported incoming edge suffices,
and the candidate-conservative set is a subset of the replacement-conservative
set. Neither simplification is assumed for general cyclic graphs. An empty result is a valid,
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

Prefer `candidate_conservative` when you want to narrow the universally
admissible set: it subjects each candidate to an independent per-context test and
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

## Operational choice from the survival-threshold profile

The evidential reduction may intentionally return several admissible systems.
When one system must nevertheless be chosen, a second, explicitly operational
reduction can use the numerical survival quantities already produced by the
pairwise assessments. This step does not revise the frozen survival requirement
or turn a post-result threshold into a prespecified evidential policy. The
primary output remains the fixed-policy survivor set; any single system chosen
below is reported separately as an **operational winner**.

The central distinction is between a measurement and its cutoff. For a directed
claim A->B in evaluation `e`, the assessment supplies literal survival
fractions, while `survival_requirement` supplies the threshold `gamma` that
turns those fractions into a binary verdict. Two claims with survival 0.80 and
0.20 are both unsupported at `gamma = 0.81`, but they are not equally close to
producing an edge.

Hold every other part of the assessment fixed. Define the **edge activation
strength** as the breakpoint `w` for which the direction is supported exactly
when `gamma < w` (equivalently, the supremum of the requirements that would
support it).
For a full two-stage assessment this is the smaller of the observed-gate and
full-assessment survival fractions, provided that both magnitude requirements
pass; if either magnitude requirement fails, the activation strength is zero.
For a reduced observed-only assessment, use the observed survival fraction and
its magnitude requirement. Thresholding activation strengths at any `gamma`
reconstructs the corresponding directed support graph. The distinct attained
strengths are its exact breakpoints: the graph is constant between consecutive
breakpoints and changes immediately below one. These intervals form the
**survival-threshold profile**; an arbitrary decimal grid is unnecessary.

The threshold profile explains when arrows emerge as `gamma` is lowered, but it
should not select a winner by stopping at the first singleton. Adding edges can
create or merge strongly connected components, so graph-maximal membership need
not change monotonically. The profile is therefore a diagnostic of the complete
reduction path, not a replacement for a declared choice rule.

For the NCI Version 2 operational choice, freeze the fixed-policy survivor set
S0 as both the eligible winners and the challenger/comparator cohort. For each
eligible system B, collect every incoming activation strength A->B over all
challengers and evaluations and sort them from largest to smallest. Compare
eligible systems lexicographically, preferring the system with the smaller
largest incoming strength, then the smaller second-largest strength if the
largest values tie, and so on. The first comparison is a minimax rule: the
strongest observed replacement claim against B controls, and favorable
comparisons cannot compensate for it. The later comparisons resolve finite
ties without averaging threats.

This operational cohort restriction does not apply to evidential selection:
an excluded system remains a challenger when testing other candidates. Supported
replacement is not assumed transitive.

## Accelerated candidate-conservative selection

The exhaustive commands and their schemas are unchanged. The separate
`accelerated` command family selects the same intersection of context source-SCC
sets while retaining explicit exclusion witnesses and survivor certificates.
It always requests candidate-conservative selection, independently of the
bundle's default reduction strategy; this request is recorded in the plan.

```sh
directed_round_robin_organizer accelerated plan \
  --bundle bundle --output accelerated_plan --batch-size 1
directed_round_robin_organizer accelerated run \
  --bundle bundle --plan accelerated_plan --results accelerated_results \
  --output accelerated_selection --threads 8
directed_round_robin_organizer accelerated audit \
  --bundle bundle --plan accelerated_plan --results accelerated_results \
  --selection accelerated_selection
```

Applicability is established separately for each context by the current judge
and validated policy. PR uses the common observed CNAP at a prevalence actually
included in the empirical gate's search (the interval lower endpoint, or the
first finite-set point). Its value is computed by `supported-ap` using the same
tie averaging kernel as the gate. A direction whose endpoint difference is at
most delta cannot pass that gate. ROC uses exact doubled Mann–Whitney credits:
Gamma=1 support requires the winner's credit to be strictly larger. These strict
potentials prove that the original context support graphs are acyclic. This is
a judge contract, not a claim that SCC condensation proves the original graph
acyclic, and not a conclusion drawn from sampling some comparisons.

Unavailable order primitives fall back to a full-roster SCC computation for
that context. `--exhaustive-context CONTEXT` explicitly disables its fast path
and can be repeated. Cycles in such a context are preserved: an exclusion
witness identifies the entire non-source component and an edge entering it
from outside. An observed numerical contradiction in an ordered context fails
the run without publishing a selection; investigate and replan that context
with the explicit exhaustive option. The default does not silently reinterpret
contradictory evidence or use an epsilon to assert an ordering.

The plan is O(contexts × systems), without enumerating all match objects. A
single coordinator issues bounded immutable batches, reads opponents in a
deterministic context-specific order, and commits worker results in request
order. An ordered-context target stops after its first supported incoming
witness. Excluded targets remain available as challengers. Work is resumable
from validated immutable pair artifacts; use the same plan/results and a new
selection output directory. Pair identities and seeds match exhaustive runs,
but artifacts retain their actual plan provenance and cannot simply be copied
between plan directories. The first implementation is a local coordinator;
it does not provide a distributed batch dispatcher or import old-plan evidence.

Batch size 1 minimizes speculative matches and can still parallelize the
judge's internal work. Larger batches (default 16) also parallelize opponents
but may finish unnecessary comparisons after a decisive witness. Neither batch
size nor worker completion order changes pair seeds or final selection. Worst
case work remains quadratic when many comparisons are necessary.

`selection_certificate.json` is untrusted input to the auditor. The audit
recomputes the context potentials, checks report hashes and original run
bindings, checks normalized directions and numerical ordering consistency,
validates each exclusion, and reconstructs every incoming obligation for every
survivor over the full roster. `AuditedSelection` has private fields and no
`Deserialize` implementation. Its comparison API distinguishes assessed
verdicts, directions ruled out by the observed-gate bound, and uncomputed
directions. Scientific `Unresolved` remains distinct from an operationally
missing comparison, and is reported as a qualification on the asserted-edge
selection, consistently with exhaustive reduction. The audit checks the saved
evidence; it does not rerun every computational replication.

By default the plan also requires all numerical pair reports within S0 in every
context, including gate-rejected pairs, preserving the NCI Version 2 activation
profiles. `--survivors-only` omits that obligation and cannot be used for
operational finalization. The NCI adapter uses the same activation and regret
implementation as exhaustive finalization:

```sh
iris_nci_parameter_tournament finalize-version2-accelerated \
  --config config.json --bundle bundle --plan accelerated_plan \
  --results accelerated_results --selection accelerated_selection \
  --output version2 --threads 8
```

The accelerated output is **not** a complete `Reduction` or a
`CompletenessAudit`. It does not promise every context graph, every per-context
maximal set, the alternative strategy, threshold-profile graphs, induced views,
or revision composition. Complete graphs are reported only for contexts
explicitly executed exhaustively. Use the existing exhaustive workflow when
those outputs are required. No candidate screen or numerical replication
budget is changed by enabling this mode.

Independent metrics remain separate tournaments. If they are declared to be
independent opportunities to exclude a candidate, intersect their fixed-policy
survivor sets to define the eligible systems, then form one descending incoming
profile over metric, evaluation, and challenger; the strongest threat under
either metric controls. A rule requiring the same replacement direction to
survive every metric is different: first form the joint graph, using the
smallest metric-specific activation strength for each direction, and let that
graph's fixed-policy survivors define eligibility. Cross-metric choice therefore
belongs in a reporting layer that can verify the intended joint claim, not in
either single-metric reduction implicitly.

Changing only `gamma` requires no new pairwise model comparison when the stored
match reports contain every stage summary that can become eligible, as in the
current one-empirical-evaluation-per-context design. Changing the survival
floor, magnitude threshold, challenge order, condition set, resampling
procedure, or metric defines a different assessment rather than a threshold
profile.

The current CLI does not compute this survival-profile rule. It can apply a
user-declared `operational_tie_break` order after the evidential reduction;
without that declaration, `operational_choice` is null. A future auditable
implementation of the survival-profile layer should retain the evidential
outcome unchanged, write the directed activation strengths and compressed
threshold intervals, identify each finalist's binding incoming edge, and
populate `operational_choice` with
`operational_choice_is_non_evidential = true`. Suggested companion artifacts
are `directed_edge_strengths.csv`, `survival_threshold_profile.json`, and
`operational_selection.json`.

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

`selection.json` holds the primary evidential selection (the strategy named by
`selection_strategy`, default `candidate_conservative`) and
`alternate_selection.json` the other strategy; `per_evaluation_maximal.json`
records each evaluation's maximal set. Multiple survivors remain an unresolved
evidential candidate set. The organizer applies no domain-specific refinement;
`operational_choice` remains null unless the specification supplies an
`operational_tie_break` order. The two evidential strategies are defined and
contrasted in "Selection strategies" above, and the proposed generic secondary
reduction is described in "Operational choice from the survival-threshold
profile."

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


## Distributed accelerated execution

`accelerated distributed` adds four scheduler-neutral stages: `targets`, `merge`,
`complete`, and `finish`. They use `--bundle`, `--plan`, `--results`, `--receipts`,
`--output`, and `--shard-count`; workers additionally use `--shard-id` and
`--threads`. Completion and finish require `--survivors` pointing to the merge
output. The NCI cluster driver wires these commands into dependent Slurm arrays.

Targets are assigned round-robin in the first context's derived ordering, while
each target retains the full challenger roster in every context. An ordered
comparison belongs to the lower-potential target. Any exhaustive context sends
the entire target phase to shard zero to retain the existing full-roster SCC
fallback; remaining shards publish empty receipts. Target receipts bind the
plan, assignment, exclusions, and immutable comparison references.

Merge requires every receipt and independently audits survivor obligations.
Its intermediate certificate explicitly has `survivor_set` scope, even though
the unchanged parent plan requests complete operational inputs. It is validated
by the distributed merge/completion interfaces; the ordinary final selection
auditor rejects it as insufficient for that plan. Completion shards partition
unordered survivor pairs, reusing validated reports already present. Finish
requires all completion receipts and the complete certificate audit before
publishing the final selection. Existing local accelerated and exhaustive
interfaces remain available. Duplicate live workers for the same shard are
unsupported and fail closed on publication conflicts; resumption runs after the
previous worker has stopped.
