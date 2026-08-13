use crate::{ClassCounts, Error, Prevalence};

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
        let mut position_sum = 0.0;
        for k in 1..=m {
            let q = k - 1;
            let lower = q.saturating_sub(b);
            let upper = q.min(a - 1);
            let mut probability = hypergeometric_probability(a - 1, b, q, lower);
            let mut conditional_precision = 0.0;
            for t in lower..=upper {
                let recall = (preceding_positive + t + 1) as f64 / n_positive;
                let fpr = (preceding_negative + q - t) as f64 / n_negative;
                let precision = pi * recall / (pi * recall + (1.0 - pi) * fpr);
                conditional_precision += probability * precision;
                if t < upper {
                    let numerator_left = (a - 1 - t) as f64;
                    let denominator_left = (t + 1) as f64;
                    let chosen_negative = q - t;
                    let numerator_right = chosen_negative as f64;
                    let denominator_right = (b - chosen_negative + 1) as f64;
                    probability *=
                        numerator_left / denominator_left * numerator_right / denominator_right;
                }
            }
            position_sum += conditional_precision;
        }
        block_terms.push(a as f64 / n_positive * position_sum / m as f64);
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

fn hypergeometric_probability(
    available_positive: usize,
    available_negative: usize,
    draws: usize,
    positive_draws: usize,
) -> f64 {
    let negative_draws = draws - positive_draws;
    let log_probability = log_choose(available_positive, positive_draws)
        + log_choose(available_negative, negative_draws)
        - log_choose(available_positive + available_negative, draws);
    log_probability.exp()
}

fn log_choose(n: usize, k: usize) -> f64 {
    if k > n {
        return f64::NEG_INFINITY;
    }
    let k = k.min(n - k);
    (1..=k)
        .map(|index| ((n - k + index) as f64).ln() - (index as f64).ln())
        .sum()
}

fn compensated_sum(values: &[f64]) -> f64 {
    let mut sum = 0.0;
    let mut correction = 0.0;
    for &value in values {
        let adjusted = value - correction;
        let next = sum + adjusted;
        correction = (next - sum) - adjusted;
        sum = next;
    }
    sum
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
