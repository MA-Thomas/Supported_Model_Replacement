#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Prior-standardized average precision, chance-normalized average precision,
//! and supported predictive performance.
//!
//! The crate implements the manuscript's canonical fixed-model regime. Equal
//! scores form one threshold block and average precision is non-interpolated.
//! Same-design bootstrap replicates preserve the observed positive and negative
//! counts. Paired supported superiority can instead declare prospective
//! replication counts while keeping both models fixed.
//!
//! Conditional-null calibration places supported chance-normalized AP between
//! its measured conditional permutation null and perfect separation at one.
//! Score ties remain part of observed performance.
//!
//! # Regimes
//!
//! The scientific regime includes the reference prevalence and the mechanisms
//! allowed to vary across replications. Reference prevalence is supplied
//! separately because it enters AP analytically. The remaining fixed-score
//! resampling mechanism is represented by the trait
//! [`FixedScoreResamplingRegime`], so clustered or hierarchical resampling of
//! existing score blocks can be supplied without forking the estimator.
//! [`CanonicalFixedModel`] is the only mechanism shipped here.
//!
//! # Scope
//!
//! Conditional-null calibration is canonical only. The permutation null must
//! inherit the declared regime, so a custom regime yields a supported CNAP
//! value and its Monte Carlo error. Its matching null has to come from the same
//! mechanism before the result sits on a
//! conditional-chance-to-perfect-separation scale.
//!
//! Paired resampling supplies estimated supported superiority. Its reported
//! standard error is Monte Carlo error from a finite bootstrap replicate count.
//! Population-level inference requires an outer uncertainty procedure, which
//! this crate does not implement.
//!
//! # Example
//!
//! ```
//! use supported_ap::{Evaluation, ReferencePrevalence};
//!
//! let scores = [0.9, 0.7, 0.7, 0.2];
//! let labels = [true, true, false, false];
//! let evaluation = Evaluation::new(&scores, &labels)?;
//! let prevalence = ReferencePrevalence::new(0.5)?;
//!
//! assert!(evaluation.cnap(prevalence).value() <= 1.0);
//! # Ok::<(), supported_ap::SupportedApError>(())
//! ```

mod bootstrap;
mod conditional_null_calibration;
mod error;
mod evaluation;
mod execution;
mod options;
mod paired;
mod random;
mod regime;
mod results;
mod support;
mod types;

pub use error::SupportedApError;
pub use evaluation::Evaluation;
pub use options::{
    BootstrapOptions, ConditionalNullCalibrationOptions, NullDistributionStorage, Parallelism,
    PermutationOptions,
};
pub use paired::PairedEvaluation;
pub use regime::{CanonicalFixedModel, FixedScoreResamplingRegime, ReplicateCounts, ReplicateRng};
pub use results::{
    CalibratedSupportedCnapEstimate, ObservedMetrics, SupportedCnapEstimate,
    SupportedSuperiorityEstimate,
};
pub use support::{
    monte_carlo_standard_error, monte_carlo_standard_error_k, quantile, support_k, support_two,
    survival_level, survival_level_k,
};
pub use types::{
    BinaryLabel, CalibratedSupportedCnap, ChanceNormalizedAveragePrecision,
    ChanceNormalizedDifference, ClassCounts, Cnap,
    ConditionallyCalibratedSupportedChanceNormalizedAveragePrecision, PermutationCount,
    PermutationPValue, PriorStandardizedAp, PriorStandardizedAveragePrecision, ReferencePrevalence,
    ReplicateCount, SupportOrder, SupportedChanceNormalizedAveragePrecision, SupportedCnap,
    SupportedDifference,
};

/// Calculate non-interpolated average precision at a reference prevalence.
///
/// # Errors
///
/// Returns an error when the scores and labels do not form a valid evaluation.
pub fn prior_standardized_average_precision<L>(
    scores: &[f64],
    labels: &[L],
    prevalence: ReferencePrevalence,
) -> Result<PriorStandardizedAp, SupportedApError>
where
    L: Copy + Into<BinaryLabel>,
{
    Ok(Evaluation::new(scores, labels)?.prior_standardized_average_precision(prevalence))
}

/// Calculate chance-normalized average precision at a reference prevalence.
///
/// # Errors
///
/// Returns an error when the scores and labels do not form a valid evaluation.
pub fn cnap<L>(
    scores: &[f64],
    labels: &[L],
    prevalence: ReferencePrevalence,
) -> Result<Cnap, SupportedApError>
where
    L: Copy + Into<BinaryLabel>,
{
    Ok(Evaluation::new(scores, labels)?.cnap(prevalence))
}

/// Construct supported CNAP with the canonical fixed-model bootstrap.
///
/// # Errors
///
/// Returns an error for an invalid evaluation, incompatible support order, or
/// unavailable execution policy.
pub fn supported_cnap<L>(
    scores: &[f64],
    labels: &[L],
    prevalence: ReferencePrevalence,
    options: BootstrapOptions,
) -> Result<SupportedCnapEstimate, SupportedApError>
where
    L: Copy + Into<BinaryLabel>,
{
    Evaluation::new(scores, labels)?.supported_cnap(prevalence, options)
}

/// Conditionally calibrate supported CNAP against its fixed-score permutation null.
///
/// The supported effect and permutation null are returned alongside it. Exact
/// permutation validity requires evaluation outcomes that played no role in
/// developing the fixed scores.
///
/// # Errors
///
/// Returns an error for invalid evaluation or resampling configuration,
/// execution failure, or a conditional null at the upper endpoint.
pub fn conditionally_calibrated_supported_cnap<L>(
    scores: &[f64],
    labels: &[L],
    prevalence: ReferencePrevalence,
    options: ConditionalNullCalibrationOptions,
) -> Result<CalibratedSupportedCnapEstimate, SupportedApError>
where
    L: Copy + Into<BinaryLabel>,
{
    Evaluation::new(scores, labels)?.conditionally_calibrated_supported_cnap(prevalence, options)
}

/// Estimate supported superiority using the same resampled units for both
/// models. Positive values favor model A.
///
/// Marginal supported values cannot be differenced to obtain this paired estimate.
///
/// # Errors
///
/// Returns an error for invalid paired inputs, incompatible support order, or
/// unavailable execution policy.
pub fn estimated_supported_superiority<L>(
    model_a_scores: &[f64],
    model_b_scores: &[f64],
    labels: &[L],
    prevalence: ReferencePrevalence,
    options: BootstrapOptions,
) -> Result<SupportedSuperiorityEstimate, SupportedApError>
where
    L: Copy + Into<BinaryLabel>,
{
    PairedEvaluation::new(model_a_scores, model_b_scores, labels)?
        .estimated_supported_superiority(prevalence, options)
}

/// Estimate supported superiority for a prospective paired evaluation using
/// the same resampled units for both fixed models. Positive values favor model
/// A.
///
/// The observed data estimate the empirical class-conditional score
/// distributions; `replication_counts` declares the positive and negative
/// sample sizes of each prospective replication.
///
/// # Errors
///
/// Returns an error for invalid paired inputs, incompatible support order, or
/// unavailable execution policy.
pub fn estimated_supported_superiority_with_replication_counts<L>(
    model_a_scores: &[f64],
    model_b_scores: &[f64],
    labels: &[L],
    prevalence: ReferencePrevalence,
    replication_counts: ClassCounts,
    options: BootstrapOptions,
) -> Result<SupportedSuperiorityEstimate, SupportedApError>
where
    L: Copy + Into<BinaryLabel>,
{
    PairedEvaluation::new(model_a_scores, model_b_scores, labels)?
        .estimated_supported_superiority_with_replication_counts(
            prevalence,
            replication_counts,
            options,
        )
}
