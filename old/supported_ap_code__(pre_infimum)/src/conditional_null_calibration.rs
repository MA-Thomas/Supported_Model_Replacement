use rand::seq::SliceRandom;

use crate::bootstrap::cnap_from_counts;
use crate::execution::map_init_indexed;
use crate::random::{PERMUTATION_BOOTSTRAP_DOMAIN, PERMUTATION_LABEL_DOMAIN, rng_for};
use crate::regime::{
    CanonicalFixedModel, FixedScoreResamplingRegime, ReplicateCounts, ReplicateRng,
};
use crate::support::support_in_place;
use crate::{
    BootstrapOptions, CalibratedSupportedCnap, CalibratedSupportedCnapEstimate,
    ConditionalNullCalibrationOptions, Evaluation, NullDistributionStorage, PermutationPValue,
    ReferencePrevalence, SupportedApError, SupportedCnap, SupportedCnapEstimate,
};

#[derive(Debug)]
struct PermutationScratch {
    indices: Vec<usize>,
    positive_blocks: Vec<usize>,
    negative_blocks: Vec<usize>,
    counts: ReplicateCounts,
    effects: Vec<Vec<f64>>,
    unit: (),
}

impl PermutationScratch {
    fn new(evaluation: &Evaluation, replicates: usize, prevalences: usize) -> Self {
        Self {
            indices: (0..evaluation.len()).collect(),
            positive_blocks: Vec::with_capacity(evaluation.class_counts().positive()),
            negative_blocks: Vec::with_capacity(evaluation.class_counts().negative()),
            counts: ReplicateCounts::new(evaluation.threshold_count(), evaluation.class_counts()),
            effects: vec![Vec::with_capacity(replicates); prevalences],
            unit: (),
        }
    }

    fn reset_indices(&mut self) {
        for (index, value) in self.indices.iter_mut().enumerate() {
            *value = index;
        }
    }
}

impl Evaluation {
    /// Run conditional-null calibration against the fixed-score permutation null.
    ///
    /// The headline score places the supported effect between the measured
    /// conditional chance level and perfect separation.
    ///
    /// # Errors
    ///
    /// Returns an error when bootstrap or permutation execution fails or the
    /// measured null reaches the fixed upper endpoint.
    pub fn conditionally_calibrated_supported_cnap(
        &self,
        prevalence: ReferencePrevalence,
        options: ConditionalNullCalibrationOptions,
    ) -> Result<CalibratedSupportedCnapEstimate, SupportedApError> {
        let mut profile =
            self.conditionally_calibrated_supported_cnap_profile(&[prevalence], options)?;
        Ok(profile.remove(0))
    }

    /// Calibrate at several reference prevalences from one set of draws.
    ///
    /// The null must be recomputed at every prevalence for which a calibrated
    /// value is reported. Block counts do not depend on the prevalence, so the
    /// `M * B` permutation draws are shared across the whole sweep and the
    /// profile costs what a single prevalence costs, plus the reduction.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty profile, invalid bootstrap configuration,
    /// execution failure, or a conditional null at the upper endpoint.
    pub fn conditionally_calibrated_supported_cnap_profile(
        &self,
        prevalences: &[ReferencePrevalence],
        options: ConditionalNullCalibrationOptions,
    ) -> Result<Vec<CalibratedSupportedCnapEstimate>, SupportedApError> {
        if prevalences.is_empty() {
            return Err(SupportedApError::EmptyPrevalenceProfile);
        }
        let estimates = self.supported_cnap_profile(prevalences, options.bootstrap)?;

        let permutation_count = options.permutation.permutations.get();
        let replicates = options.bootstrap.replicates.get();

        // Permutations are the parallel unit. Their bootstrap loops stay
        // sequential, which avoids nested pools and keeps scratch worker-local.
        let rows = map_init_indexed(
            permutation_count,
            options.permutation.parallelism,
            || PermutationScratch::new(self, replicates, prevalences.len()),
            |scratch, permutation| {
                permutation_supported(
                    self,
                    prevalences,
                    options.bootstrap,
                    options.permutation.seed,
                    permutation,
                    scratch,
                )
            },
        )?;

        // rows is indexed by permutation; transpose to per-prevalence columns.
        let mut nulls: Vec<Vec<SupportedCnap>> =
            vec![Vec::with_capacity(permutation_count); prevalences.len()];
        for row in rows {
            let row = row?;
            for (column, value) in nulls.iter_mut().zip(row) {
                column.push(value);
            }
        }

        let mut out = Vec::with_capacity(prevalences.len());
        for (estimate, null_values) in estimates.into_iter().zip(nulls) {
            out.push(assemble(estimate, null_values, permutation_count, options)?);
        }
        Ok(out)
    }
}

fn assemble(
    estimate: SupportedCnapEstimate,
    null_values: Vec<SupportedCnap>,
    permutation_count: usize,
    options: ConditionalNullCalibrationOptions,
) -> Result<CalibratedSupportedCnapEstimate, SupportedApError> {
    let null_mean =
        null_values.iter().map(|value| value.value()).sum::<f64>() / permutation_count as f64;
    let null = SupportedCnap::from_value(null_mean);
    let denominator = 1.0 - null_mean;
    if denominator <= 0.0 {
        return Err(SupportedApError::ConditionalNullAtUpperEndpoint { null: null_mean });
    }

    let supported = estimate.supported().value();
    let calibrated = CalibratedSupportedCnap::from_value((supported - null_mean) / denominator);

    // The null mean averages M independent permutation-level estimates, so its
    // Monte Carlo error is the usual sd / sqrt(M).
    debug_assert!(permutation_count >= 2);
    let variance = null_values
        .iter()
        .map(|value| {
            let deviation = value.value() - null_mean;
            deviation * deviation
        })
        .sum::<f64>()
        / (permutation_count - 1) as f64;
    let null_standard_error = (variance / permutation_count as f64).sqrt();

    let calibrated_standard_error = delta_method_standard_error(
        supported,
        null_mean,
        estimate.monte_carlo_standard_error(),
        null_standard_error,
    );

    let exceedances = null_values
        .iter()
        .filter(|value| value.value() >= supported)
        .count();
    let p_value =
        PermutationPValue::from_value((1 + exceedances) as f64 / (1 + permutation_count) as f64);

    let stored_null = match options.permutation.storage {
        NullDistributionStorage::SummaryOnly => None,
        NullDistributionStorage::Full => Some(null_values),
    };

    Ok(CalibratedSupportedCnapEstimate::new(
        estimate,
        null,
        calibrated,
        p_value,
        null_standard_error,
        calibrated_standard_error,
        options.permutation.permutations,
        stored_null,
    ))
}

/// Delta method through `C = (S - N) / (1 - N)`, treating the supported effect
/// and conditional null as independent because they draw from separate streams.
fn delta_method_standard_error(
    supported: f64,
    null: f64,
    supported_error: f64,
    null_error: f64,
) -> f64 {
    let denominator = 1.0 - null;
    debug_assert!(denominator > 0.0);
    debug_assert!(null_error.is_finite());
    let d2 = denominator * denominator;
    let d4 = d2 * d2;
    let from_supported = (supported_error * supported_error) / d2;
    let from_null = (supported - 1.0) * (supported - 1.0) * null_error * null_error / d4;
    (from_supported + from_null).sqrt()
}

/// One permutation: relabel, then run the canonical bootstrap once and reduce
/// it at every prevalence.
fn permutation_supported(
    evaluation: &Evaluation,
    prevalences: &[ReferencePrevalence],
    bootstrap: BootstrapOptions,
    permutation_seed: u64,
    permutation: usize,
    scratch: &mut PermutationScratch,
) -> Result<Vec<SupportedCnap>, SupportedApError> {
    scratch.reset_indices();
    scratch.positive_blocks.clear();
    scratch.negative_blocks.clear();
    for effects in scratch.effects.iter_mut() {
        effects.clear();
    }

    let mut label_rng = rng_for(permutation_seed, PERMUTATION_LABEL_DOMAIN, permutation, 0);
    scratch.indices.shuffle(&mut label_rng);

    let positive_count = evaluation.class_counts().positive();
    for &observation in &scratch.indices[..positive_count] {
        scratch
            .positive_blocks
            .push(evaluation.block_by_observation[observation]);
    }
    for &observation in &scratch.indices[positive_count..] {
        scratch
            .negative_blocks
            .push(evaluation.block_by_observation[observation]);
    }

    // Disjoint field borrows: the regime reads the block arrays while the
    // counts buffer and the effect columns are written.
    let regime = CanonicalFixedModel::new(
        &scratch.positive_blocks,
        &scratch.negative_blocks,
        evaluation.threshold_count(),
        evaluation.class_counts(),
    );
    let buffer = &mut scratch.counts;
    let columns = &mut scratch.effects;
    let unit = &mut scratch.unit;

    for replicate in 0..bootstrap.replicates.get() {
        let mut source = rng_for(
            bootstrap.seed,
            PERMUTATION_BOOTSTRAP_DOMAIN,
            permutation,
            replicate,
        );
        let mut rng = ReplicateRng::new(&mut source);
        regime.draw(replicate, &mut rng, unit, buffer)?;
        for (column, &prevalence) in columns.iter_mut().zip(prevalences) {
            column.push(cnap_from_counts(buffer, prevalence).value());
        }
    }

    Ok(columns
        .iter_mut()
        .map(|column| SupportedCnap::from_value(support_in_place(column, bootstrap.support_order)))
        .collect())
}
