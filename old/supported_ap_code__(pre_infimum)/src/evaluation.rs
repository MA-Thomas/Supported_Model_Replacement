use crate::{
    BinaryLabel, ClassCounts, Cnap, PriorStandardizedAp, ReferencePrevalence, SupportedApError,
};

/// A validated, tie-block-indexed binary ranking evaluation.
///
/// Construction sorts scores once, groups exact equal scores into threshold
/// blocks, and records each observation's block. Bootstrap and permutation
/// estimators reuse this representation.
#[derive(Debug, Clone)]
#[must_use]
pub struct Evaluation {
    pub(crate) block_by_observation: Vec<usize>,
    pub(crate) positive_blocks: Vec<usize>,
    pub(crate) negative_blocks: Vec<usize>,
    pub(crate) observed_positive_counts: Vec<usize>,
    pub(crate) observed_negative_counts: Vec<usize>,
    counts: ClassCounts,
}

impl Evaluation {
    /// Construct an evaluation from numerical scores and labels convertible to
    /// [`BinaryLabel`], including `bool` and `BinaryLabel` itself.
    ///
    /// # Errors
    ///
    /// Returns an error for mismatched or empty inputs, non-finite scores, a
    /// missing outcome class, or an unrepresentable sample size.
    pub fn new<L>(scores: &[f64], labels: &[L]) -> Result<Self, SupportedApError>
    where
        L: Copy + Into<BinaryLabel>,
    {
        if scores.len() != labels.len() {
            return Err(SupportedApError::LengthMismatch {
                scores: scores.len(),
                labels: labels.len(),
            });
        }
        if scores.is_empty() {
            return Err(SupportedApError::EmptyEvaluation);
        }
        for (index, &score) in scores.iter().enumerate() {
            if !score.is_finite() {
                return Err(SupportedApError::InvalidScore {
                    index,
                    value: score,
                });
            }
        }

        let labels: Vec<BinaryLabel> = labels.iter().copied().map(Into::into).collect();
        Self::from_validated_labels(scores, &labels)
    }

    /// Construct an evaluation from labels encoded as zero and one.
    ///
    /// # Errors
    ///
    /// Returns the same validation errors as [`Self::new`], plus an error when
    /// a label is not zero or one.
    pub fn from_u8(scores: &[f64], labels: &[u8]) -> Result<Self, SupportedApError> {
        if scores.len() != labels.len() {
            return Err(SupportedApError::LengthMismatch {
                scores: scores.len(),
                labels: labels.len(),
            });
        }
        let labels = labels
            .iter()
            .copied()
            .map(BinaryLabel::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(scores, &labels)
    }

    fn from_validated_labels(
        scores: &[f64],
        labels: &[BinaryLabel],
    ) -> Result<Self, SupportedApError> {
        let positive = labels.iter().filter(|label| label.is_positive()).count();
        let negative = labels.len() - positive;
        if positive == 0 {
            return Err(SupportedApError::MissingClass {
                class: BinaryLabel::Positive,
            });
        }
        if negative == 0 {
            return Err(SupportedApError::MissingClass {
                class: BinaryLabel::Negative,
            });
        }
        let counts = ClassCounts::new(positive, negative)?;

        let mut order: Vec<usize> = (0..scores.len()).collect();
        order.sort_unstable_by(|&left, &right| scores[right].total_cmp(&scores[left]));

        let mut block_by_observation = vec![0; scores.len()];
        let mut block_scores = Vec::<f64>::new();
        for observation in order {
            let score = scores[observation];
            let block = match block_scores.last() {
                Some(&previous) if previous == score => block_scores.len() - 1,
                _ => {
                    block_scores.push(score);
                    block_scores.len() - 1
                }
            };
            block_by_observation[observation] = block;
        }

        let threshold_count = block_scores.len();
        let mut positive_blocks = Vec::with_capacity(positive);
        let mut negative_blocks = Vec::with_capacity(negative);
        let mut observed_positive_counts = vec![0; threshold_count];
        let mut observed_negative_counts = vec![0; threshold_count];

        for (observation, label) in labels.iter().copied().enumerate() {
            let block = block_by_observation[observation];
            if label.is_positive() {
                positive_blocks.push(block);
                observed_positive_counts[block] += 1;
            } else {
                negative_blocks.push(block);
                observed_negative_counts[block] += 1;
            }
        }

        Ok(Self {
            block_by_observation,
            positive_blocks,
            negative_blocks,
            observed_positive_counts,
            observed_negative_counts,
            counts,
        })
    }

    /// The observed positive and negative counts.
    #[inline]
    pub const fn class_counts(&self) -> ClassCounts {
        self.counts
    }

    /// The number of evaluation units.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.block_by_observation.len()
    }

    /// Whether the evaluation contains no units.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.block_by_observation.is_empty()
    }

    /// The number of distinct score values, which is the number of
    /// threshold blocks. Equal scores form one block.
    #[must_use]
    #[inline]
    pub fn threshold_count(&self) -> usize {
        self.observed_positive_counts.len()
    }

    /// The threshold-block index of every observation, in input order.
    ///
    /// Blocks are ordered by descending score and equal scores share a block.
    /// This view lets custom resampling regimes reuse the evaluation's exact tie
    /// semantics instead of reconstructing its private indexing algorithm.
    #[must_use]
    #[inline]
    pub fn block_indices(&self) -> &[usize] {
        &self.block_by_observation
    }

    /// Threshold-block indices of the positive observations, in input order.
    #[must_use]
    #[inline]
    pub fn positive_block_indices(&self) -> &[usize] {
        &self.positive_blocks
    }

    /// Threshold-block indices of the negative observations, in input order.
    #[must_use]
    #[inline]
    pub fn negative_block_indices(&self) -> &[usize] {
        &self.negative_blocks
    }

    /// Non-interpolated stepwise prior-standardized AP at the declared reference prevalence.
    pub fn prior_standardized_average_precision(
        &self,
        prevalence: ReferencePrevalence,
    ) -> PriorStandardizedAp {
        PriorStandardizedAp::from_value(prior_standardized_ap_from_counts(
            &self.observed_positive_counts,
            &self.observed_negative_counts,
            self.counts,
            prevalence,
        ))
    }

    /// Chance-normalized AP at the declared reference prevalence.
    pub fn cnap(&self, prevalence: ReferencePrevalence) -> Cnap {
        let ap = self
            .prior_standardized_average_precision(prevalence)
            .value();
        Cnap::from_value(cnap_from_prior_standardized_ap(ap, prevalence))
    }
}

#[inline]
pub(crate) fn cnap_from_prior_standardized_ap(ap: f64, prevalence: ReferencePrevalence) -> f64 {
    let prevalence = prevalence.value();
    if ap == 1.0 {
        return 1.0;
    }
    if ap == prevalence {
        return 0.0;
    }
    (ap - prevalence) / (1.0 - prevalence)
}

pub(crate) fn prior_standardized_ap_from_counts(
    positive_counts: &[usize],
    negative_counts: &[usize],
    totals: ClassCounts,
    prevalence: ReferencePrevalence,
) -> f64 {
    debug_assert_eq!(positive_counts.len(), negative_counts.len());

    let positive_total = totals.positive() as f64;
    let negative_total = totals.negative() as f64;
    let prevalence = prevalence.value();
    let mut true_positives = 0usize;
    let mut false_positives = 0usize;
    let mut average_precision = 0.0;

    for (&positive_increment, &negative_increment) in positive_counts.iter().zip(negative_counts) {
        true_positives += positive_increment;
        false_positives += negative_increment;

        if positive_increment == 0 {
            continue;
        }

        let recall = true_positives as f64 / positive_total;
        let false_positive_rate = false_positives as f64 / negative_total;
        let numerator = prevalence * recall;
        let precision = numerator / (numerator + (1.0 - prevalence) * false_positive_rate);
        let recall_increment = positive_increment as f64 / positive_total;
        average_precision += recall_increment * precision;
    }

    average_precision
}
