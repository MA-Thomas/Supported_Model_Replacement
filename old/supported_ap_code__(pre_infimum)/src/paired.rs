use rand::Rng;

use crate::evaluation::{cnap_from_prior_standardized_ap, prior_standardized_ap_from_counts};
use crate::execution::map_init_indexed;
use crate::random::{PAIRED_BOOTSTRAP_DOMAIN, rng_for};
use crate::support::{standard_error_from_values, support_validated};
use crate::{
    BinaryLabel, BootstrapOptions, ChanceNormalizedDifference, ClassCounts, Cnap, Evaluation,
    ReferencePrevalence, SupportedApError, SupportedDifference, SupportedSuperiorityEstimate,
};

/// Two model rankings evaluated on the same labeled units.
#[derive(Debug, Clone)]
#[must_use]
pub struct PairedEvaluation {
    model_a: Evaluation,
    model_b: Evaluation,
}

impl PairedEvaluation {
    /// Construct a paired evaluation from two score vectors on shared labels.
    ///
    /// # Errors
    ///
    /// Returns an error for mismatched lengths or when either evaluation fails
    /// score, label, or class validation.
    pub fn new<L>(
        model_a_scores: &[f64],
        model_b_scores: &[f64],
        labels: &[L],
    ) -> Result<Self, SupportedApError>
    where
        L: Copy + Into<BinaryLabel>,
    {
        if model_a_scores.len() != labels.len() || model_b_scores.len() != labels.len() {
            return Err(SupportedApError::PairedLengthMismatch {
                model_a: model_a_scores.len(),
                model_b: model_b_scores.len(),
                labels: labels.len(),
            });
        }
        Ok(Self {
            model_a: Evaluation::new(model_a_scores, labels)?,
            model_b: Evaluation::new(model_b_scores, labels)?,
        })
    }

    /// Construct a paired evaluation from labels encoded as zero and one.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::new`], plus an error when a label is
    /// not zero or one.
    pub fn from_u8(
        model_a_scores: &[f64],
        model_b_scores: &[f64],
        labels: &[u8],
    ) -> Result<Self, SupportedApError> {
        if model_a_scores.len() != labels.len() || model_b_scores.len() != labels.len() {
            return Err(SupportedApError::PairedLengthMismatch {
                model_a: model_a_scores.len(),
                model_b: model_b_scores.len(),
                labels: labels.len(),
            });
        }
        let labels = labels
            .iter()
            .copied()
            .map(BinaryLabel::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(model_a_scores, model_b_scores, &labels)
    }

    /// The evaluation for model A.
    #[inline]
    pub fn model_a(&self) -> &Evaluation {
        &self.model_a
    }

    /// The evaluation for model B.
    #[inline]
    pub fn model_b(&self) -> &Evaluation {
        &self.model_b
    }

    /// The within-evaluation CNAP difference on the observed data.
    pub fn observed_difference(
        &self,
        prevalence: ReferencePrevalence,
    ) -> ChanceNormalizedDifference {
        ChanceNormalizedDifference::from_value(
            self.model_a.cnap(prevalence).value() - self.model_b.cnap(prevalence).value(),
        )
    }

    /// Estimate supported superiority using shared class-stratified resamples.
    /// Positive values favor model A.
    ///
    /// # Errors
    ///
    /// Returns an error for an excessive support order or unavailable execution
    /// policy.
    pub fn estimated_supported_superiority(
        &self,
        prevalence: ReferencePrevalence,
        options: BootstrapOptions,
    ) -> Result<SupportedSuperiorityEstimate, SupportedApError> {
        let mut profile = self.estimated_supported_superiority_profile(&[prevalence], options)?;
        Ok(profile.remove(0))
    }

    /// Estimate supported superiority for a prospective paired evaluation
    /// with declared positive and negative replication counts.
    ///
    /// The observed evaluation supplies the empirical class-conditional score
    /// distributions. `replication_counts` determines how many shared positive
    /// and negative units are drawn for each prospective replication.
    ///
    /// # Errors
    ///
    /// Returns an error for an excessive support order or unavailable execution
    /// policy.
    pub fn estimated_supported_superiority_with_replication_counts(
        &self,
        prevalence: ReferencePrevalence,
        replication_counts: ClassCounts,
        options: BootstrapOptions,
    ) -> Result<SupportedSuperiorityEstimate, SupportedApError> {
        let mut profile = self.estimated_supported_superiority_profile_with_replication_counts(
            &[prevalence],
            replication_counts,
            options,
        )?;
        Ok(profile.remove(0))
    }

    /// Estimate supported superiority at several prevalences from one
    /// shared set of same-design resamples.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty profile, excessive support order, or
    /// unavailable execution policy.
    pub fn estimated_supported_superiority_profile(
        &self,
        prevalences: &[ReferencePrevalence],
        options: BootstrapOptions,
    ) -> Result<Vec<SupportedSuperiorityEstimate>, SupportedApError> {
        self.estimated_supported_superiority_profile_with_replication_counts(
            prevalences,
            self.model_a.class_counts(),
            options,
        )
    }

    /// Estimate supported superiority at several prevalences for a prospective
    /// paired evaluation with declared positive and negative replication
    /// counts, using one shared set of resamples.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty profile, excessive support order, or
    /// unavailable execution policy.
    pub fn estimated_supported_superiority_profile_with_replication_counts(
        &self,
        prevalences: &[ReferencePrevalence],
        replication_counts: ClassCounts,
        options: BootstrapOptions,
    ) -> Result<Vec<SupportedSuperiorityEstimate>, SupportedApError> {
        if prevalences.is_empty() {
            return Err(SupportedApError::EmptyPrevalenceProfile);
        }
        let order = options.support_order.get();
        if order > options.replicates.get() {
            return Err(SupportedApError::SupportOrderExceedsSample {
                order,
                sample_size: options.replicates.get(),
            });
        }

        let rows = map_init_indexed(
            options.replicates.get(),
            options.parallelism,
            || PairedBootstrapScratch::new(&self.model_a, &self.model_b),
            |scratch, replicate| {
                let mut rng = rng_for(options.seed, PAIRED_BOOTSTRAP_DOMAIN, 0, replicate);
                resampled_differences(
                    self.model_a.threshold_count(),
                    self.model_b.threshold_count(),
                    &self.model_a.positive_blocks,
                    &self.model_a.negative_blocks,
                    &self.model_b.positive_blocks,
                    &self.model_b.negative_blocks,
                    self.model_a.class_counts(),
                    replication_counts,
                    prevalences,
                    &mut rng,
                    scratch,
                )
            },
        )?;

        let mut columns = vec![Vec::with_capacity(options.replicates.get()); prevalences.len()];
        for row in rows {
            for (column, value) in columns.iter_mut().zip(row) {
                column.push(value);
            }
        }

        Ok(columns
            .into_iter()
            .zip(prevalences)
            .map(|(differences, &prevalence)| {
                build_paired_estimate(self, prevalence, differences, replication_counts, options)
            })
            .collect())
    }
}

fn build_paired_estimate(
    paired: &PairedEvaluation,
    prevalence: ReferencePrevalence,
    differences: Vec<ChanceNormalizedDifference>,
    replication_counts: ClassCounts,
    options: BootstrapOptions,
) -> SupportedSuperiorityEstimate {
    let values: Vec<f64> = differences.iter().map(|value| value.value()).collect();
    let supported =
        SupportedDifference::from_value(support_validated(&values, options.support_order));
    let mean =
        ChanceNormalizedDifference::from_value(values.iter().sum::<f64>() / values.len() as f64);
    let standard_error = standard_error_from_values(&values, options.support_order);

    SupportedSuperiorityEstimate::new(
        paired.observed_difference(prevalence),
        supported,
        mean,
        standard_error,
        differences,
        paired.model_a.class_counts(),
        replication_counts,
        prevalence,
        options.support_order,
    )
}

#[derive(Debug)]
struct PairedBootstrapScratch {
    positive_a: Vec<usize>,
    negative_a: Vec<usize>,
    positive_b: Vec<usize>,
    negative_b: Vec<usize>,
}

impl PairedBootstrapScratch {
    fn new(model_a: &Evaluation, model_b: &Evaluation) -> Self {
        Self {
            positive_a: vec![0; model_a.threshold_count()],
            negative_a: vec![0; model_a.threshold_count()],
            positive_b: vec![0; model_b.threshold_count()],
            negative_b: vec![0; model_b.threshold_count()],
        }
    }

    fn clear(&mut self) {
        self.positive_a.fill(0);
        self.negative_a.fill(0);
        self.positive_b.fill(0);
        self.negative_b.fill(0);
    }
}

#[allow(clippy::too_many_arguments)]
fn resampled_differences<R>(
    threshold_count_a: usize,
    threshold_count_b: usize,
    positive_a: &[usize],
    negative_a: &[usize],
    positive_b: &[usize],
    negative_b: &[usize],
    observed_counts: ClassCounts,
    replication_counts: ClassCounts,
    prevalences: &[ReferencePrevalence],
    rng: &mut R,
    scratch: &mut PairedBootstrapScratch,
) -> Vec<ChanceNormalizedDifference>
where
    R: Rng + ?Sized,
{
    debug_assert_eq!(scratch.positive_a.len(), threshold_count_a);
    debug_assert_eq!(scratch.positive_b.len(), threshold_count_b);
    scratch.clear();

    for _ in 0..replication_counts.positive() {
        let sampled = rng.gen_range(0..observed_counts.positive());
        scratch.positive_a[positive_a[sampled]] += 1;
        scratch.positive_b[positive_b[sampled]] += 1;
    }
    for _ in 0..replication_counts.negative() {
        let sampled = rng.gen_range(0..observed_counts.negative());
        scratch.negative_a[negative_a[sampled]] += 1;
        scratch.negative_b[negative_b[sampled]] += 1;
    }

    prevalences
        .iter()
        .map(|&prevalence| {
            let ap_a = prior_standardized_ap_from_counts(
                &scratch.positive_a,
                &scratch.negative_a,
                replication_counts,
                prevalence,
            );
            let ap_b = prior_standardized_ap_from_counts(
                &scratch.positive_b,
                &scratch.negative_b,
                replication_counts,
                prevalence,
            );
            let cnap_a = Cnap::from_value(cnap_from_prior_standardized_ap(ap_a, prevalence));
            let cnap_b = Cnap::from_value(cnap_from_prior_standardized_ap(ap_b, prevalence));
            ChanceNormalizedDifference::from_value(cnap_a.value() - cnap_b.value())
        })
        .collect()
}
