use std::num::NonZeroUsize;

use crate::{PermutationCount, ReplicateCount, SupportOrder, SupportedApError};

/// How independent Monte Carlo jobs should be executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Parallelism {
    /// Always execute on the calling thread.
    Sequential,
    /// Use Rayon when the `parallel` feature is enabled, otherwise execute
    /// sequentially.
    #[default]
    Auto,
    /// Use a private Rayon pool with exactly this many worker threads.
    Threads(NonZeroUsize),
}

impl Parallelism {
    /// Request a private Rayon pool with exactly `value` worker threads.
    ///
    /// # Errors
    ///
    /// Returns [`SupportedApError::InvalidThreadCount`] when `value` is zero.
    pub fn threads(value: usize) -> Result<Self, SupportedApError> {
        NonZeroUsize::new(value)
            .map(Self::Threads)
            .ok_or(SupportedApError::InvalidThreadCount)
    }
}

/// Options for the canonical stratified bootstrap estimator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootstrapOptions {
    /// The number of bootstrap replicates `B`.
    pub replicates: ReplicateCount,
    /// The number of replications a claim must survive. `K = 2` is the default
    /// and the smallest count that expresses a replication criterion.
    /// Larger `K` states a stricter claim and must be prespecified.
    pub support_order: SupportOrder,
    /// The base seed. Streams are derived from logical indices, so results
    /// do not depend on scheduling or thread count.
    pub seed: u64,
    /// How independent replicates are executed.
    pub parallelism: Parallelism,
}

impl Default for BootstrapOptions {
    fn default() -> Self {
        Self {
            replicates: ReplicateCount::new(5_000).expect("the default is valid"),
            support_order: SupportOrder::new(2).expect("the default is valid"),
            seed: 0x5355_5041_5042_4f4f,
            parallelism: Parallelism::Auto,
        }
    }
}

/// Whether conditional-null calibration should return all permutation-level
/// supported-CNAP values or only their summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NullDistributionStorage {
    /// Retain the null mean and p-value only.
    SummaryOnly,
    /// Retain every permutation-level estimate.
    Full,
}

/// Options for the observation-label permutation null.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermutationOptions {
    /// The number of permutations `M`.
    pub permutations: PermutationCount,
    /// The base seed for outcome-label permutations. The nested bootstrap uses
    /// [`BootstrapOptions::seed`].
    pub seed: u64,
    /// How independent permutations are executed.
    pub parallelism: Parallelism,
    /// Whether permutation-level estimates are retained.
    pub storage: NullDistributionStorage,
}

impl Default for PermutationOptions {
    fn default() -> Self {
        Self {
            permutations: PermutationCount::new(500).expect("the default is valid"),
            seed: 0x5355_5041_5050_4552,
            parallelism: Parallelism::Auto,
            storage: NullDistributionStorage::SummaryOnly,
        }
    }
}

/// Options for conditional-null calibration. The empirical supported value
/// uses `bootstrap`; the null parallelizes across `permutation` jobs and
/// executes each job's bootstrap loop sequentially.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConditionalNullCalibrationOptions {
    /// Options for the observed supported effect and each permutation-level
    /// supported effect.
    pub bootstrap: BootstrapOptions,
    /// Options for the permutation null.
    pub permutation: PermutationOptions,
}
