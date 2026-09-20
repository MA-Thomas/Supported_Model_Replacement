//! Outward-rounded reductions for interval-search certificates. The support
//! operators are coordinatewise monotone, so reducing lower/upper effect lists
//! bounds the final decision, including anchored nested challenges.

use crate::{AnchoredEffectRow, enclosure::Enclosure, prevalence_profile::hypergeometric_weights};

pub(crate) fn support(values: &[f64], order: usize) -> Enclosure {
    let mut sorted = values.to_vec();
    sorted.sort_unstable_by(f64::total_cmp);
    if sorted[0] == *sorted.last().unwrap() {
        return Enclosure::point(sorted[0]);
    }
    let n = sorted.len();
    let mut weight = Enclosure::ratio(order, n);
    let mut sum = Enclosure::ZERO;
    for (i, &value) in sorted.iter().take(n - order + 1).enumerate() {
        sum = sum + weight * Enclosure::point(value);
        if i < n - order {
            weight = weight * Enclosure::ratio(n - i - order, n - i - 1);
        }
    }
    sum.intersect(Enclosure {
        lo: sorted[0],
        hi: *sorted.last().unwrap(),
    })
}

pub(crate) fn survival(successes: usize, trials: usize, order: usize) -> Enclosure {
    if successes < order {
        return Enclosure::ZERO;
    }
    if successes == trials {
        return Enclosure::ONE;
    }
    (0..order)
        .fold(Enclosure::ONE, |p, i| {
            p * Enclosure::ratio(successes - i, trials - i)
        })
        .intersect(Enclosure { lo: 0.0, hi: 1.0 })
}

/// Same finite-array nested minimum as `anchored_nested_support`, with every
/// probability, width and accumulation enclosed instead of rounded to a point.
pub(crate) fn nested(
    rows: &[AnchoredEffectRow],
    empirical: usize,
    computational: usize,
    floor: f64,
) -> (Enclosure, Enclosure) {
    let fractions: Vec<_> = rows
        .iter()
        .map(|row| {
            if row.observed_effect <= floor {
                Enclosure::ZERO
            } else {
                survival(
                    row.computational_effects
                        .iter()
                        .filter(|&&x| x > floor)
                        .count(),
                    row.computational_effects.len(),
                    computational,
                )
            }
        })
        .collect();
    let literal = ProductTree::new(&fractions, empirical).value();
    let mut tree = ProductTree::new(&vec![Enclosure::ONE; rows.len()], empirical);
    let mut events = Vec::new();
    for (row, effects) in rows.iter().enumerate() {
        events.push((effects.observed_effect, row, true));
        events.extend(
            effects
                .computational_effects
                .iter()
                .map(|&x| (x, row, false)),
        );
    }
    events.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut active = vec![true; rows.len()];
    let mut remaining: Vec<_> = rows.iter().map(|r| r.computational_effects.len()).collect();
    let mut area = Enclosure::point(events[0].0);
    let mut index = 0;
    while index < events.len() {
        let level = events[index].0;
        let mut dirty = Vec::new();
        while index < events.len() && events[index].0 == level {
            let (_, row, anchor) = events[index];
            if anchor {
                active[row] = false;
            } else {
                remaining[row] -= 1;
            }
            dirty.push(row);
            index += 1;
        }
        dirty.sort_unstable();
        dirty.dedup();
        for row in dirty {
            tree.update(
                row,
                if active[row] {
                    survival(
                        remaining[row],
                        rows[row].computational_effects.len(),
                        computational,
                    )
                } else {
                    Enclosure::ZERO
                },
            );
        }
        if index < events.len() {
            area =
                area + (Enclosure::point(events[index].0) - Enclosure::point(level)) * tree.value();
        }
    }
    (
        area.intersect(Enclosure {
            lo: events[0].0,
            hi: events.last().unwrap().0,
        }),
        literal,
    )
}

// Normalized elementary-symmetric product tree. Merge weights depend only on
// row counts and are prepared once; updates touch only the path to the root.
struct ProductTree {
    leaves: usize,
    order: usize,
    counts: Vec<usize>,
    coefficients: Vec<Vec<Enclosure>>,
    weights: Vec<Vec<Vec<(usize, Enclosure)>>>,
}

impl ProductTree {
    fn new(values: &[Enclosure], order: usize) -> Self {
        let leaves = values.len().next_power_of_two();
        let mut tree = Self {
            leaves,
            order,
            counts: vec![0; leaves * 2],
            coefficients: vec![vec![Enclosure::ZERO; order + 1]; leaves * 2],
            weights: vec![Vec::new(); leaves * 2],
        };
        for i in 0..leaves {
            tree.coefficients[leaves + i][0] = Enclosure::ONE;
            if let Some(&value) = values.get(i) {
                tree.counts[leaves + i] = 1;
                tree.coefficients[leaves + i][1] = value;
            }
        }
        for node in (1..leaves).rev() {
            let a = tree.counts[node * 2];
            let b = tree.counts[node * 2 + 1];
            tree.counts[node] = a + b;
            tree.weights[node] = (0..=order.min(a + b))
                .map(|k| hypergeometric_weights(a, b, k))
                .collect();
            tree.rebuild(node);
        }
        tree
    }

    fn value(&self) -> Enclosure {
        self.coefficients[1][self.order]
    }

    fn update(&mut self, row: usize, value: Enclosure) {
        let mut node = self.leaves + row;
        self.coefficients[node][1] = value;
        node /= 2;
        while node != 0 {
            self.rebuild(node);
            node /= 2;
        }
    }

    fn rebuild(&mut self, node: usize) {
        self.coefficients[node][0] = Enclosure::ONE;
        for k in 1..=self.order.min(self.counts[node]) {
            let value = self.weights[node][k]
                .iter()
                .fold(Enclosure::ZERO, |sum, &(j, weight)| {
                    sum + weight
                        * self.coefficients[node * 2][j]
                        * self.coefficients[node * 2 + 1][k - j]
                });
            self.coefficients[node][k] = value.intersect(Enclosure { lo: 0.0, hi: 1.0 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subsets(n: usize, k: usize) -> Vec<Vec<usize>> {
        (0usize..1 << n)
            .filter(|mask| mask.count_ones() as usize == k)
            .map(|mask| (0..n).filter(|i| mask & (1 << i) != 0).collect())
            .collect()
    }

    #[test]
    fn rounded_flat_bounds_enclose_complete_subset_enumeration() {
        let values = [-0.25, 0.0, 0.125, 0.5, 0.875];
        for k in 1..=values.len() {
            let selected = subsets(values.len(), k);
            let expected = selected
                .iter()
                .map(|s| s.iter().map(|&i| values[i]).fold(f64::INFINITY, f64::min))
                .sum::<f64>()
                / selected.len() as f64;
            let bound = support(&values, k);
            assert!(bound.lo <= expected && expected <= bound.hi);
            for floor in [-0.25, 0.0, 0.125, 0.9] {
                let fraction = selected
                    .iter()
                    .filter(|s| s.iter().all(|&i| values[i] > floor))
                    .count() as f64
                    / selected.len() as f64;
                let bounds = survival(
                    values.iter().filter(|&&x| x > floor).count(),
                    values.len(),
                    k,
                );
                assert!(bounds.lo <= fraction && fraction <= bounds.hi);
            }
        }
    }

    #[test]
    fn rounded_nested_bounds_enclose_complete_anchored_challenges() {
        let rows = [
            AnchoredEffectRow {
                observed_effect: 0.5,
                computational_effects: vec![-0.25, 0.125, 0.75],
            },
            AnchoredEffectRow {
                observed_effect: 0.25,
                computational_effects: vec![0.0, 0.5, 0.875],
            },
            AnchoredEffectRow {
                observed_effect: 0.75,
                computational_effects: vec![0.125, 0.5, 0.75],
            },
        ];
        fn expand(
            rows: &[AnchoredEffectRow],
            selected: &[usize],
            k: usize,
            minimum: f64,
            out: &mut Vec<f64>,
        ) {
            let Some((&first, rest)) = selected.split_first() else {
                out.push(minimum);
                return;
            };
            for subset in subsets(rows[first].computational_effects.len(), k) {
                let m = subset
                    .iter()
                    .fold(minimum.min(rows[first].observed_effect), |m, &i| {
                        m.min(rows[first].computational_effects[i])
                    });
                expand(rows, rest, k, m, out);
            }
        }
        for empirical in 1..=rows.len() {
            for computational in 1..=3 {
                let mut minima = Vec::new();
                for selected in subsets(rows.len(), empirical) {
                    expand(&rows, &selected, computational, f64::INFINITY, &mut minima);
                }
                let expected = minima.iter().sum::<f64>() / minima.len() as f64;
                for floor in [0.0, 0.125, 0.5] {
                    let fraction =
                        minima.iter().filter(|&&x| x > floor).count() as f64 / minima.len() as f64;
                    let (magnitude, literal) = nested(&rows, empirical, computational, floor);
                    assert!(
                        magnitude.lo <= expected && expected <= magnitude.hi,
                        "{magnitude:?}, {expected}"
                    );
                    assert!(
                        literal.lo <= fraction && fraction <= literal.hi,
                        "{literal:?}, {fraction}"
                    );
                }
            }
        }
    }
}
