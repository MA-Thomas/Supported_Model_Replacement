use std::fmt;

#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    PairedLengthMismatch,
    EmptyEvaluation,
    InvalidScore {
        index: usize,
        value: f64,
    },
    MissingClass,
    InvalidPrevalence(f64),
    EmptyPrevalenceSet,
    InvalidPrevalenceInterval,
    TooFewGridPoints,
    InvalidSearchTolerance,
    InvalidSearchIterations,
    InvalidPrevalenceSearchCertificate,
    InvalidSupportOrder,
    InvalidComputationalReplicationCount,
    SupportOrderExceedsList {
        order: usize,
        list_length: usize,
    },
    InvalidClassCounts,
    EmptyEffects,
    InvalidEffect {
        index: usize,
        value: f64,
    },
    InvalidMagnitudeThreshold(f64),
    InvalidSurvivalFloor(f64),
    InvalidSurvivalRequirement(f64),
    InvalidConcentrationFactor(f64),
    InvalidOptimizationTolerance,
    InvalidOptimizationTimeLimit,
    InvalidOptimizationThreadCount,
    UnsupportedCombinatorialThreadCount(u32),
    InvalidOptimizationBudget,
    ReferenceBackendTooLarge {
        pairs: usize,
        limit: usize,
    },
    ReferenceBackendUnavailable,
    InvalidConcentrationSearch,
    OptimizationFailed(String),
    EmptyEmpiricalEvaluations,
    NestedRowDesignCountMismatch {
        evaluations: usize,
        designs: usize,
    },
    ComputationalOrderExceedsRow {
        row: usize,
        order: usize,
        list_length: usize,
    },
    InvalidObservedEffect {
        row: usize,
        value: f64,
    },
    InvalidComputationalEffect {
        row: usize,
        index: usize,
        value: f64,
    },
    MissingExchangeabilityJustification,
    MissingReferenceLimitation,
    MissingTransportJustification,
    Csv(csv::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
    MissingColumn(String),
    InvalidLabel {
        row: usize,
        value: String,
    },
    InvalidCsvScore {
        row: usize,
        column: String,
        value: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PairedLengthMismatch => {
                write!(f, "paired score vectors and labels have different lengths")
            }
            Self::EmptyEvaluation => write!(f, "an evaluation must contain observations"),
            Self::InvalidScore { index, value } => {
                write!(f, "score at index {index} is not finite: {value}")
            }
            Self::MissingClass => write!(f, "both outcome classes must be present"),
            Self::InvalidPrevalence(value) => write!(
                f,
                "prevalence must be finite and strictly between zero and one: {value}"
            ),
            Self::EmptyPrevalenceSet => write!(f, "target-prevalence set cannot be empty"),
            Self::InvalidPrevalenceInterval => write!(f, "invalid prevalence interval"),
            Self::TooFewGridPoints => write!(f, "interval search needs at least three points"),
            Self::InvalidSearchTolerance => write!(f, "invalid search tolerance"),
            Self::InvalidPrevalenceSearchCertificate => {
                write!(f, "invalid prevalence-search certificate")
            }
            Self::InvalidSearchIterations => {
                write!(f, "interval search iterations must be positive")
            }
            Self::InvalidSupportOrder => write!(f, "support order K must be positive"),
            Self::InvalidComputationalReplicationCount => {
                write!(f, "computational replication count J must be positive")
            }
            Self::SupportOrderExceedsList { order, list_length } => write!(
                f,
                "support order K={order} exceeds finite effect-list length L={list_length}"
            ),
            Self::InvalidClassCounts => write!(f, "both class counts must be positive"),
            Self::EmptyEffects => write!(f, "effect sample cannot be empty"),
            Self::InvalidEffect { index, value } => {
                write!(f, "effect at index {index} is not finite: {value}")
            }
            Self::InvalidMagnitudeThreshold(value) => write!(
                f,
                "magnitude threshold must be finite and nonnegative: {value}"
            ),
            Self::InvalidSurvivalFloor(value) => {
                write!(f, "survival floor must be finite and nonnegative: {value}")
            }
            Self::InvalidSurvivalRequirement(value) => write!(
                f,
                "literal-survival requirement gamma must be strictly between zero and one: {value}"
            ),
            Self::InvalidConcentrationFactor(value) => write!(
                f,
                "concentration factor Gamma must be finite and at least one: {value}"
            ),
            Self::InvalidOptimizationTolerance => {
                write!(f, "optimization tolerances must be finite and nonnegative")
            }
            Self::InvalidOptimizationTimeLimit => {
                write!(f, "optimization time limit must be finite and positive")
            }
            Self::InvalidOptimizationThreadCount => {
                write!(f, "optimization thread count must be positive")
            }
            Self::UnsupportedCombinatorialThreadCount(threads) => write!(
                f,
                "the combinatorial AUROC backend is single-threaded per solve; requested {threads} threads"
            ),
            Self::InvalidOptimizationBudget => write!(
                f,
                "optimization node budget and restart count must be positive"
            ),
            Self::ReferenceBackendTooLarge { pairs, limit } => write!(
                f,
                "the HiGHS reference backend accepts at most {limit} pairs; this problem has \
                 {pairs}. It exists to check the combinatorial solver on small evaluations, not \
                 to run full-design assessments"
            ),
            Self::ReferenceBackendUnavailable => write!(
                f,
                "the HiGHS reference backend requires the `highs-reference` feature"
            ),
            Self::InvalidConcentrationSearch => write!(
                f,
                "concentration search requires a positive tolerance and iteration count"
            ),
            Self::OptimizationFailed(message) => write!(f, "optimization failed: {message}"),
            Self::EmptyEmpiricalEvaluations => {
                write!(f, "at least one empirical evaluation is required")
            }
            Self::NestedRowDesignCountMismatch {
                evaluations,
                designs,
            } => write!(
                f,
                "nested assessment has {evaluations} empirical evaluations but {designs} row resampling designs"
            ),
            Self::ComputationalOrderExceedsRow {
                row,
                order,
                list_length,
            } => write!(
                f,
                "computational order K_C={order} exceeds row {row} effect-list length J={list_length}"
            ),
            Self::InvalidObservedEffect { row, value } => {
                write!(f, "observed effect in row {row} is not finite: {value}")
            }
            Self::InvalidComputationalEffect { row, index, value } => write!(
                f,
                "computational effect at row {row}, index {index} is not finite: {value}"
            ),
            Self::MissingExchangeabilityJustification => write!(
                f,
                "the exchangeable-label diagnostic requires a nonempty scientific justification"
            ),
            Self::MissingReferenceLimitation => write!(
                f,
                "a nonempty reason is required when exchangeable labels are not asserted"
            ),
            Self::MissingTransportJustification => {
                write!(f, "score-level transport requires a justification")
            }
            Self::Csv(error) => error.fmt(f),
            Self::Io(error) => error.fmt(f),
            Self::Json(error) => error.fmt(f),
            Self::MissingColumn(column) => write!(f, "column not found: {column}"),
            Self::InvalidLabel { row, value } => {
                write!(f, "invalid label at row {row}: {value}")
            }
            Self::InvalidCsvScore { row, column, value } => {
                write!(f, "invalid score at row {row}, column {column}: {value}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<csv::Error> for Error {
    fn from(value: csv::Error) -> Self {
        Self::Csv(value)
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
