use std::error::Error;
use std::fmt;

use crate::BinaryLabel;

/// Errors returned when an evaluation or estimator violates its mathematical
/// contract.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SupportedApError {
    /// An evaluation contained no units.
    EmptyEvaluation,
    /// Scores and labels had different lengths.
    LengthMismatch {
        /// Number of scores.
        scores: usize,
        /// Number of labels.
        labels: usize,
    },
    /// Paired model scores and labels had different lengths.
    PairedLengthMismatch {
        /// Number of scores for model A.
        model_a: usize,
        /// Number of scores for model B.
        model_b: usize,
        /// Number of labels.
        labels: usize,
    },
    /// A score was not finite.
    InvalidScore {
        /// Zero-based position of the invalid score.
        index: usize,
        /// Invalid score value.
        value: f64,
    },
    /// One outcome class was absent.
    MissingClass {
        /// Missing outcome class.
        class: BinaryLabel,
    },
    /// A byte label was neither zero nor one.
    InvalidBinaryLabel {
        /// Invalid byte value.
        value: u8,
    },
    /// A reference prevalence fell outside the open unit interval.
    InvalidReferencePrevalence {
        /// Invalid prevalence.
        value: f64,
    },
    /// A class count was zero.
    InvalidClassCount {
        /// Outcome class with an invalid count.
        class: BinaryLabel,
        /// Invalid count.
        value: usize,
    },
    /// The positive and negative counts overflow `usize`.
    SampleSizeOverflow,
    /// Fewer than two bootstrap replicates were requested.
    TooFewReplicates {
        /// Requested replicate count.
        value: usize,
    },
    /// Fewer than two permutations were requested.
    TooFewPermutations {
        /// Requested permutation count.
        value: usize,
    },
    /// A support order of zero was requested.
    ZeroSupportOrder,
    /// The support order exceeded the number of effects.
    SupportOrderExceedsSample {
        /// Requested support order.
        order: usize,
        /// Number of available effects.
        sample_size: usize,
    },
    /// Fewer effects were supplied than the support order requires.
    TooFewEffects {
        /// Number of supplied effects.
        value: usize,
    },
    /// An effect value was not finite.
    InvalidEffect {
        /// Zero-based position of the invalid effect.
        index: usize,
        /// Invalid effect value.
        value: f64,
    },
    /// A thread count of zero was requested.
    InvalidThreadCount,
    /// An explicit thread count needs the optional `parallel` feature.
    ParallelFeatureDisabled,
    /// The Rayon thread pool could not be built.
    ThreadPoolBuild {
        /// Error returned by Rayon.
        message: String,
    },
    /// The measured conditional null reached the fixed upper endpoint.
    ConditionalNullAtUpperEndpoint {
        /// Estimated conditional null.
        null: f64,
    },
    /// A conditional-null distribution was requested but was not retained.
    ConditionalNullDistributionNotStored,
    /// A survival frequency fell outside `(0, 1]`.
    InvalidSurvivalFrequency {
        /// Invalid survival frequency.
        value: f64,
    },
    /// A probability fell outside the closed unit interval.
    InvalidProbability {
        /// Invalid probability.
        value: f64,
    },
    /// A random index was requested from an empty range.
    InvalidRandomBound,
    /// A regime addressed a threshold block outside its declared layout.
    RegimeBlockOutOfRange {
        /// Requested zero-based block index.
        block: usize,
        /// Number of declared threshold blocks.
        threshold_count: usize,
    },
    /// A regime count overflowed `usize`.
    RegimeCountOverflow,
    /// A custom regime declared a threshold layout incompatible with the evaluation.
    RegimeThresholdCountMismatch {
        /// Threshold blocks in the evaluation.
        evaluation: usize,
        /// Threshold blocks declared by the regime.
        regime: usize,
    },
    /// A regime recorded a different number of units than it declared.
    RegimeCountMismatch {
        /// Positive units the regime declared for the replicate.
        declared_positive: usize,
        /// Negative units the regime declared for the replicate.
        declared_negative: usize,
        /// Positive units the regime actually recorded.
        recorded_positive: usize,
        /// Negative units the regime actually recorded.
        recorded_negative: usize,
    },
    /// A prevalence profile was requested with no reference prevalences.
    EmptyPrevalenceProfile,
}

impl fmt::Display for SupportedApError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyEvaluation => write!(f, "an evaluation must contain at least one unit"),
            Self::LengthMismatch { scores, labels } => write!(
                f,
                "scores and labels have different lengths ({scores} and {labels})"
            ),
            Self::PairedLengthMismatch {
                model_a,
                model_b,
                labels,
            } => write!(
                f,
                "paired model scores and labels have different lengths \
                 (model A: {model_a}, model B: {model_b}, labels: {labels})"
            ),
            Self::InvalidScore { index, value } => {
                write!(f, "score at index {index} is not finite: {value}")
            }
            Self::MissingClass { class } => {
                write!(f, "the evaluation contains no {class} units")
            }
            Self::InvalidBinaryLabel { value } => {
                write!(f, "binary labels must be 0 or 1, received {value}")
            }
            Self::InvalidReferencePrevalence { value } => write!(
                f,
                "reference prevalence must be finite and strictly between 0 and 1, received {value}"
            ),
            Self::InvalidClassCount { class, value } => {
                write!(
                    f,
                    "the {class} class count must be positive, received {value}"
                )
            }
            Self::SampleSizeOverflow => write!(f, "positive and negative counts overflow usize"),
            Self::TooFewReplicates { value } => {
                write!(
                    f,
                    "supported CNAP requires at least two replicates, received {value}"
                )
            }
            Self::TooFewPermutations { value } => write!(
                f,
                "conditional-null calibration requires at least two permutations, received {value}"
            ),
            Self::ZeroSupportOrder => write!(f, "the support order K must be positive"),
            Self::SupportOrderExceedsSample { order, sample_size } => write!(
                f,
                "support order {order} exceeds the effect sample size {sample_size}"
            ),
            Self::TooFewEffects { value } => write!(
                f,
                "the two-replication support estimator requires at least two effects, received {value}"
            ),
            Self::InvalidEffect { index, value } => {
                write!(f, "effect at index {index} is not finite: {value}")
            }
            Self::InvalidThreadCount => write!(f, "a requested thread count must be positive"),
            Self::ParallelFeatureDisabled => write!(
                f,
                "an explicit thread count requires the optional `parallel` feature"
            ),
            Self::ThreadPoolBuild { message } => {
                write!(f, "failed to build the Rayon thread pool: {message}")
            }
            Self::ConditionalNullAtUpperEndpoint { null } => write!(
                f,
                "the measured conditional null must be below the fixed upper endpoint of one, \
                 received {null}"
            ),
            Self::ConditionalNullDistributionNotStored => write!(
                f,
                "the conditional-null distribution was not retained; use \
                 NullDistributionStorage::Full"
            ),
            Self::InvalidSurvivalFrequency { value } => write!(
                f,
                "a survival frequency must be finite and in (0, 1], received {value}"
            ),
            Self::InvalidProbability { value } => write!(
                f,
                "a probability must be finite and in [0, 1], received {value}"
            ),
            Self::InvalidRandomBound => {
                write!(f, "a random index bound must be positive")
            }
            Self::RegimeBlockOutOfRange {
                block,
                threshold_count,
            } => write!(
                f,
                "regime block index {block} is outside 0..{threshold_count}"
            ),
            Self::RegimeCountOverflow => {
                write!(f, "a regime threshold-block count overflowed usize")
            }
            Self::RegimeThresholdCountMismatch { evaluation, regime } => write!(
                f,
                "the regime declares {regime} threshold blocks but the evaluation has {evaluation}"
            ),
            Self::RegimeCountMismatch {
                declared_positive,
                declared_negative,
                recorded_positive,
                recorded_negative,
            } => write!(
                f,
                "the regime declared {declared_positive} positive and {declared_negative} \
                 negative units but recorded {recorded_positive} and {recorded_negative}"
            ),
            Self::EmptyPrevalenceProfile => {
                write!(
                    f,
                    "a prevalence profile needs at least one reference prevalence"
                )
            }
        }
    }
}

impl Error for SupportedApError {}
