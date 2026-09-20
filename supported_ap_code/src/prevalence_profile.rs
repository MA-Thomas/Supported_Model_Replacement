//! A prepared, bounded CNAP difference. Tie probabilities depend on the
//! resampled ranking, not prevalence. Cache them within a fixed memory budget;
//! very large tie expansions are streamed to avoid quadratic retained memory.

use crate::{ClassCounts, enclosure::Enclosure};

#[derive(Clone, Copy)]
struct Term {
    weight: Enclosure,
    recall: Enclosure,
    fpr: Enclosure,
}

// At most 6 MiB of prepared terms per concurrently searched profile. The
// fallback retains only block counts and one hypergeometric support at a time.
const MAX_PREPARED_TERMS: usize = 131_072;

struct UnpreparedDifference {
    a: (Vec<usize>, Vec<usize>),
    b: (Vec<usize>, Vec<usize>),
    totals: ClassCounts,
}

pub(crate) struct PreparedDifference {
    constant: Enclosure,
    terms: Vec<Term>,
    streaming: Option<UnpreparedDifference>,
}

impl PreparedDifference {
    pub fn new(a: (&[usize], &[usize]), b: (&[usize], &[usize]), totals: ClassCounts) -> Self {
        Self::with_cache_limit(a, b, totals, MAX_PREPARED_TERMS)
    }

    fn with_cache_limit(
        a: (&[usize], &[usize]),
        b: (&[usize], &[usize]),
        totals: ClassCounts,
        limit: usize,
    ) -> Self {
        let mut result = Self {
            constant: Enclosure::ZERO,
            terms: Vec::new(),
            streaming: None,
        };
        if a == b {
            return result;
        }
        let estimate = |counts: (&[usize], &[usize])| {
            counts.0.iter().zip(counts.1).fold(0usize, |sum, (&a, &b)| {
                sum.saturating_add(
                    a.saturating_add(b)
                        .saturating_mul(a.min(b.saturating_add(1))),
                )
            })
        };
        if estimate(a).saturating_add(estimate(b)) > limit {
            result.streaming = Some(UnpreparedDifference {
                a: (a.0.to_vec(), a.1.to_vec()),
                b: (b.0.to_vec(), b.1.to_vec()),
                totals,
            });
        } else {
            let mut collect = |term: Term| {
                if term.fpr.hi == 0.0 {
                    result.constant = result.constant + term.weight;
                } else {
                    result.terms.push(term);
                }
            };
            visit_terms(a, totals, false, &mut collect);
            visit_terms(b, totals, true, &mut collect);
        }
        result
    }

    fn visit(&self, mut visitor: impl FnMut(Term)) {
        if let Some(source) = &self.streaming {
            visit_terms(
                (&source.a.0, &source.a.1),
                source.totals,
                false,
                &mut visitor,
            );
            visit_terms(
                (&source.b.0, &source.b.1),
                source.totals,
                true,
                &mut visitor,
            );
        } else {
            for &term in &self.terms {
                visitor(term);
            }
        }
    }

    pub fn point(&self, pi: f64) -> Enclosure {
        let mut sum = self.constant;
        self.visit(|term| sum = sum + term.weight * term.value(pi));
        sum
    }

    pub fn range(&self, left: f64, right: f64, fl: Enclosure, fr: Enclosure) -> Enclosure {
        let mut natural = self.constant;
        let mut derivative = Enclosure::ZERO;
        self.visit(|term| {
            // Each individual rational term is monotone, though their signed
            // sum need not be. Endpoint hulls enclose each term everywhere.
            natural = natural + term.weight * term.value(left).hull(term.value(right));
            if term.fpr.hi != 0.0 {
                let denominator = term.denominator(left).hull(term.denominator(right));
                derivative = derivative
                    + term.weight * term.fpr * (term.recall - term.fpr)
                        / (denominator * denominator).nonnegative();
            }
        });
        let width = Enclosure::point(right) - Enclosure::point(left);
        let offset = Enclosure {
            lo: 0.0,
            hi: width.hi,
        };
        // Mean value theorem; bounding the SUM of derivatives captures
        // cancellation between models and becomes tight around stationary points.
        natural
            .intersect(fl + derivative * offset)
            .intersect(fr - derivative * offset)
    }
}

impl Term {
    fn denominator(&self, pi: f64) -> Enclosure {
        let p = Enclosure::point(pi);
        p * self.recall + (Enclosure::ONE - p) * self.fpr
    }

    fn value(&self, pi: f64) -> Enclosure {
        if self.fpr.hi == 0.0 {
            return Enclosure::ONE;
        }
        Enclosure::point(pi) * (self.recall - self.fpr) / self.denominator(pi)
    }
}

fn visit_terms(
    counts: (&[usize], &[usize]),
    totals: ClassCounts,
    reverse: bool,
    visitor: &mut impl FnMut(Term),
) {
    let mut tp = 0;
    let mut fp = 0;
    for (&a, &b) in counts.0.iter().zip(counts.1) {
        let m = a + b;
        if a != 0 {
            let scale = Enclosure::ratio(a, totals.positive()) / Enclosure::integer(m);
            for k in 1..=m {
                let q = k - 1;
                for (t, probability) in hypergeometric_weights(a - 1, b, q) {
                    let positive = tp + t + 1;
                    let negative = fp + q - t;
                    // CNAP term h(pi)=pi(r-f)/(pi*r+(1-pi)*f), with
                    // exactly zero contribution when recall equals FPR.
                    if positive as u128 * totals.negative() as u128
                        == negative as u128 * totals.positive() as u128
                    {
                        continue;
                    }
                    let weight = (scale * probability).nonnegative();
                    visitor(Term {
                        weight: if reverse { -weight } else { weight },
                        recall: Enclosure::ratio(positive, totals.positive()),
                        fpr: Enclosure::ratio(negative, totals.negative()),
                    });
                }
            }
        }
        tp += a;
        fp += b;
    }
}

// Mode-centred relative probabilities, with outward rounding at every step.
// Include the complete support: an underflowed tail still has an upper bound.
pub(crate) fn hypergeometric_weights(a: usize, b: usize, draws: usize) -> Vec<(usize, Enclosure)> {
    let lower = draws.saturating_sub(b);
    let upper = draws.min(a);
    if lower == upper {
        return vec![(lower, Enclosure::ONE)];
    }
    let mode = (((draws as u128 + 1) * (a as u128 + 1)) / (a as u128 + b as u128 + 2)) as usize;
    let mode = mode.clamp(lower, upper);
    let mut weights = Vec::with_capacity(upper - lower + 1);
    weights.push((mode, Enclosure::ONE));
    let mut mass = Enclosure::ONE;
    let mut weight = Enclosure::ONE;
    for t in mode..upper {
        weight = (weight
            * Enclosure::ratio(a - t, t + 1)
            * Enclosure::ratio(draws - t, b - (draws - t) + 1))
        .nonnegative();
        mass = mass + weight;
        weights.push((t + 1, weight));
    }
    weight = Enclosure::ONE;
    for t in (lower + 1..=mode).rev() {
        weight = (weight
            * Enclosure::ratio(t, a - t + 1)
            * Enclosure::ratio(b - (draws - t), draws - t + 1))
        .nonnegative();
        mass = mass + weight;
        weights.push((t - 1, weight));
    }
    for (_, weight) in &mut weights {
        *weight = (*weight / mass).nonnegative();
    }
    weights
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Prevalence, metric::tie_averaged_cnap};

    #[test]
    fn large_expansions_stream_and_keep_the_same_mathematical_bounds() {
        let a = (&[1, 3, 0][..], &[0, 2, 1][..]);
        let b = (&[0, 4][..], &[3, 0][..]);
        let totals = ClassCounts::new(4, 3).unwrap();
        let cached = PreparedDifference::new(a, b, totals);
        let streamed = PreparedDifference::with_cache_limit(a, b, totals, 0);
        assert!(cached.streaming.is_none() && streamed.streaming.is_some());
        for pi in [0.01, 0.2, 0.9] {
            let p = Prevalence::new(pi).unwrap();
            let expected =
                tie_averaged_cnap(a.0, a.1, totals, p) - tie_averaged_cnap(b.0, b.1, totals, p);
            for profile in [&cached, &streamed] {
                let bound = profile.point(pi);
                assert!(bound.lo <= expected && expected <= bound.hi);
            }
        }
        let huge = PreparedDifference::new(
            (&[100_000], &[100_000]),
            (&[100_000, 0], &[0, 100_000]),
            ClassCounts::new(100_000, 100_000).unwrap(),
        );
        assert!(huge.streaming.is_some() && huge.terms.is_empty());
    }

    #[test]
    fn prepared_bounds_cover_explicit_tie_order_enumeration() {
        // Enumerate every binary ordering of a mixed tie, with a positive
        // preceding it and a negative following it. No hypergeometric oracle.
        for a in 1..=3 {
            for b in 1..=3 {
                let totals = ClassCounts::new(a + 1, b + 1).unwrap();
                let profile = PreparedDifference::new(
                    (&[1, a, 0], &[0, b, 1]),
                    (&[0, a + 1], &[b + 1, 0]),
                    totals,
                );
                for pi in [0.001, 0.125, 0.5, 0.875, 0.999] {
                    let mut values = Vec::new();
                    for mask in 0usize..(1 << (a + b)) {
                        if mask.count_ones() as usize != a {
                            continue;
                        }
                        let labels: Vec<_> = std::iter::once(true)
                            .chain((0..a + b).map(|i| mask & (1 << i) != 0))
                            .chain(std::iter::once(false))
                            .collect();
                        let mut tp = 0;
                        let mut fp = 0;
                        let mut cnap = 0.0;
                        for positive in labels {
                            if positive {
                                tp += 1;
                                let r = tp as f64 / (a + 1) as f64;
                                let f = fp as f64 / (b + 1) as f64;
                                cnap += pi * (r - f) / (pi * r + (1.0 - pi) * f) / (a + 1) as f64;
                            } else {
                                fp += 1;
                            }
                        }
                        // Reversed ranking's positive contributions, directly.
                        let reversed: f64 = (1..=a + 1)
                            .map(|tp| {
                                let r = tp as f64 / (a + 1) as f64;
                                pi * (r - 1.0) / (pi * r + 1.0 - pi) / (a + 1) as f64
                            })
                            .sum();
                        values.push(cnap - reversed);
                    }
                    let expected = values.iter().sum::<f64>() / values.len() as f64;
                    let bound = profile.point(pi);
                    assert!(
                        bound.lo <= expected && expected <= bound.hi,
                        "{a},{b},{pi}: {bound:?}, {expected}"
                    );
                    assert!(bound.hi - bound.lo < 1e-11);
                }
                for (left, right) in [(0.001, 0.999), (0.1, 0.2), (0.6, 0.601)] {
                    let bound =
                        profile.range(left, right, profile.point(left), profile.point(right));
                    for i in 0..=100 {
                        let pi = left + (right - left) * i as f64 / 100.0;
                        let point = profile.point(pi);
                        assert!(bound.lo <= point.hi && point.lo <= bound.hi);
                    }
                }
            }
        }
    }

    #[test]
    fn large_tie_probabilities_retain_tail_mass_and_enclose_mean() {
        for (a, b, draws) in [
            (999, 1000, 999),
            (1, 1999, 1000),
            (1999, 1, 1000),
            (0, 100, 40),
            (100, 0, 40),
        ] {
            let weights = hypergeometric_weights(a, b, draws);
            let mass = weights.iter().fold(Enclosure::ZERO, |s, &(_, p)| s + p);
            let mean = weights
                .iter()
                .fold(Enclosure::ZERO, |s, &(t, p)| s + p * Enclosure::integer(t));
            let expected = draws as f64 * a as f64 / (a + b) as f64;
            assert!(mass.lo <= 1.0 && 1.0 <= mass.hi);
            assert!(mean.lo <= expected && expected <= mean.hi);
            assert!(mass.hi - mass.lo < 1e-9);
        }
    }

    #[test]
    fn prepared_nontied_and_tied_profiles_match_existing_kernel() {
        let totals = ClassCounts::new(40, 60).unwrap();
        let a = (&[10, 30][..], &[40, 20][..]);
        let b = (&[20, 20][..], &[10, 50][..]);
        let profile = PreparedDifference::new(a, b, totals);
        for pi in [0.0001, 0.01, 0.1, 0.4, 0.9, 0.999] {
            let p = Prevalence::new(pi).unwrap();
            let expected =
                tie_averaged_cnap(a.0, a.1, totals, p) - tie_averaged_cnap(b.0, b.1, totals, p);
            let bounds = profile.point(pi);
            assert!(
                bounds.lo <= expected && expected <= bounds.hi,
                "{pi}: {bounds:?} vs {expected}"
            );
        }
    }
}
