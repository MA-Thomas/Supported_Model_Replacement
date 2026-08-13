# supported-ap

`supported_ap` implements non-interpolated prior-standardized average precision
at a declared reference prevalence, chance-normalized average precision
(CNAP), supported CNAP, and conditionally calibrated supported CNAP.

Conditionally calibrated supported CNAP is the primary reported quantity. It
places the supported value on a scale from its conditional permutation null to
perfect separation.

The canonical fixed-model regime:

- equal scores enter as one threshold block;
- bootstrap replicates independently resample the observed positive and
  negative units; same-design analysis preserves both observed class counts;
- supported CNAP uses every distinct pair of bootstrap effects;
- conditional-null calibration permutes labels across observations and reruns the complete
  bootstrap estimator for every permutation;
- the upper calibration endpoint is one, corresponding to perfect separation;
- estimated supported superiority uses the same resampled units for both models
  and may declare prospective positive and negative replication counts.

The implementation precomputes score-tie blocks and accumulates bootstrap
multiplicities directly into them, so replicates require no sorting. The
support estimator sorts only the scalar replicate effects.

## Independent Python validation

`python_tests/` exercises the compiled CLI as a black box. Deterministic
prior-standardized AP and CNAP outputs are compared with scikit-learn's
non-interpolated weighted average precision over fixed edge cases and
randomized datasets. Small stratified bootstrap and conditional-permutation
problems are also enumerated exhaustively in Python and used as independent
targets for the reported Monte Carlo estimates.

Run the extensive profile from this directory with:

```text
sh scripts/test-python-differential.sh extensive
```

See `python_tests/README.md` for the smoke and stress profiles and the exact
weighting used by the scikit-learn oracle.

## Domain interpretation study

`domain_study/` builds an eight-figure pedagogical story around the UCI Bank
Marketing dataset: prevalence transport, matched point estimates with unequal
replication support, design-specific chance anchors, conditional-null
re-anchoring, repeated rare-outcome evaluations, and paired supported model
superiority.

Run the full reproducible analysis and pedagogical plots with:

```text
sh scripts/run-bank-marketing-study.sh publication
```

The generated report, raw CLI JSON, tables, predictions, and figures are placed
under `domain_study/outputs/`. See `domain_study/README.md` for the design and
interpretation limits.

## The reporting ladder

Report four quantities. Each is the one above it after a single adjustment,
and each adjustment is the gap between neighbours.

| Quantity | Rust access | Reads as |
| --- | --- | --- |
| Observed CNAP | `result.estimate().observed().cnap()` | what this evaluation gave |
| Expected across replications | `result.estimate().replicate_mean()` | expected performance across replications |
| Supported CNAP | `result.estimate().supported()` | the level retained by `K` replications |
| Conditionally calibrated supported CNAP (headline) | `result.calibrated()` | position between conditional chance and perfect separation |

The mean belongs in the report because the support operator subtracts, and a
difference cannot report its operands. A design returning 0.9 and 0.1 with
equal probability and a design returning 0.3 every time both have supported
CNAP of 0.30. Their means, 0.50 and 0.30, separate them immediately.

Report `result.conditional_null()` and `result.p_value()` alongside the headline. They
describe the conditional chance distribution used for calibration.

## Example

```rust
use supported_ap::{BootstrapOptions, Evaluation, ReferencePrevalence, ReplicateCount};

let scores = [0.9, 0.7, 0.7, 0.2];
let labels = [true, true, false, false];
let evaluation = Evaluation::new(&scores, &labels)?;
let prevalence = ReferencePrevalence::new(0.5)?;

let observed = evaluation.cnap(prevalence);
let estimate = evaluation.supported_cnap(
    prevalence,
    BootstrapOptions {
        replicates: ReplicateCount::new(2_000)?,
        seed: 42,
        ..BootstrapOptions::default()
    },
)?;

assert!(observed.value() <= 1.0);
assert!(estimate.supported().value() <= 1.0);
assert!(estimate.monte_carlo_standard_error() >= 0.0);

# Ok::<(), supported_ap::SupportedApError>(())
```

## Conditional-null calibration

### The null

Chance is a property of a design, so it is measured rather than assumed.
Non-interpolated AP is upward biased under random ranking, while the minimum
operator imposes a support penalty. The two can leave a design-dependent
residual of either sign.

For each permutation `m = 1, ..., M`:

1. Permute outcome labels across observations, preserving the score multiset,
   its tie structure, and both class counts.
2. Run the same `B`-replicate stratified bootstrap on the permuted evaluation.
3. Apply the same order-`K` support operator.

The null is conditional on the realized score vector and its ties. It is
appropriate when the scores were developed independently of the outcomes being
permuted, as with held-out test data. It does not repeat model development.

The null is the mean of those `M` estimates. The upper-tail p-value
uses the plus-one correction:

```text
p = (1 + count(null[m] >= S_hat)) / (M + 1)
```

`NullDistributionStorage::Full` retains the individual values, which makes
`result.conditional_null_quantile(p)` return the requested quantile.
Distribution access returns `None` when only the summary was stored. Quantile
access remains fallible because it requires the full distribution and a valid
probability. The mean alone is the least informative summary of the null.

### Calibration

```text
S_hat_cal = (S_hat - S_hat_null) / (1 - S_hat_null)
```

The upper endpoint is the substantive criterion of perfect separation, whose
CNAP and supported CNAP are one. Ties can prevent a realized score vector from
attaining that endpoint. They remain visible in its observed and supported
performance rather than changing the scale.

A positive denominator does not make the headline positive. The sign follows
the numerator, and `result.is_above_conditional_null()` checks the comparison
directly. Below that anchor the value is an extrapolation rather than a
position on the measured scale; `result.is_extrapolation()` flags it. Report
such a result as at or below measured chance and read its direction from the
signed observed CNAP, whose negative region is ordered and bounded. Near the
anchor, use the permutation p-value rather than the sign of one Monte Carlo
difference. The command-line report always includes conditionally calibrated
supported CNAP and its Monte Carlo error. It marks whether the supported value
clears the conditional null and whether the calibrated value is an
extrapolation, leaving the downstream decision to the consumer.

## Calibrated example

```rust
use supported_ap::{
    BootstrapOptions, ConditionalNullCalibrationOptions, Evaluation, NullDistributionStorage,
    PermutationCount, PermutationOptions, ReferencePrevalence, ReplicateCount,
};

let scores = [0.91, 0.74, 0.74, 0.52, 0.31, 0.08];
let labels = [true, true, false, true, false, false];
let evaluation = Evaluation::new(&scores, &labels)?;
let prevalence = ReferencePrevalence::new(0.10)?;

let result = evaluation.conditionally_calibrated_supported_cnap(
    prevalence,
    ConditionalNullCalibrationOptions {
        bootstrap: BootstrapOptions {
            replicates: ReplicateCount::new(5_000)?,
            seed: 7,
            ..BootstrapOptions::default()
        },
        permutation: PermutationOptions {
            permutations: PermutationCount::new(500)?,
            seed: 11,
            storage: NullDistributionStorage::SummaryOnly,
            ..PermutationOptions::default()
        },
    },
)?;

println!("conditionally calibrated supported CNAP: {}", result.calibrated());
println!("expected replication performance: {}", result.estimate().replicate_mean());
println!("supported CNAP: {}", result.estimate().supported());
println!("conditional null: {}", result.conditional_null());
println!("permutation p-value: {}", result.p_value());

if result.is_extrapolation() {
    println!("at or below measured chance");
}

# Ok::<(), supported_ap::SupportedApError>(())
```

## Monte Carlo error

Supported CNAP is a U-statistic of degree `K`, so its Monte Carlo standard error
follows from the Hoeffding projection and costs one prefix sum over an array
the estimator has already sorted. The null's error is the usual `sd / sqrt(M)`,
and the headline's follows by the delta method through the calibration.

```text
result.estimate().monte_carlo_standard_error()          // finite B
result.conditional_null_monte_carlo_standard_error()    // finite M
result.monte_carlo_standard_error()                     // calibrated value
```

These describe numerical error only. The statistical error from estimating the
replication distribution out of one evaluation is governed by `n_+` and `n_-`,
and no increase in `B` reduces it. A nested bootstrap measures that: resample
units to form outer datasets and run the complete estimator inside each.

## The lower tail

`estimate.survival_level(gamma)` returns the level retained with frequency
`gamma`, the sample analogue of `Q(1 - gamma^(1/K))`. Two numbers do not
determine a distribution, so the ladder can agree while the tails differ. Add
this summary whenever the lower tail is the decision, which the sparse-positive
regime makes the common case.

## Estimated supported superiority

Marginal supported CNAP values cannot be differenced to obtain estimated
supported superiority. The support operator is superadditive on gains, so

```text
S_K(A) - S_K(B) >= S_K(A - B)
```

always, with equality exactly when B's performance and A's advantage are
comonotone. Differencing marginals therefore never understates the paired
quantity. `PairedEvaluation::estimated_supported_superiority` evaluates both
models on the same resampled units and applies the support operator to their
within-replication CNAP differences. A positive value reports estimated
supported superiority of model A. By default, each replication has the
observed positive and negative counts. For a prospective evaluation, use
`estimated_supported_superiority_with_replication_counts` (or its profile
counterpart) to declare different replication counts. The result reports
observed and replication counts separately.

Its Monte Carlo standard error is a numerical diagnostic for finite `B`, not an
inferential standard error. Population-level inference requires a separate
outer uncertainty procedure.

## Reference-prevalence sweeps

Threshold-block counts do not depend on the reference prevalence, so a
replicate is drawn once and evaluated at every prevalence in a sweep:

```text
evaluation.supported_cnap_profile(&prevalences, options)
evaluation.conditionally_calibrated_supported_cnap_profile(&prevalences, options)
```

A profile is an optimization, not a different estimator. Replicate streams are
keyed on logical indices, so a profile returns bit-identical results to one run
per prevalence while performing one set of draws instead of one per value. The
`M * B` permutation draws are shared the same way, which matters because the
null has to be recomputed at every prevalence for which a calibrated value is
reported.

Because the replicates are shared, the comparison across prevalences is paired
rather than independently noisy. Model orderings can reverse across reference
prevalences, so a claim that one model has greater supported performance should
state the prevalence or the range over which the ordering holds.

`PairedEvaluation::estimated_supported_superiority_profile` provides the
corresponding shared-draw sweep.

## Declaring a regime

Supported CNAP is defined relative to a declared fixed-score resampling regime.
The replication mechanism is the `FixedScoreResamplingRegime` trait:

```text
trait FixedScoreResamplingRegime {
    type Scratch: Send;
    fn new_scratch(&self) -> Self::Scratch;
    fn threshold_count(&self) -> usize;
    fn class_counts(&self) -> ClassCounts;
    fn draw(
        &self,
        replicate,
        rng,
        scratch,
        out: &mut ReplicateCounts,
    ) -> Result<(), SupportedApError>;
}
```

An implementation fills counts for the original score blocks and declares the
class counts of its replicate. It never sees the reference prevalence, which
makes sweep sharing possible.
Stream derivation stays inside the crate, so `rand` does not appear in the
trait and results stay independent of scheduling. A regime that records a
different number of units than it declares is rejected with
`SupportedApError::RegimeCountMismatch` rather than producing a quietly wrong
average precision. Drawing, random requests, and block-count updates are
fallible; invalid bounds, probabilities, and block indices therefore return
`SupportedApError` instead of panicking. An incompatible threshold layout is
rejected before resampling begins. `Evaluation::block_indices`,
`positive_block_indices`, and `negative_block_indices` expose the validated
layout needed to build a custom regime without recreating tie handling.

`CanonicalFixedModel` is the only mechanism shipped here. Use a custom one
through `Evaluation::supported_cnap_with_fixed_score_regime`.

Conditional-null calibration uses the canonical fixed-score bootstrap. A
custom regime yields supported CNAP and its Monte Carlo error; its matching
null must use the same mechanism. Regimes that retrain a model or change the
score vector require a higher-level simulation outside this block-count
interface.

## Support order

`BootstrapOptions::support_order` sets how many replications a claim must
survive. `K = 2` is the default and the smallest count that expresses a
replication criterion. `S_K` is decreasing in `K` for any nondegenerate
distribution, so a larger `K` states a stricter claim and must be prespecified.

## Command line

```text
supported_ap_metrics --input predictions.csv --output supported_ap_metrics.json \
    --label-col label --reference-prevalence 0.10 --score-prefix score_
```

`--reference-prevalence` is required. It accepts `observed`, a value in
`(0, 1)`, or a comma-separated sweep, because model orderings can reverse
across reference prevalences and the null is recomputed at each value. A sweep
reuses one set of replicates, so it costs one resampling pass rather than one
per value.

Input is streamed and projected to the label column and the selected score
columns, so memory tracks the retained columns rather than the file size.
Quoted fields spanning newlines are handled, and a leading byte order mark is
stripped from the header.

Every selected model is evaluated on the same rows by default, so the results
are comparable and the shared resample plan holds: equal class counts and equal
seeds draw the same units for every model. `--allow-ragged-rows` disables this
and lets each model keep its own valid rows, which makes the models
incomparable. `--paired-baseline NAME` adds estimated supported superiority of every
other model over that one. `--replication-positive-count` and
`--replication-negative-count` declare a prospective paired design; either
count defaults to its observed value when omitted.

`--fail-on-error` makes the process exit non-zero when any result failed.
Without it a run in which every model errored still exits zero, with the
reasons recorded in the JSON. JSON schema version 8 contains no null-valued
fields: successful result objects have `status: "ok"` and all computed fields;
failed objects have `status: "error"` and an error message, with unavailable
metric fields omitted. Automatic threading and row handling use explicit
`thread_policy.mode` and `row_mode` values. In shared-row mode,
`rows.skipped_any_score` reports exclusions caused by any selected model while
each metric's `n_skipped_score` reports invalid values in that model alone.

## Parallel execution

```toml
[dependencies]
supported-ap = { version = "0.6", features = ["parallel"] }
```

`Parallelism::Auto` uses Rayon when the feature is enabled and runs
sequentially otherwise. `Parallelism::Threads(n)` requests a private Rayon
thread pool and returns an error if the feature is unavailable. Seeds are
derived from logical permutation and replicate indices, so results do not
depend on scheduling or thread count.

Conditional-null calibration parallelizes permutations. Its bootstrap loop is
sequential within each permutation, avoiding nested thread pools.

## Cost

Conditional-null calibration performs `M * B` resampling steps for the null,
plus `B` for the empirical supported value. Each rebuilds the threshold counts
and sweeps them once, at `O(n)`. Sorting contributes `O(M B log B)`. The
resampling sweep dominates whenever `n > log B`, which covers every evaluation
large enough to be worth reporting.

## Scope

Only the canonical fixed-model regime ships. The conditional null requires
outcomes that were not used to develop the fixed scores. Clustered units and
hierarchical sampling need a suitable `FixedScoreResamplingRegime` and matching
null. Repeated training belongs in a higher-level simulation because it changes
the score vector.

Licensed under the MIT license.
