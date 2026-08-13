//! Pairwise gain matrix and its multiplicity-weighted marginals.
//!
//! The gain matrix is built once from the observed evaluation and never rebuilt.
//! Projected computational resampling is carried as multiplicity vectors over
//! the *original* holdout index space, so every sweep runs against one
//! contiguous allocation.
//!
//! Entries are stored doubled (`i8` in `-2..=2`) so tie half-credit is exact in
//! integer arithmetic. Marginal sums stay in doubled integer units and are
//! converted to `f64` once, at the point of use.

use crate::PairedEvaluation;

/// Number of distinct doubled gain values: `-2, -1, 0, 1, 2`.
pub(crate) const GAIN_LEVELS: usize = 5;

/// Doubled gain value carried by histogram bin `level`.
#[inline]
pub(crate) const fn level_value(level: usize) -> i64 {
    level as i64 - 2
}

#[derive(Debug, Clone)]
pub(crate) struct GainMatrix {
    /// Doubled gains, row-major over positives.
    values: Box<[i8]>,
    positive_count: usize,
    negative_count: usize,
}

impl GainMatrix {
    pub(crate) fn new(evaluation: &PairedEvaluation) -> Self {
        let positive: Vec<_> = evaluation
            .labels
            .iter()
            .enumerate()
            .filter_map(|(index, &label)| label.then_some(index))
            .collect();
        let negative: Vec<_> = evaluation
            .labels
            .iter()
            .enumerate()
            .filter_map(|(index, &label)| (!label).then_some(index))
            .collect();
        let mut values = Vec::with_capacity(positive.len() * negative.len());
        for &i in &positive {
            for &k in &negative {
                let gain_a = pair_credit(evaluation.scores_a[i], evaluation.scores_a[k]);
                let gain_b = pair_credit(evaluation.scores_b[i], evaluation.scores_b[k]);
                values.push(gain_a - gain_b);
            }
        }
        Self {
            values: values.into_boxed_slice(),
            positive_count: positive.len(),
            negative_count: negative.len(),
        }
    }

    pub(crate) fn positive_count(&self) -> usize {
        self.positive_count
    }

    pub(crate) fn negative_count(&self) -> usize {
        self.negative_count
    }

    /// Contiguous doubled gains for one positive case.
    #[inline]
    pub(crate) fn row(&self, positive: usize) -> &[i8] {
        let start = positive * self.negative_count;
        &self.values[start..start + self.negative_count]
    }

    /// Undoubled gain for one pair.
    #[inline]
    #[cfg(any(test, feature = "highs-reference"))]
    pub(crate) fn get(&self, positive: usize, negative: usize) -> f64 {
        f64::from(self.values[positive * self.negative_count + negative]) / 2.0
    }

    /// Multiplicities placing every observed case exactly once.
    pub(crate) fn identity_multiplicities(&self) -> Multiplicities {
        Multiplicities::uniform(self.positive_count, self.negative_count)
    }
}

#[inline]
fn pair_credit(positive_score: f64, negative_score: f64) -> i8 {
    if positive_score > negative_score {
        2
    } else if positive_score == negative_score {
        1
    } else {
        0
    }
}

/// How many times each original case appears in an evaluation.
///
/// Class totals are the *design* sizes `n_+` and `n_-`. Under the same-design
/// resampling required by the appendix they equal the observed class counts, so
/// the concentration cap `Gamma / n_y` is identical across replications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Multiplicities {
    positive: Box<[u32]>,
    negative: Box<[u32]>,
    positive_total: u64,
    negative_total: u64,
}

impl Multiplicities {
    pub(crate) fn new(positive: Box<[u32]>, negative: Box<[u32]>) -> Self {
        let positive_total = positive.iter().map(|&count| u64::from(count)).sum();
        let negative_total = negative.iter().map(|&count| u64::from(count)).sum();
        Self {
            positive,
            negative,
            positive_total,
            negative_total,
        }
    }

    pub(crate) fn uniform(positive_count: usize, negative_count: usize) -> Self {
        Self::new(
            vec![1u32; positive_count].into_boxed_slice(),
            vec![1u32; negative_count].into_boxed_slice(),
        )
    }

    #[inline]
    pub(crate) fn positive(&self) -> &[u32] {
        &self.positive
    }

    #[inline]
    pub(crate) fn negative(&self) -> &[u32] {
        &self.negative
    }

    /// Design size `n_+`, counting repeats.
    #[inline]
    pub(crate) fn positive_total(&self) -> f64 {
        self.positive_total as f64
    }

    /// Design size `n_-`, counting repeats.
    #[inline]
    pub(crate) fn negative_total(&self) -> f64 {
        self.negative_total as f64
    }

    /// Smallest positive multiplicity on the empirical support.
    pub(crate) fn positive_minimum(&self) -> u32 {
        self.positive
            .iter()
            .copied()
            .filter(|&count| count > 0)
            .min()
            .unwrap_or(0)
    }

    /// Smallest negative multiplicity on the empirical support.
    pub(crate) fn negative_minimum(&self) -> u32 {
        self.negative
            .iter()
            .copied()
            .filter(|&count| count > 0)
            .min()
            .unwrap_or(0)
    }

    pub(crate) fn is_degenerate(&self) -> bool {
        self.positive_total == 0 || self.negative_total == 0
    }
}

/// Multiplicity-weighted marginals of the gain matrix.
///
/// Every field is independent of the concentration factor, so this is computed
/// once per replication and reused across the whole `Gamma` search. That reuse
/// is why search cost is dominated by the number of replications rather than by
/// the number of searched factors.
#[derive(Debug, Clone)]
pub(crate) struct Marginals {
    /// `R_i = sum_k m_k G_ik`, doubled.
    row: Box<[i64]>,
    /// `C_k = sum_i m_i G_ik`, doubled.
    col: Box<[i64]>,
    /// `T = sum_ik m_i m_k G_ik`, doubled.
    total: i128,
    /// Per positive case, negative multiplicity mass at each gain level.
    row_levels: Box<[[u64; GAIN_LEVELS]]>,
    /// Per negative case, positive multiplicity mass at each gain level.
    col_levels: Box<[[u64; GAIN_LEVELS]]>,
    positive_support: Box<[u32]>,
    negative_support: Box<[u32]>,
}

impl Marginals {
    /// Single pass over the matrix restricted to the multiplicity support.
    ///
    /// The positive index is the outer loop so each row is read contiguously.
    pub(crate) fn new(matrix: &GainMatrix, multiplicities: &Multiplicities) -> Self {
        let positive_counts = multiplicities.positive();
        let negative_counts = multiplicities.negative();

        let positive_support: Vec<u32> = positive_counts
            .iter()
            .enumerate()
            .filter_map(|(index, &count)| (count > 0).then_some(index as u32))
            .collect();
        let negative_support: Vec<u32> = negative_counts
            .iter()
            .enumerate()
            .filter_map(|(index, &count)| (count > 0).then_some(index as u32))
            .collect();

        let mut row = vec![0i64; matrix.positive_count()];
        let mut col = vec![0i64; matrix.negative_count()];
        let mut row_levels = vec![[0u64; GAIN_LEVELS]; matrix.positive_count()];
        let mut col_levels = vec![[0u64; GAIN_LEVELS]; matrix.negative_count()];
        let mut total: i128 = 0;

        for &positive in &positive_support {
            let positive = positive as usize;
            let positive_mass = u64::from(positive_counts[positive]);
            let values = matrix.row(positive);
            let mut row_sum = 0i64;
            let row_level = &mut row_levels[positive];
            for &negative in &negative_support {
                let negative = negative as usize;
                let negative_mass = u64::from(negative_counts[negative]);
                let gain = i64::from(values[negative]);
                row_sum += gain * negative_mass as i64;
                col[negative] += gain * positive_mass as i64;
                let level = (gain + 2) as usize;
                row_level[level] += negative_mass;
                col_levels[negative][level] += positive_mass;
            }
            row[positive] = row_sum;
            total += i128::from(row_sum) * i128::from(positive_mass);
        }

        Self {
            row: row.into_boxed_slice(),
            col: col.into_boxed_slice(),
            total,
            row_levels: row_levels.into_boxed_slice(),
            col_levels: col_levels.into_boxed_slice(),
            positive_support: positive_support.into_boxed_slice(),
            negative_support: negative_support.into_boxed_slice(),
        }
    }

    /// Ordinary paired AUROC difference under these multiplicities.
    ///
    /// This is the analytic value of the adversarial problem at `Gamma = 1`,
    /// where normalization forces uniform within-class weights.
    pub(crate) fn uniform_difference(&self, multiplicities: &Multiplicities) -> f64 {
        let denominator = multiplicities.positive_total() * multiplicities.negative_total();
        (self.total as f64 / 2.0) / denominator
    }

    /// Least favorable single pair, the analytic value once both caps saturate.
    pub(crate) fn minimum_entry(&self, matrix: &GainMatrix) -> f64 {
        let mut minimum = i8::MAX;
        for &positive in &self.positive_support {
            let values = matrix.row(positive as usize);
            for &negative in &self.negative_support {
                let gain = values[negative as usize];
                if gain < minimum {
                    minimum = gain;
                }
            }
        }
        f64::from(minimum) / 2.0
    }

    #[inline]
    pub(crate) fn row_sums(&self) -> &[i64] {
        &self.row
    }

    #[inline]
    pub(crate) fn col_sums(&self) -> &[i64] {
        &self.col
    }

    #[inline]
    pub(crate) fn row_levels(&self) -> &[[u64; GAIN_LEVELS]] {
        &self.row_levels
    }

    #[inline]
    pub(crate) fn col_levels(&self) -> &[[u64; GAIN_LEVELS]] {
        &self.col_levels
    }

    #[inline]
    pub(crate) fn total(&self) -> i128 {
        self.total
    }

    #[inline]
    pub(crate) fn positive_support(&self) -> &[u32] {
        &self.positive_support
    }

    #[inline]
    pub(crate) fn negative_support(&self) -> &[u32] {
        &self.negative_support
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evaluation() -> PairedEvaluation {
        PairedEvaluation::new(
            &[2.0, 1.0, 1.0, 0.0],
            &[1.0, 1.0, 2.0, 0.0],
            &[true, false, true, false],
        )
        .unwrap()
    }

    #[test]
    fn gain_matrix_uses_half_credit_for_ties() {
        let matrix = GainMatrix::new(&evaluation());
        assert_eq!(matrix.get(0, 0), 0.5);
        assert_eq!(matrix.get(1, 0), -0.5);
        assert_eq!(matrix.get(0, 1), 0.0);
        assert_eq!(matrix.get(1, 1), 0.0);
        let identity = matrix.identity_multiplicities();
        let marginals = Marginals::new(&matrix, &identity);
        assert_eq!(marginals.uniform_difference(&identity), 0.0);
    }

    #[test]
    fn marginals_agree_with_direct_summation() {
        let matrix = GainMatrix::new(&evaluation());
        let multiplicities = Multiplicities::new(
            vec![3u32, 1].into_boxed_slice(),
            vec![2u32, 5].into_boxed_slice(),
        );
        let marginals = Marginals::new(&matrix, &multiplicities);
        let mut total = 0.0;
        let mut rows = [0.0; 2];
        let mut cols = [0.0; 2];
        for (i, row_total) in rows.iter_mut().enumerate() {
            for (k, col_total) in cols.iter_mut().enumerate() {
                let mass = f64::from(multiplicities.positive()[i] * multiplicities.negative()[k]);
                total += mass * matrix.get(i, k);
                *row_total += f64::from(multiplicities.negative()[k]) * matrix.get(i, k);
                *col_total += f64::from(multiplicities.positive()[i]) * matrix.get(i, k);
            }
        }
        assert!((marginals.total() as f64 / 2.0 - total).abs() < 1e-12);
        for (i, row_total) in rows.iter().enumerate() {
            assert!((marginals.row_sums()[i] as f64 / 2.0 - row_total).abs() < 1e-12);
        }
        for (k, col_total) in cols.iter().enumerate() {
            assert!((marginals.col_sums()[k] as f64 / 2.0 - col_total).abs() < 1e-12);
        }
        assert!(
            (marginals.uniform_difference(&multiplicities) - total / (4.0 * 7.0)).abs() < 1e-12
        );
    }

    #[test]
    fn level_histograms_partition_the_opposite_class_mass() {
        let matrix = GainMatrix::new(&evaluation());
        let multiplicities = Multiplicities::new(
            vec![3u32, 1].into_boxed_slice(),
            vec![2u32, 5].into_boxed_slice(),
        );
        let marginals = Marginals::new(&matrix, &multiplicities);
        for &positive in marginals.positive_support() {
            let bins = marginals.row_levels()[positive as usize];
            assert_eq!(bins.iter().sum::<u64>(), 7);
            let weighted: i64 = (0..GAIN_LEVELS)
                .map(|level| level_value(level) * bins[level] as i64)
                .sum();
            assert_eq!(weighted, marginals.row_sums()[positive as usize]);
        }
        for &negative in marginals.negative_support() {
            let bins = marginals.col_levels()[negative as usize];
            assert_eq!(bins.iter().sum::<u64>(), 4);
            let weighted: i64 = (0..GAIN_LEVELS)
                .map(|level| level_value(level) * bins[level] as i64)
                .sum();
            assert_eq!(weighted, marginals.col_sums()[negative as usize]);
        }
    }

    #[test]
    fn zero_multiplicity_cases_leave_the_support() {
        let matrix = GainMatrix::new(&evaluation());
        let multiplicities = Multiplicities::new(
            vec![0u32, 2].into_boxed_slice(),
            vec![4u32, 0].into_boxed_slice(),
        );
        let marginals = Marginals::new(&matrix, &multiplicities);
        assert_eq!(marginals.positive_support(), &[1]);
        assert_eq!(marginals.negative_support(), &[0]);
        // Only the (1, 0) pair survives, with gain -0.5 and mass 2 * 4.
        assert!((marginals.total() as f64 / 2.0 - (-0.5 * 8.0)).abs() < 1e-12);
        assert_eq!(marginals.minimum_entry(&matrix), -0.5);
        assert!((marginals.uniform_difference(&multiplicities) - (-0.5)).abs() < 1e-12);
    }
}
