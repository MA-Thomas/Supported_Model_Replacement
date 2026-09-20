use crate::{ClassCounts, Error, Prevalence};

/// Observed CNAP using the same tie averaging kernel as the paired empirical gate.
pub fn observed_cnap(
    scores: &[f64],
    labels: &[bool],
    prevalence: Prevalence,
) -> Result<f64, Error> {
    let (positive, negative, counts) = observed_blocks(scores, labels)?;
    let value = tie_averaged_cnap(&positive, &negative, counts, prevalence);
    if !value.is_finite() {
        return Err(Error::InvalidEffect { index: 0, value });
    }
    Ok(value)
}

/// Twice the observed Mann–Whitney credit, before division by the common
/// positive/negative pair count. Equal scores receive one unit of credit.
/// The bound also keeps signed paired gain accumulation within `i64`.
pub fn observed_auroc_order_key(scores: &[f64], labels: &[bool]) -> Result<u64, Error> {
    let (positive, negative, counts) = observed_blocks(scores, labels)?;
    let pairs = (counts.positive() as u64)
        .checked_mul(counts.negative() as u64)
        .and_then(|n| n.checked_mul(2))
        .ok_or(Error::InvalidClassCounts)?;
    if pairs > i64::MAX as u64 {
        return Err(Error::InvalidClassCounts);
    }
    let mut below = counts.negative() as u64;
    let mut credit = 0;
    for (p, n) in positive.into_iter().zip(negative) {
        below -= n as u64;
        credit += p as u64 * (2 * below + n as u64);
    }
    Ok(credit)
}

fn observed_blocks(
    scores: &[f64],
    labels: &[bool],
) -> Result<(Vec<usize>, Vec<usize>, ClassCounts), Error> {
    if scores.len() != labels.len() {
        return Err(Error::PairedLengthMismatch);
    }
    let ranking = Ranking::new(scores)?;
    let positive = labels.iter().filter(|&&label| label).count();
    ClassCounts::new(positive, labels.len() - positive)?;
    Ok(ranking.block_counts(labels, &vec![1; labels.len()]))
}

#[cfg(test)]
mod observed_order_tests {
    use super::*;
    use crate::{PairedEvaluation, paired_auroc_difference};

    #[test]
    fn order_primitives_agree_exactly_with_paired_kernels_including_ties() {
        let labels = [true, false, true, false, true, false];
        let scores = [
            [1.0, 0.0, 0.8, 0.2, 0.6, 0.4],
            [0.0, 1.0, 0.2, 0.8, 0.4, 0.6],
            [0.0, -0.0, 0.0, -0.0, 0.0, -0.0],
            [1.0, 1.0, 0.0, 0.0, 0.0, 0.0],
            [1.0, 0.0, 1.0, 1.0, 1.0, 0.0],
        ];
        for a in &scores {
            for b in &scores {
                let paired = PairedEvaluation::new(a, b, &labels).unwrap();
                let credit_a = observed_auroc_order_key(a, &labels).unwrap();
                let credit_b = observed_auroc_order_key(b, &labels).unwrap();
                assert_eq!(
                    paired_auroc_difference(&paired),
                    ((credit_a as i64 - credit_b as i64) as f64 / 2.0) / 9.0
                );
                for pi in [0.001, 0.01, 0.1, 0.3, 0.5, 0.999] {
                    let pi = Prevalence::new(pi).unwrap();
                    assert_eq!(
                        paired.tie_averaged_difference(pi),
                        observed_cnap(a, &labels, pi).unwrap()
                            - observed_cnap(b, &labels, pi).unwrap()
                    );
                }
            }
        }
    }

    #[test]
    fn order_primitives_reject_invalid_inputs() {
        let pi = Prevalence::new(0.1).unwrap();
        assert!(observed_cnap(&[1.0], &[true, false], pi).is_err());
        assert!(observed_auroc_order_key(&[1.0, 2.0], &[true, true]).is_err());
        assert!(observed_cnap(&[f64::NAN, 2.0], &[true, false], pi).is_err());
        assert!(observed_auroc_order_key(&[], &[]).is_err());
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Ranking {
    pub(crate) block_by_observation: Vec<usize>,
    pub(crate) block_count: usize,
}

impl Ranking {
    pub(crate) fn new(scores: &[f64]) -> Result<Self, Error> {
        if scores.is_empty() {
            return Err(Error::EmptyEvaluation);
        }
        for (index, &value) in scores.iter().enumerate() {
            if !value.is_finite() {
                return Err(Error::InvalidScore { index, value });
            }
        }
        let mut order: Vec<usize> = (0..scores.len()).collect();
        order.sort_unstable_by(|&left, &right| scores[right].total_cmp(&scores[left]));
        let mut block_by_observation = vec![0; scores.len()];
        let mut block_scores = Vec::new();
        for observation in order {
            let score = scores[observation];
            let block = match block_scores.last() {
                Some(previous) if *previous == score => block_scores.len() - 1,
                _ => {
                    block_scores.push(score);
                    block_scores.len() - 1
                }
            };
            block_by_observation[observation] = block;
        }
        Ok(Self {
            block_by_observation,
            block_count: block_scores.len(),
        })
    }

    pub(crate) fn block_counts(
        &self,
        labels: &[bool],
        multiplicities: &[usize],
    ) -> (Vec<usize>, Vec<usize>, ClassCounts) {
        debug_assert_eq!(labels.len(), self.block_by_observation.len());
        debug_assert_eq!(labels.len(), multiplicities.len());
        let mut positive = vec![0usize; self.block_count];
        let mut negative = vec![0usize; self.block_count];
        let mut positive_total = 0usize;
        let mut negative_total = 0usize;
        for (observation, (&label, &multiplicity)) in labels.iter().zip(multiplicities).enumerate()
        {
            let block = self.block_by_observation[observation];
            if label {
                positive[block] += multiplicity;
                positive_total += multiplicity;
            } else {
                negative[block] += multiplicity;
                negative_total += multiplicity;
            }
        }
        let counts = ClassCounts::new(positive_total, negative_total)
            .expect("resampling preserves both outcome classes");
        (positive, negative, counts)
    }
}

#[cfg(test)]
pub(crate) fn ordinary_ap(
    positive_counts: &[usize],
    negative_counts: &[usize],
    totals: ClassCounts,
    prevalence: Prevalence,
) -> f64 {
    let n_positive = totals.positive() as f64;
    let n_negative = totals.negative() as f64;
    let pi = prevalence.get();
    let mut tp = 0usize;
    let mut fp = 0usize;
    let mut terms = Vec::with_capacity(positive_counts.len());
    for (&positive, &negative) in positive_counts.iter().zip(negative_counts) {
        tp += positive;
        fp += negative;
        if positive == 0 {
            continue;
        }
        let recall = tp as f64 / n_positive;
        let false_positive_rate = fp as f64 / n_negative;
        let precision = pi * recall / (pi * recall + (1.0 - pi) * false_positive_rate);
        terms.push(positive as f64 / n_positive * precision);
    }
    compensated_sum(&terms)
}

pub(crate) fn tie_averaged_ap(
    positive_counts: &[usize],
    negative_counts: &[usize],
    totals: ClassCounts,
    prevalence: Prevalence,
) -> f64 {
    let n_positive = totals.positive() as f64;
    let n_negative = totals.negative() as f64;
    let pi = prevalence.get();
    let mut preceding_positive = 0usize;
    let mut preceding_negative = 0usize;
    let mut block_terms = Vec::with_capacity(positive_counts.len());

    for (&a, &b) in positive_counts.iter().zip(negative_counts) {
        let m = a + b;
        if a == 0 {
            preceding_negative += b;
            continue;
        }
        let mut position_sum = CompensatedSum::default();
        for k in 1..=m {
            let q = k - 1;
            let conditional_precision = hypergeometric_expectation(a - 1, b, q, |t| {
                let recall = (preceding_positive + t + 1) as f64 / n_positive;
                let fpr = (preceding_negative + q - t) as f64 / n_negative;
                pi * recall / (pi * recall + (1.0 - pi) * fpr)
            });
            position_sum.add(conditional_precision);
        }
        block_terms.push(a as f64 / n_positive * position_sum.total / m as f64);
        preceding_positive += a;
        preceding_negative += b;
    }
    compensated_sum(&block_terms)
}

pub(crate) fn tie_averaged_cnap(
    positive_counts: &[usize],
    negative_counts: &[usize],
    totals: ClassCounts,
    prevalence: Prevalence,
) -> f64 {
    let pi = prevalence.get();
    let ap = tie_averaged_ap(positive_counts, negative_counts, totals, prevalence);
    if ap == 1.0 {
        1.0
    } else if ap == pi {
        0.0
    } else {
        (ap - pi) / (1.0 - pi)
    }
}

/// Average a bounded function under the hypergeometric law without seeding a
/// recurrence at a possibly underflowed tail probability. The mode has relative
/// weight one; both outward recurrences decrease, and normalization cancels the
/// unknown probability at the mode. No logarithms or binomial coefficients are
/// needed. The caller supplies a valid draw count and finite precisions in [0,1].
fn hypergeometric_expectation(
    available_positive: usize,
    available_negative: usize,
    draws: usize,
    mut evaluate: impl FnMut(usize) -> f64,
) -> f64 {
    debug_assert!(draws <= available_positive + available_negative);
    let lower = draws.saturating_sub(available_negative);
    let upper = draws.min(available_positive);
    // Widen before multiplication so finding the integer mode cannot overflow
    // usize, and do not round the mode through floating point.
    let mode = (((draws as u128 + 1) * (available_positive as u128 + 1))
        / (available_positive as u128 + available_negative as u128 + 2)) as usize;
    let mode = mode.clamp(lower, upper);
    let mut mass = CompensatedSum::default();
    let mut weighted = CompensatedSum::default();
    mass.add(1.0);
    weighted.add(evaluate(mode));

    let mut weight = 1.0;
    for t in mode..upper {
        let negative_draws = draws - t;
        weight *= (available_positive - t) as f64 / (t + 1) as f64
            * (negative_draws as f64 / (available_negative - negative_draws + 1) as f64);
        if weight == 0.0 {
            break;
        }
        mass.add(weight);
        weighted.add(weight * evaluate(t + 1));
    }

    weight = 1.0;
    for t in (lower + 1..=mode).rev() {
        let negative_draws = draws - t;
        weight *= t as f64 / (available_positive - t + 1) as f64
            * ((available_negative - negative_draws) as f64 / (negative_draws + 1) as f64);
        if weight == 0.0 {
            break;
        }
        mass.add(weight);
        weighted.add(weight * evaluate(t - 1));
    }
    weighted.total / mass.total
}

#[derive(Default)]
struct CompensatedSum {
    total: f64,
    correction: f64,
}

impl CompensatedSum {
    fn add(&mut self, value: f64) {
        let adjusted = value - self.correction;
        let next = self.total + adjusted;
        self.correction = (next - self.total) - adjusted;
        self.total = next;
    }
}

fn compensated_sum(values: &[f64]) -> f64 {
    let mut sum = CompensatedSum::default();
    for &value in values {
        sum.add(value);
    }
    sum.total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_form_tie_average_matches_enumeration() {
        let counts = ClassCounts::new(2, 2).unwrap();
        for prevalence in [0.05, 0.2, 0.5, 0.9].map(|value| Prevalence::new(value).unwrap()) {
            let exact = tie_averaged_ap(&[2], &[2], counts, prevalence);
            let arrangements = binary_arrangements(2, 2);
            let enumerated = arrangements
                .iter()
                .map(|labels| {
                    let positive: Vec<usize> =
                        labels.iter().map(|label| usize::from(*label)).collect();
                    let negative: Vec<usize> =
                        labels.iter().map(|label| usize::from(!*label)).collect();
                    ordinary_ap(&positive, &negative, counts, prevalence)
                })
                .sum::<f64>()
                / arrangements.len() as f64;
            assert!((exact - enumerated).abs() < 1e-13, "{prevalence:?}");
        }
    }

    #[test]
    fn multiple_tie_blocks_match_cartesian_enumeration() {
        let counts = ClassCounts::new(2, 2).unwrap();
        let prevalence = Prevalence::new(0.3).unwrap();
        let exact = tie_averaged_ap(&[1, 1], &[1, 1], counts, prevalence);
        let blocks = binary_arrangements(1, 1);
        let mut total = 0.0;
        let mut count = 0;
        for first in &blocks {
            for second in &blocks {
                let labels: Vec<bool> = first.iter().chain(second).copied().collect();
                let positive: Vec<usize> = labels.iter().map(|label| usize::from(*label)).collect();
                let negative: Vec<usize> =
                    labels.iter().map(|label| usize::from(!*label)).collect();
                total += ordinary_ap(&positive, &negative, counts, prevalence);
                count += 1;
            }
        }
        assert!((exact - total / count as f64).abs() < 1e-13);
    }

    #[test]
    fn large_all_tied_scores_match_finite_random_ranking_identity() {
        // At the observed prevalence, E[AP] = P/N +
        // (N-P)(H_N-1)/(N(N-1)), so E[CNAP] = (H_N-1)/(N-1).
        // This oracle does not use hypergeometric probabilities.
        for (positive, negative) in [
            (1000, 1000),
            (2000, 2000),
            (200, 1800),
            (1800, 200),
            (1, 1999),
            (1999, 1),
        ] {
            let n = positive + negative;
            let labels: Vec<_> = (0..n).map(|index| index < positive).collect();
            let prevalence = Prevalence::new(positive as f64 / n as f64).unwrap();
            let actual = observed_cnap(&vec![0.0; n], &labels, prevalence).unwrap();
            let harmonic = compensated_sum(&(1..=n).map(|k| 1.0 / k as f64).collect::<Vec<_>>());
            let expected = (harmonic - 1.0) / (n - 1) as f64;
            assert!(
                (actual - expected).abs() < 2e-12,
                "P={positive}, N_negative={negative}: actual={actual}, expected={expected}"
            );
        }
    }

    #[test]
    fn hypergeometric_normalization_and_mean_cover_large_and_degenerate_supports() {
        for (positive, negative) in [
            (0, 0),
            (0, 1000),
            (1000, 0),
            (1, 1999),
            (1999, 1),
            (999, 1000),
        ] {
            let population = positive + negative;
            for draws in [0, population / 2, population] {
                let mass = hypergeometric_expectation(positive, negative, draws, |_| 1.0);
                assert_eq!(mass, 1.0);
                let mean = hypergeometric_expectation(positive, negative, draws, |t| {
                    t as f64 / (positive + 1) as f64
                });
                let expected = if population == 0 {
                    0.0
                } else {
                    draws as f64 / population as f64 * positive as f64 / (positive + 1) as f64
                };
                assert!((mean - expected).abs() < 2e-14);
            }
        }
    }

    #[test]
    fn unequal_tie_blocks_and_target_prevalences_match_enumeration() {
        for a in 1..=4 {
            for b in 1..=4 {
                // Include positives and negatives on either side of the tie.
                let totals = ClassCounts::new(a + 2, b + 2).unwrap();
                let arrangements = binary_arrangements(a, b);
                for pi in [0.001, 0.1, 0.5, 0.9, 0.999] {
                    let prevalence = Prevalence::new(pi).unwrap();
                    let actual = tie_averaged_ap(&[1, a, 1], &[1, b, 1], totals, prevalence);
                    let expected = arrangements
                        .iter()
                        .map(|labels| {
                            // Keep the flanking blocks tie-averaged as well.
                            let mut total = 0.0;
                            for first in binary_arrangements(1, 1) {
                                for last in binary_arrangements(1, 1) {
                                    let expanded: Vec<_> =
                                        first.iter().chain(labels).chain(&last).copied().collect();
                                    let p: Vec<_> =
                                        expanded.iter().map(|&y| usize::from(y)).collect();
                                    let n: Vec<_> =
                                        expanded.iter().map(|&y| usize::from(!y)).collect();
                                    total += ordinary_ap(&p, &n, totals, prevalence);
                                }
                            }
                            total / 4.0
                        })
                        .sum::<f64>()
                        / arrangements.len() as f64;
                    assert!((actual - expected).abs() < 2e-13, "a={a}, b={b}, pi={pi}");
                }
            }
        }
    }

    fn binary_arrangements(positive: usize, negative: usize) -> Vec<Vec<bool>> {
        fn visit(
            positive: usize,
            negative: usize,
            current: &mut Vec<bool>,
            out: &mut Vec<Vec<bool>>,
        ) {
            if positive == 0 && negative == 0 {
                out.push(current.clone());
                return;
            }
            if positive > 0 {
                current.push(true);
                visit(positive - 1, negative, current, out);
                current.pop();
            }
            if negative > 0 {
                current.push(false);
                visit(positive, negative - 1, current, out);
                current.pop();
            }
        }
        let mut out = Vec::new();
        visit(positive, negative, &mut Vec::new(), &mut out);
        out
    }
}
