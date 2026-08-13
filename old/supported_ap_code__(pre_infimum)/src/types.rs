use std::fmt;
use std::num::NonZeroUsize;

use crate::SupportedApError;

/// A binary outcome label.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryLabel {
    /// The unit did not experience the outcome.
    Negative,
    /// The unit experienced the outcome.
    Positive,
}

impl BinaryLabel {
    /// Whether this label is [`BinaryLabel::Positive`].
    #[must_use]
    #[inline]
    pub const fn is_positive(self) -> bool {
        matches!(self, Self::Positive)
    }
}

impl From<bool> for BinaryLabel {
    fn from(value: bool) -> Self {
        if value {
            Self::Positive
        } else {
            Self::Negative
        }
    }
}

impl TryFrom<u8> for BinaryLabel {
    type Error = SupportedApError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Negative),
            1 => Ok(Self::Positive),
            value => Err(SupportedApError::InvalidBinaryLabel { value }),
        }
    }
}

impl fmt::Display for BinaryLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Negative => f.write_str("negative"),
            Self::Positive => f.write_str("positive"),
        }
    }
}

/// A validated reference prevalence in the open interval `(0, 1)`.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct ReferencePrevalence(f64);

impl ReferencePrevalence {
    /// Validate a reference prevalence in the open interval `(0, 1)`.
    ///
    /// # Errors
    ///
    /// Returns an error unless `value` is finite and strictly between zero and
    /// one.
    pub fn new(value: f64) -> Result<Self, SupportedApError> {
        if value.is_finite() && value > 0.0 && value < 1.0 {
            Ok(Self(value))
        } else {
            Err(SupportedApError::InvalidReferencePrevalence { value })
        }
    }

    /// The prevalence as a plain float.
    #[must_use]
    #[inline]
    pub const fn value(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for ReferencePrevalence {
    type Error = SupportedApError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl fmt::Display for ReferencePrevalence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A bootstrap replicate count, guaranteed to be at least two.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplicateCount(NonZeroUsize);

impl ReplicateCount {
    /// Validate a replicate count of at least two.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is less than two.
    pub fn new(value: usize) -> Result<Self, SupportedApError> {
        if value < 2 {
            return Err(SupportedApError::TooFewReplicates { value });
        }
        Ok(Self(
            NonZeroUsize::new(value).expect("value is at least two"),
        ))
    }

    /// The replicate count as a plain integer.
    #[must_use]
    #[inline]
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

/// A permutation count, guaranteed to be at least two.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PermutationCount(NonZeroUsize);

impl PermutationCount {
    /// A single permutation yields a null mean from one draw and a p-value
    /// restricted to two values, so the floor is two.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is less than two.
    pub fn new(value: usize) -> Result<Self, SupportedApError> {
        if value < 2 {
            return Err(SupportedApError::TooFewPermutations { value });
        }
        Ok(Self(
            NonZeroUsize::new(value).expect("value is at least two"),
        ))
    }

    /// The permutation count as a plain integer.
    #[must_use]
    #[inline]
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

/// The number of replications whose common minimum defines `S_K`.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SupportOrder(NonZeroUsize);

impl SupportOrder {
    /// Validate a positive support order `K`.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is zero.
    pub fn new(value: usize) -> Result<Self, SupportedApError> {
        NonZeroUsize::new(value)
            .map(Self)
            .ok_or(SupportedApError::ZeroSupportOrder)
    }

    /// The support order as a plain integer.
    #[must_use]
    #[inline]
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

/// Positive and negative evaluation counts, each guaranteed to be nonzero.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClassCounts {
    positive: NonZeroUsize,
    negative: NonZeroUsize,
}

impl ClassCounts {
    /// Validate a pair of nonzero class counts.
    ///
    /// # Errors
    ///
    /// Returns an error when either count is zero or their sum overflows
    /// `usize`.
    pub fn new(positive: usize, negative: usize) -> Result<Self, SupportedApError> {
        let positive = NonZeroUsize::new(positive).ok_or(SupportedApError::InvalidClassCount {
            class: BinaryLabel::Positive,
            value: positive,
        })?;
        let negative = NonZeroUsize::new(negative).ok_or(SupportedApError::InvalidClassCount {
            class: BinaryLabel::Negative,
            value: negative,
        })?;
        positive
            .get()
            .checked_add(negative.get())
            .ok_or(SupportedApError::SampleSizeOverflow)?;
        Ok(Self { positive, negative })
    }

    /// The number of positive units.
    #[must_use]
    #[inline]
    pub const fn positive(self) -> usize {
        self.positive.get()
    }

    /// The number of negative units.
    #[must_use]
    #[inline]
    pub const fn negative(self) -> usize {
        self.negative.get()
    }

    /// The total number of units.
    #[must_use]
    #[inline]
    pub fn total(self) -> usize {
        self.positive.get() + self.negative.get()
    }
}

macro_rules! effect_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[must_use]
        #[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
        pub struct $name(f64);

        impl $name {
            /// The underlying value.
            #[inline]
            pub const fn value(self) -> f64 {
                self.0
            }

            #[inline]
            pub(crate) const fn from_value(value: f64) -> Self {
                Self(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

effect_type!(
    PriorStandardizedAveragePrecision,
    "Non-interpolated prior-standardized average precision evaluated at a declared reference prevalence."
);
effect_type!(
    ChanceNormalizedAveragePrecision,
    "Prior-standardized average precision normalized so population random ranking is zero and perfect ranking is one."
);
effect_type!(
    SupportedChanceNormalizedAveragePrecision,
    "Supported chance-normalized average precision: the empirical K-replication support operator applied to bootstrap CNAP values."
);
effect_type!(
    ConditionallyCalibratedSupportedChanceNormalizedAveragePrecision,
    "Conditionally calibrated supported chance-normalized average precision: the supported value rescaled between its conditional permutation null and perfect separation at one."
);
effect_type!(
    ChanceNormalizedDifference,
    "The within-evaluation CNAP difference between paired models A and B."
);
effect_type!(
    SupportedDifference,
    "Estimated supported superiority: the empirical K-replication support operator applied to paired CNAP differences."
);
effect_type!(
    PermutationPValue,
    "The upper-tail permutation p-value with the standard plus-one correction."
);

/// Short alias for [`PriorStandardizedAveragePrecision`].
pub type PriorStandardizedAp = PriorStandardizedAveragePrecision;
/// Short alias for [`ChanceNormalizedAveragePrecision`].
pub type Cnap = ChanceNormalizedAveragePrecision;
/// Short alias for [`SupportedChanceNormalizedAveragePrecision`].
pub type SupportedCnap = SupportedChanceNormalizedAveragePrecision;
/// Short alias for [`ConditionallyCalibratedSupportedChanceNormalizedAveragePrecision`].
pub type CalibratedSupportedCnap = ConditionallyCalibratedSupportedChanceNormalizedAveragePrecision;
