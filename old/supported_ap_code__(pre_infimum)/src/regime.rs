//! Fixed-score resampling mechanisms for constructing an empirical replication distribution.
//!
//! Supported performance is defined relative to a declared
//! evaluation-generating regime.
//! The full scientific regime includes the reference prevalence and the
//! mechanisms that may vary across replications. This module implements its
//! fixed-score resampling component: how existing evaluation units or score
//! blocks are resampled and whether class counts are fixed. Each draw is a
//! computational bootstrap replicate used to estimate the scientific
//! replication distribution. Only the canonical fixed-model mechanism ships
//! here.
//!
//! The resampling mechanism fills threshold-block counts and does not receive
//! the reference prevalence. Prevalence enters when each draw is evaluated.
//! This analytical separation lets one set of bootstrap replicates serve a
//! whole prevalence sweep.

use rand::Rng;

use crate::{ClassCounts, SupportedApError};

/// The random source handed to a regime.
///
/// Stream derivation stays inside the crate, so results remain independent of
/// scheduling and thread count, and the `rand` version stays out of the public
/// API.
pub struct ReplicateRng<'a> {
    inner: &'a mut dyn RngCore8,
}

/// Object-safe view of the parts of `rand::Rng` a regime needs.
trait RngCore8 {
    fn index(&mut self, bound: usize) -> usize;
    fn boolean(&mut self, probability: f64) -> bool;
}

impl<R> RngCore8 for R
where
    R: Rng,
{
    fn index(&mut self, bound: usize) -> usize {
        self.gen_range(0..bound)
    }

    fn boolean(&mut self, probability: f64) -> bool {
        self.gen_bool(probability)
    }
}

impl<'a> ReplicateRng<'a> {
    pub(crate) fn new<R>(inner: &'a mut R) -> Self
    where
        R: Rng,
    {
        Self { inner }
    }

    /// A uniform index in `0..bound`.
    ///
    /// # Errors
    ///
    /// Returns [`SupportedApError::InvalidRandomBound`] when `bound` is zero.
    #[inline]
    pub fn index(&mut self, bound: usize) -> Result<usize, SupportedApError> {
        if bound == 0 {
            return Err(SupportedApError::InvalidRandomBound);
        }
        Ok(self.inner.index(bound))
    }

    /// A Bernoulli draw.
    ///
    /// # Errors
    ///
    /// Returns [`SupportedApError::InvalidProbability`] unless `probability` is
    /// finite and in `[0, 1]`.
    #[inline]
    pub fn boolean(&mut self, probability: f64) -> Result<bool, SupportedApError> {
        if !(probability.is_finite() && (0.0..=1.0).contains(&probability)) {
            return Err(SupportedApError::InvalidProbability { value: probability });
        }
        Ok(self.inner.boolean(probability))
    }
}

/// Threshold-block counts for one replicate evaluation.
///
/// Blocks are ordered by descending score, so block zero holds the highest
/// score. A regime reports how many positive and negative units of its
/// replicate landed in each block, together with the replicate's class counts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct ReplicateCounts {
    positive: Vec<usize>,
    negative: Vec<usize>,
    counts: ClassCounts,
}

impl ReplicateCounts {
    /// An empty buffer sized for `threshold_count` blocks.
    pub fn new(threshold_count: usize, counts: ClassCounts) -> Self {
        Self {
            positive: vec![0; threshold_count],
            negative: vec![0; threshold_count],
            counts,
        }
    }

    /// Zero every block and declare the class counts of the next replicate.
    ///
    /// A mechanism that varies class counts across bootstrap replicates reports them here.
    #[inline]
    pub fn reset(&mut self, counts: ClassCounts) {
        self.positive.fill(0);
        self.negative.fill(0);
        self.counts = counts;
    }

    /// Record one positive unit in `block`.
    ///
    /// # Errors
    ///
    /// Returns an error when `block` is outside the declared threshold layout
    /// or its count would overflow.
    #[inline]
    pub fn add_positive(&mut self, block: usize) -> Result<(), SupportedApError> {
        increment_block(&mut self.positive, block)
    }

    /// Record one negative unit in `block`.
    ///
    /// # Errors
    ///
    /// Returns an error when `block` is outside the declared threshold layout
    /// or its count would overflow.
    #[inline]
    pub fn add_negative(&mut self, block: usize) -> Result<(), SupportedApError> {
        increment_block(&mut self.negative, block)
    }

    /// Positive units per threshold block.
    #[must_use]
    #[inline]
    pub fn positive(&self) -> &[usize] {
        &self.positive
    }

    /// Negative units per threshold block.
    #[must_use]
    #[inline]
    pub fn negative(&self) -> &[usize] {
        &self.negative
    }

    /// The class counts of this replicate.
    #[inline]
    pub const fn class_counts(&self) -> ClassCounts {
        self.counts
    }

    /// The number of threshold blocks the buffer covers.
    #[must_use]
    #[inline]
    pub fn threshold_count(&self) -> usize {
        self.positive.len()
    }

    /// Check that the recorded units match the declared class counts.
    pub(crate) fn validate(&self) -> Result<(), SupportedApError> {
        let positive = checked_sum(&self.positive)?;
        let negative = checked_sum(&self.negative)?;
        if positive != self.counts.positive() || negative != self.counts.negative() {
            return Err(SupportedApError::RegimeCountMismatch {
                declared_positive: self.counts.positive(),
                declared_negative: self.counts.negative(),
                recorded_positive: positive,
                recorded_negative: negative,
            });
        }
        Ok(())
    }
}

/// How one fixed-score bootstrap replicate of a declared regime is generated.
///
/// Implement this to state a replication mechanism other than the canonical
/// one: resample whole clusters, hold sites fixed and resample within them, or
/// generate class counts under a sampling design. Every bootstrap replicate
/// uses the original ordered score blocks. A process that retrains a model or
/// changes its scores needs a higher-level simulation that produces complete
/// replication effects.
///
/// # Calibration
///
/// The conditional permutation null must respect the declared resampling
/// design, so calibrated estimation in this release is available for the
/// canonical independent-unit regime only.
pub trait FixedScoreResamplingRegime: Sync {
    /// Per-worker state, allocated once and reused across replicates.
    ///
    /// Use `()` when a regime needs none.
    type Scratch: Send;

    /// Allocate the per-worker state.
    fn new_scratch(&self) -> Self::Scratch;

    /// The number of threshold blocks, which fixes the buffer width.
    ///
    /// Every block a replicate can occupy must be counted here, including
    /// blocks a particular replicate leaves empty.
    fn threshold_count(&self) -> usize;

    /// The class counts of the observed evaluation.
    ///
    /// Used to size buffers. A regime may report different counts per
    /// replicate through [`ReplicateCounts::reset`].
    fn class_counts(&self) -> ClassCounts;

    /// Draw replicate `replicate` into `out`.
    ///
    /// Call [`ReplicateCounts::reset`] first. The same logical index always
    /// receives the same random stream, so results do not depend on execution
    /// order.
    ///
    /// # Errors
    ///
    /// Return an error when the regime cannot complete the draw or a checked
    /// random/count operation rejects its input.
    fn draw(
        &self,
        replicate: usize,
        rng: &mut ReplicateRng<'_>,
        scratch: &mut Self::Scratch,
        out: &mut ReplicateCounts,
    ) -> Result<(), SupportedApError>;
}

/// The canonical fixed-model regime: a stratified `n`-out-of-`n` bootstrap.
///
/// Positive units are drawn with replacement from the observed positives and
/// negative units from the observed negatives, so every replicate preserves
/// both class counts and the total size. This approximates replication of the
/// same evaluation design under the empirical class-conditional distributions
/// of the observed data. It supplies no variation the data does not contain,
/// so it answers a within-sample question.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct CanonicalFixedModel<'a> {
    positive_blocks: &'a [usize],
    negative_blocks: &'a [usize],
    threshold_count: usize,
    counts: ClassCounts,
}

impl<'a> CanonicalFixedModel<'a> {
    pub(crate) fn new(
        positive_blocks: &'a [usize],
        negative_blocks: &'a [usize],
        threshold_count: usize,
        counts: ClassCounts,
    ) -> Self {
        debug_assert_eq!(positive_blocks.len(), counts.positive());
        debug_assert_eq!(negative_blocks.len(), counts.negative());
        Self {
            positive_blocks,
            negative_blocks,
            threshold_count,
            counts,
        }
    }
}

impl FixedScoreResamplingRegime for CanonicalFixedModel<'_> {
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
        rng: &mut ReplicateRng<'_>,
        _scratch: &mut Self::Scratch,
        out: &mut ReplicateCounts,
    ) -> Result<(), SupportedApError> {
        out.reset(self.counts);
        let positive = self.positive_blocks.len();
        let negative = self.negative_blocks.len();
        for _ in 0..positive {
            let sampled = rng.index(positive)?;
            out.add_positive(self.positive_blocks[sampled])?;
        }
        for _ in 0..negative {
            let sampled = rng.index(negative)?;
            out.add_negative(self.negative_blocks[sampled])?;
        }
        Ok(())
    }
}

fn increment_block(counts: &mut [usize], block: usize) -> Result<(), SupportedApError> {
    let threshold_count = counts.len();
    let count = counts
        .get_mut(block)
        .ok_or(SupportedApError::RegimeBlockOutOfRange {
            block,
            threshold_count,
        })?;
    *count = count
        .checked_add(1)
        .ok_or(SupportedApError::RegimeCountOverflow)?;
    Ok(())
}

fn checked_sum(values: &[usize]) -> Result<usize, SupportedApError> {
    values.iter().try_fold(0usize, |total, &value| {
        total
            .checked_add(value)
            .ok_or(SupportedApError::RegimeCountOverflow)
    })
}
