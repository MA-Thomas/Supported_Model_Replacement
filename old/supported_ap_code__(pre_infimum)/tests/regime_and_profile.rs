//! The regime seam and the prevalence profile.
//!
//! A profile is an optimization, not a different estimator. Replicate streams
//! are keyed on logical indices, so drawing once and reducing at every
//! prevalence must give bit-identical results to one run per prevalence.

use supported_ap::{
    BootstrapOptions, ClassCounts, ConditionalNullCalibrationOptions, Evaluation,
    FixedScoreResamplingRegime, NullDistributionStorage, PairedEvaluation, Parallelism,
    PermutationCount, PermutationOptions, ReferencePrevalence, ReplicateCount, ReplicateCounts,
    ReplicateRng, SupportOrder, SupportedApError,
};

fn evaluation() -> Evaluation {
    let scores: Vec<f64> = (0..120)
        .map(|index| ((index * 47 % 61) as f64) / 61.0)
        .collect();
    let labels: Vec<bool> = (0..120).map(|index| index % 5 == 0).collect();
    Evaluation::new(&scores, &labels).unwrap()
}

fn options(replicates: usize, seed: u64) -> BootstrapOptions {
    BootstrapOptions {
        replicates: ReplicateCount::new(replicates).unwrap(),
        seed,
        parallelism: Parallelism::Sequential,
        ..BootstrapOptions::default()
    }
}

fn prevalences() -> Vec<ReferencePrevalence> {
    [0.01, 0.05, 0.10, 0.25, 0.50]
        .into_iter()
        .map(|value| ReferencePrevalence::new(value).unwrap())
        .collect()
}

#[test]
fn profile_is_bit_identical_to_one_run_per_prevalence() {
    let evaluation = evaluation();
    let options = options(600, 4242);
    let sweep = prevalences();

    let profile = evaluation.supported_cnap_profile(&sweep, options).unwrap();
    assert_eq!(profile.len(), sweep.len());

    for (prevalence, from_profile) in sweep.iter().zip(&profile) {
        let alone = evaluation.supported_cnap(*prevalence, options).unwrap();
        assert_eq!(
            alone.supported().value(),
            from_profile.supported().value(),
            "supported effect diverged at prevalence {prevalence}"
        );
        assert_eq!(
            alone.replicate_mean().value(),
            from_profile.replicate_mean().value()
        );
        assert_eq!(
            alone.monte_carlo_standard_error(),
            from_profile.monte_carlo_standard_error()
        );
        assert_eq!(alone.replicate_cnaps(), from_profile.replicate_cnaps());
    }
}

#[test]
fn calibrated_profile_is_bit_identical_to_one_run_per_prevalence() {
    let evaluation = evaluation();
    let sweep = prevalences();
    let conditional_null_calibration = ConditionalNullCalibrationOptions {
        bootstrap: options(120, 7),
        permutation: PermutationOptions {
            permutations: PermutationCount::new(24).unwrap(),
            seed: 11,
            parallelism: Parallelism::Sequential,
            storage: NullDistributionStorage::SummaryOnly,
        },
    };

    let profile = evaluation
        .conditionally_calibrated_supported_cnap_profile(&sweep, conditional_null_calibration)
        .unwrap();
    for (prevalence, from_profile) in sweep.iter().zip(&profile) {
        let alone = evaluation
            .conditionally_calibrated_supported_cnap(*prevalence, conditional_null_calibration)
            .unwrap();
        assert_eq!(
            alone.calibrated().value(),
            from_profile.calibrated().value()
        );
        assert_eq!(
            alone.conditional_null().value(),
            from_profile.conditional_null().value()
        );
        assert_eq!(alone.p_value().value(), from_profile.p_value().value());
    }
}

#[test]
fn estimated_supported_superiority_profile_matches_separate_runs() {
    let model_a: Vec<f64> = (0..80)
        .map(|index| ((index * 31 % 47) as f64) / 47.0)
        .collect();
    let model_b: Vec<f64> = (0..80)
        .map(|index| ((index * 19 % 43) as f64) / 43.0)
        .collect();
    let labels: Vec<bool> = (0..80).map(|index| index % 4 == 0).collect();
    let paired = PairedEvaluation::new(&model_a, &model_b, &labels).unwrap();
    let sweep = prevalences();
    let bootstrap = options(80, 17);

    let profile = paired
        .estimated_supported_superiority_profile(&sweep, bootstrap)
        .unwrap();
    for (prevalence, from_profile) in sweep.iter().zip(profile) {
        let alone = paired
            .estimated_supported_superiority(*prevalence, bootstrap)
            .unwrap();
        assert_eq!(alone, from_profile);
    }
}

#[test]
fn an_empty_profile_is_rejected() {
    let evaluation = evaluation();
    assert!(matches!(
        evaluation.supported_cnap_profile(&[], options(64, 1)),
        Err(SupportedApError::EmptyPrevalenceProfile)
    ));
}

/// A regime that returns the observed evaluation unchanged in every replicate.
/// Its replication distribution is degenerate, so the supported effect equals
/// the observed effect and the Monte Carlo error is zero.
struct NoVariation {
    positive_blocks: Vec<usize>,
    negative_blocks: Vec<usize>,
    threshold_count: usize,
    counts: ClassCounts,
}

impl FixedScoreResamplingRegime for NoVariation {
    type Scratch = ();

    fn new_scratch(&self) -> Self::Scratch {}

    fn threshold_count(&self) -> usize {
        self.threshold_count
    }

    fn class_counts(&self) -> ClassCounts {
        self.counts
    }

    fn draw(
        &self,
        _replicate: usize,
        _rng: &mut ReplicateRng<'_>,
        _scratch: &mut Self::Scratch,
        out: &mut ReplicateCounts,
    ) -> Result<(), SupportedApError> {
        out.reset(self.counts);
        for &block in &self.positive_blocks {
            out.add_positive(block)?;
        }
        for &block in &self.negative_blocks {
            out.add_negative(block)?;
        }
        Ok(())
    }
}

#[test]
fn a_custom_regime_with_no_variation_reproduces_the_observed_effect() {
    let scores = [0.9, 0.75, 0.75, 0.4, 0.2, 0.05];
    let labels = [true, true, false, true, false, false];
    let evaluation = Evaluation::new(&scores, &labels).unwrap();
    let prevalence = ReferencePrevalence::new(0.2).unwrap();

    let regime = NoVariation {
        positive_blocks: evaluation.positive_block_indices().to_vec(),
        negative_blocks: evaluation.negative_block_indices().to_vec(),
        threshold_count: evaluation.threshold_count(),
        counts: evaluation.class_counts(),
    };

    let estimates = evaluation
        .supported_cnap_with_fixed_score_regime(&regime, &[prevalence], options(32, 3))
        .unwrap();
    let estimate = &estimates[0];

    // The supported effect is exact: identical replicates take the degenerate
    // path, which returns the common value rather than a weighted sum.
    let observed = evaluation.cnap(prevalence).value();
    assert_eq!(estimate.supported().value(), observed);

    // The penalty and the standard error are not exact. Both derive from a
    // running sum over the replicates, and summing a non-dyadic value B times
    // accumulates about one unit in the last place, so they land near zero
    // rather than on it.
    assert!(
        estimate.support_penalty().abs() < 1e-12,
        "penalty {} should be zero up to accumulation error",
        estimate.support_penalty()
    );
    assert!(
        estimate.monte_carlo_standard_error() < 1e-12,
        "standard error {} should be zero up to accumulation error",
        estimate.monte_carlo_standard_error()
    );
}

/// A regime that under-reports its own units must be caught rather than
/// producing a quietly wrong average precision.
struct Miscounting {
    counts: ClassCounts,
}

impl FixedScoreResamplingRegime for Miscounting {
    type Scratch = ();
    fn new_scratch(&self) -> Self::Scratch {}
    fn threshold_count(&self) -> usize {
        2
    }
    fn class_counts(&self) -> ClassCounts {
        self.counts
    }
    fn draw(
        &self,
        _replicate: usize,
        _rng: &mut ReplicateRng<'_>,
        _scratch: &mut Self::Scratch,
        out: &mut ReplicateCounts,
    ) -> Result<(), SupportedApError> {
        // Declares the observed counts but records one positive unit only.
        out.reset(self.counts);
        out.add_positive(0)
    }
}

struct WrongLayout {
    counts: ClassCounts,
}

impl FixedScoreResamplingRegime for WrongLayout {
    type Scratch = ();

    fn new_scratch(&self) -> Self::Scratch {}

    fn threshold_count(&self) -> usize {
        3
    }

    fn class_counts(&self) -> ClassCounts {
        self.counts
    }

    fn draw(
        &self,
        _replicate: usize,
        _rng: &mut ReplicateRng<'_>,
        _scratch: &mut Self::Scratch,
        _out: &mut ReplicateCounts,
    ) -> Result<(), SupportedApError> {
        Ok(())
    }
}

#[test]
fn a_custom_regime_with_an_incompatible_layout_is_rejected() {
    let evaluation = Evaluation::new(&[0.9, 0.1], &[true, false]).unwrap();
    let prevalence = ReferencePrevalence::new(0.5).unwrap();
    let regime = WrongLayout {
        counts: evaluation.class_counts(),
    };

    assert!(matches!(
        evaluation.supported_cnap_with_fixed_score_regime(&regime, &[prevalence], options(8, 1)),
        Err(SupportedApError::RegimeThresholdCountMismatch {
            evaluation: 2,
            regime: 3
        })
    ));
}

struct InvalidBlock {
    counts: ClassCounts,
}

impl FixedScoreResamplingRegime for InvalidBlock {
    type Scratch = ();

    fn new_scratch(&self) -> Self::Scratch {}

    fn threshold_count(&self) -> usize {
        2
    }

    fn class_counts(&self) -> ClassCounts {
        self.counts
    }

    fn draw(
        &self,
        _replicate: usize,
        _rng: &mut ReplicateRng<'_>,
        _scratch: &mut Self::Scratch,
        out: &mut ReplicateCounts,
    ) -> Result<(), SupportedApError> {
        out.reset(self.counts);
        out.add_positive(2)?;
        out.add_negative(0)
    }
}

#[test]
fn an_out_of_range_regime_block_is_an_error_not_a_panic() {
    let evaluation = Evaluation::new(&[0.9, 0.1], &[true, false]).unwrap();
    let prevalence = ReferencePrevalence::new(0.5).unwrap();
    let regime = InvalidBlock {
        counts: evaluation.class_counts(),
    };

    assert!(matches!(
        evaluation.supported_cnap_with_fixed_score_regime(&regime, &[prevalence], options(8, 1)),
        Err(SupportedApError::RegimeBlockOutOfRange {
            block: 2,
            threshold_count: 2
        })
    ));
}

struct InvalidRandomRequest {
    counts: ClassCounts,
    invalid_probability: bool,
}

impl FixedScoreResamplingRegime for InvalidRandomRequest {
    type Scratch = ();

    fn new_scratch(&self) -> Self::Scratch {}

    fn threshold_count(&self) -> usize {
        2
    }

    fn class_counts(&self) -> ClassCounts {
        self.counts
    }

    fn draw(
        &self,
        _replicate: usize,
        rng: &mut ReplicateRng<'_>,
        _scratch: &mut Self::Scratch,
        _out: &mut ReplicateCounts,
    ) -> Result<(), SupportedApError> {
        if self.invalid_probability {
            rng.boolean(f64::NAN)?;
        } else {
            rng.index(0)?;
        }
        Ok(())
    }
}

#[test]
fn an_empty_random_index_range_is_an_error_not_a_panic() {
    let evaluation = Evaluation::new(&[0.9, 0.1], &[true, false]).unwrap();
    let prevalence = ReferencePrevalence::new(0.5).unwrap();
    let regime = InvalidRandomRequest {
        counts: evaluation.class_counts(),
        invalid_probability: false,
    };

    assert_eq!(
        evaluation
            .supported_cnap_with_fixed_score_regime(&regime, &[prevalence], options(8, 1))
            .unwrap_err(),
        SupportedApError::InvalidRandomBound
    );
}

#[test]
fn an_invalid_bernoulli_probability_is_an_error_not_a_panic() {
    let evaluation = Evaluation::new(&[0.9, 0.1], &[true, false]).unwrap();
    let prevalence = ReferencePrevalence::new(0.5).unwrap();
    let regime = InvalidRandomRequest {
        counts: evaluation.class_counts(),
        invalid_probability: true,
    };

    assert!(matches!(
        evaluation.supported_cnap_with_fixed_score_regime(&regime, &[prevalence], options(8, 1)),
        Err(SupportedApError::InvalidProbability { value }) if value.is_nan()
    ));
}

#[test]
fn a_regime_that_miscounts_its_units_is_rejected() {
    let evaluation = Evaluation::new(&[0.9, 0.1], &[true, false]).unwrap();
    let prevalence = ReferencePrevalence::new(0.5).unwrap();
    let regime = Miscounting {
        counts: evaluation.class_counts(),
    };
    assert!(matches!(
        evaluation.supported_cnap_with_fixed_score_regime(&regime, &[prevalence], options(8, 1)),
        Err(SupportedApError::RegimeCountMismatch { .. })
    ));
}

#[test]
fn support_order_is_validated_against_the_replicate_count() {
    let evaluation = evaluation();
    let prevalence = ReferencePrevalence::new(0.1).unwrap();
    let bad = BootstrapOptions {
        support_order: SupportOrder::new(50).unwrap(),
        ..options(10, 1)
    };
    assert!(matches!(
        evaluation.supported_cnap(prevalence, bad),
        Err(SupportedApError::SupportOrderExceedsSample { .. })
    ));
}
