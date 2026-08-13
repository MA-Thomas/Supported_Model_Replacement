use crate::support::{quantile, survival_from_cnaps};
use crate::{
    CalibratedSupportedCnap, ChanceNormalizedDifference, ClassCounts, Cnap, PermutationCount,
    PermutationPValue, PriorStandardizedAp, ReferencePrevalence, SupportOrder, SupportedApError,
    SupportedCnap, SupportedDifference,
};

/// The two deterministic metrics calculated on the observed evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct ObservedMetrics {
    prior_standardized_ap: PriorStandardizedAp,
    cnap: Cnap,
}

impl ObservedMetrics {
    pub(crate) const fn new(prior_standardized_ap: PriorStandardizedAp, cnap: Cnap) -> Self {
        Self {
            prior_standardized_ap,
            cnap,
        }
    }

    /// Non-interpolated prior-standardized AP at the reference prevalence.
    #[inline]
    pub const fn prior_standardized_ap(self) -> PriorStandardizedAp {
        self.prior_standardized_ap
    }

    /// Chance-normalized average precision on the observed evaluation.
    #[inline]
    pub const fn cnap(self) -> Cnap {
        self.cnap
    }
}

/// Result of the canonical fixed-model bootstrap estimator.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct SupportedCnapEstimate {
    observed: ObservedMetrics,
    supported: SupportedCnap,
    replicate_mean: Cnap,
    support_penalty: f64,
    standard_error: f64,
    replicate_cnaps: Vec<Cnap>,
    counts: ClassCounts,
    prevalence: ReferencePrevalence,
    support_order: SupportOrder,
}

impl SupportedCnapEstimate {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        observed: ObservedMetrics,
        supported: SupportedCnap,
        replicate_mean: Cnap,
        standard_error: f64,
        replicate_cnaps: Vec<Cnap>,
        counts: ClassCounts,
        prevalence: ReferencePrevalence,
        support_order: SupportOrder,
    ) -> Self {
        let support_penalty = replicate_mean.value() - supported.value();
        Self {
            observed,
            supported,
            replicate_mean,
            support_penalty,
            standard_error,
            replicate_cnaps,
            counts,
            prevalence,
            support_order,
        }
    }

    /// The metrics calculated directly on the observed evaluation.
    #[inline]
    pub const fn observed(&self) -> ObservedMetrics {
        self.observed
    }

    /// Supported CNAP, the level retained by `K` replications of the declared design.
    #[inline]
    pub const fn supported(&self) -> SupportedCnap {
        self.supported
    }

    /// Expected performance across replications, the mean of the replicate effects.
    ///
    /// Report this with the supported effect. The support operator subtracts
    /// the support penalty from this mean, and a difference cannot report its
    /// operands: a stable mediocre design and an unstable good one can return
    /// the same supported effect.
    #[inline]
    pub const fn replicate_mean(&self) -> Cnap {
        self.replicate_mean
    }

    /// Expected performance across replications minus the `K`-replication supported effect.
    ///
    /// For `K = 2`, this equals half the expected absolute disagreement between
    /// two replications. The quantity is non-negative in exact arithmetic. It is
    /// calculated as the difference of two accumulated sums, so a degenerate
    /// replication distribution can place it within one unit in the last place
    /// of zero.
    #[must_use]
    #[inline]
    pub const fn support_penalty(&self) -> f64 {
        self.support_penalty
    }

    /// The Monte Carlo standard error of the supported effect at this `B`.
    ///
    /// This describes numerical error from a finite replicate count. It does
    /// not describe the statistical error from estimating the replication
    /// distribution out of one evaluation, which is governed by the positive
    /// and negative counts and which a larger `B` cannot reduce.
    #[must_use]
    #[inline]
    pub const fn monte_carlo_standard_error(&self) -> f64 {
        self.standard_error
    }

    /// The level retained by `K` replications with frequency `gamma`.
    ///
    /// A low quantile of the same distribution the supported effect averages.
    /// Report it when the lower tail is the decision, which the sparse-positive
    /// regime makes the common case. It is noisier than the supported effect
    /// and needs a larger `B`.
    ///
    /// # Errors
    ///
    /// Returns an error unless `gamma` is finite and in `(0, 1]`.
    pub fn survival_level(&self, gamma: f64) -> Result<f64, SupportedApError> {
        survival_from_cnaps(&self.replicate_cnaps, gamma, self.support_order)
    }

    /// The replicate effects, in generation order.
    #[inline]
    pub fn replicate_cnaps(&self) -> &[Cnap] {
        &self.replicate_cnaps
    }

    /// The number of replications the criterion required.
    #[inline]
    pub const fn support_order(&self) -> SupportOrder {
        self.support_order
    }

    /// The observed positive and negative counts.
    #[inline]
    pub const fn class_counts(&self) -> ClassCounts {
        self.counts
    }

    /// The reference prevalence the effects were evaluated at.
    #[inline]
    pub const fn reference_prevalence(&self) -> ReferencePrevalence {
        self.prevalence
    }
}

/// Result of the conditional-null calibration pipeline.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct CalibratedSupportedCnapEstimate {
    estimate: SupportedCnapEstimate,
    conditional_null: SupportedCnap,
    calibrated: CalibratedSupportedCnap,
    p_value: PermutationPValue,
    conditional_null_standard_error: f64,
    calibrated_standard_error: f64,
    permutations: PermutationCount,
    conditional_null_distribution: Option<Vec<SupportedCnap>>,
}

impl CalibratedSupportedCnapEstimate {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        estimate: SupportedCnapEstimate,
        conditional_null: SupportedCnap,
        calibrated: CalibratedSupportedCnap,
        p_value: PermutationPValue,
        conditional_null_standard_error: f64,
        calibrated_standard_error: f64,
        permutations: PermutationCount,
        conditional_null_distribution: Option<Vec<SupportedCnap>>,
    ) -> Self {
        Self {
            estimate,
            conditional_null,
            calibrated,
            p_value,
            conditional_null_standard_error,
            calibrated_standard_error,
            permutations,
            conditional_null_distribution,
        }
    }

    /// The uncalibrated estimate the headline score is built from.
    #[inline]
    pub const fn estimate(&self) -> &SupportedCnapEstimate {
        &self.estimate
    }

    /// The supported-CNAP level under the conditional permutation null.
    #[inline]
    pub const fn conditional_null(&self) -> SupportedCnap {
        self.conditional_null
    }

    /// Conditionally calibrated supported CNAP, the headline score.
    ///
    /// The position of the supported effect between the measured conditional
    /// chance level and perfect separation.
    #[inline]
    pub const fn calibrated(&self) -> CalibratedSupportedCnap {
        self.calibrated
    }

    /// The upper-tail permutation p-value with the plus-one correction.
    #[inline]
    pub const fn p_value(&self) -> PermutationPValue {
        self.p_value
    }

    /// The Monte Carlo standard error of the conditional null.
    #[must_use]
    #[inline]
    pub const fn conditional_null_monte_carlo_standard_error(&self) -> f64 {
        self.conditional_null_standard_error
    }

    /// The Monte Carlo standard error of the headline score.
    ///
    /// Obtained by the delta method through conditional-null calibration,
    /// treating the supported effect and conditional null as independent
    /// because they draw from separate streams.
    #[must_use]
    #[inline]
    pub const fn monte_carlo_standard_error(&self) -> f64 {
        self.calibrated_standard_error
    }

    /// The permutation count used for the null.
    #[inline]
    pub const fn permutations(&self) -> PermutationCount {
        self.permutations
    }

    /// The retained permutation-level estimates.
    ///
    /// Returns `None` when calibration used
    /// [`NullDistributionStorage::SummaryOnly`](crate::NullDistributionStorage).
    #[must_use]
    #[inline]
    pub fn conditional_null_distribution(&self) -> Option<&[SupportedCnap]> {
        self.conditional_null_distribution.as_deref()
    }

    /// A quantile of the retained permutation-level estimates.
    ///
    /// The mean alone is the least informative summary of the null. Requires
    /// [`NullDistributionStorage::Full`](crate::NullDistributionStorage).
    ///
    /// # Errors
    ///
    /// Returns an error when the distribution was not stored or `probability`
    /// is not finite and in `[0, 1]`.
    pub fn conditional_null_quantile(&self, probability: f64) -> Result<f64, SupportedApError> {
        let values: Vec<f64> = self
            .conditional_null_distribution()
            .ok_or(SupportedApError::ConditionalNullDistributionNotStored)?
            .iter()
            .map(|value| value.value())
            .collect();
        quantile(&values, probability)
    }

    /// Whether the supported effect clears its measured chance level.
    ///
    /// The comparison is strict. The calibrated score is on its measured scale
    /// only when this holds.
    #[must_use]
    #[inline]
    pub fn is_above_conditional_null(&self) -> bool {
        self.estimate.supported().value() > self.conditional_null.value()
    }

    /// Whether the headline score falls below its lower anchor.
    ///
    /// Such a value is an extrapolation rather than a position on the measured
    /// scale. Report the result as at or below measured chance and read its
    /// direction from the signed observed effect.
    #[must_use]
    #[inline]
    pub fn is_extrapolation(&self) -> bool {
        !self.is_above_conditional_null()
    }
}

/// Estimated supported superiority from paired fixed-model bootstrap resampling.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct SupportedSuperiorityEstimate {
    observed_difference: ChanceNormalizedDifference,
    estimated_supported_superiority: SupportedDifference,
    replicate_mean_difference: ChanceNormalizedDifference,
    support_penalty: f64,
    standard_error: f64,
    replicate_differences: Vec<ChanceNormalizedDifference>,
    observed_counts: ClassCounts,
    replication_counts: ClassCounts,
    prevalence: ReferencePrevalence,
    support_order: SupportOrder,
}

impl SupportedSuperiorityEstimate {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        observed_difference: ChanceNormalizedDifference,
        estimated_supported_superiority: SupportedDifference,
        replicate_mean_difference: ChanceNormalizedDifference,
        standard_error: f64,
        replicate_differences: Vec<ChanceNormalizedDifference>,
        observed_counts: ClassCounts,
        replication_counts: ClassCounts,
        prevalence: ReferencePrevalence,
        support_order: SupportOrder,
    ) -> Self {
        let support_penalty =
            replicate_mean_difference.value() - estimated_supported_superiority.value();
        Self {
            observed_difference,
            estimated_supported_superiority,
            replicate_mean_difference,
            support_penalty,
            standard_error,
            replicate_differences,
            observed_counts,
            replication_counts,
            prevalence,
            support_order,
        }
    }

    /// The within-evaluation difference on the observed data.
    #[inline]
    pub const fn observed_difference(&self) -> ChanceNormalizedDifference {
        self.observed_difference
    }

    /// The estimated supported superiority of model A over model B.
    ///
    /// Positive values favor model A. Marginal supported effects cannot be
    /// differenced to obtain this paired estimate.
    #[inline]
    pub const fn estimated_supported_superiority(&self) -> SupportedDifference {
        self.estimated_supported_superiority
    }

    /// Expected advantage across replications, the mean of the replicate differences.
    #[inline]
    pub const fn replicate_mean_difference(&self) -> ChanceNormalizedDifference {
        self.replicate_mean_difference
    }

    /// Expected advantage across replications minus its `K`-replication supported value.
    ///
    /// For `K = 2`, this equals half the expected absolute disagreement between
    /// two independently replicated advantages.
    #[must_use]
    #[inline]
    pub const fn support_penalty(&self) -> f64 {
        self.support_penalty
    }

    /// The Monte Carlo standard error of the supported difference.
    #[must_use]
    #[inline]
    pub const fn monte_carlo_standard_error(&self) -> f64 {
        self.standard_error
    }

    /// The replicate differences, in generation order.
    #[inline]
    pub fn replicate_differences(&self) -> &[ChanceNormalizedDifference] {
        &self.replicate_differences
    }

    /// The observed positive and negative counts used to estimate the
    /// empirical class-conditional score distributions.
    #[inline]
    pub const fn observed_class_counts(&self) -> ClassCounts {
        self.observed_counts
    }

    /// The positive and negative counts declared for each prospective
    /// replication.
    #[inline]
    pub const fn replication_class_counts(&self) -> ClassCounts {
        self.replication_counts
    }

    /// The observed positive and negative counts.
    ///
    /// This is a compatibility alias for [`Self::observed_class_counts`].
    #[inline]
    pub const fn class_counts(&self) -> ClassCounts {
        self.observed_counts
    }

    /// The reference prevalence the differences were evaluated at.
    #[inline]
    pub const fn reference_prevalence(&self) -> ReferencePrevalence {
        self.prevalence
    }

    /// The number of replications the criterion required.
    #[inline]
    pub const fn support_order(&self) -> SupportOrder {
        self.support_order
    }
}
