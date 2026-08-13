use serde::{Deserialize, Serialize};

use crate::survival::choose_ratio;
use crate::{Error, SupportOrder, SurvivalFloor};

/// One empirical row after every observed and computational evaluation has
/// already faced its complete condition challenge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnchoredEffectRow {
    pub observed_effect: f64,
    pub computational_effects: Vec<f64>,
}

/// Literal-survival diagnostics for one anchored empirical row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnchoredRowSurvival {
    pub observed_anchor_clears_floor: bool,
    pub computational_survivor_count: usize,
    pub computational_effect_count: usize,
    pub subset_fraction: f64,
}

/// Literal survival of complete nested challenges at a strict floor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NestedLiteralSurvival {
    pub empirical_row_count: usize,
    pub empirical_order: SupportOrder,
    pub computational_order: SupportOrder,
    pub survival_floor: SurvivalFloor,
    pub subset_fraction: f64,
    pub rows: Vec<AnchoredRowSurvival>,
}

/// Exact finite-array summaries of the manuscript's complete anchored
/// empirical--computational challenges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnchoredNestedSupport {
    pub supported_magnitude: f64,
    pub literal_survival: NestedLiteralSurvival,
}

/// Computes observed-anchored nested support without enumerating complete
/// selections.
///
/// Empirical subsets receive equal weight. Conditional on an empirical subset,
/// every distinct order-`K_C` computational subset in every selected row
/// receives equal weight. Both the magnitude and literal-survival results refer
/// to exactly those complete selections. The implementation uses a sorted
/// effect-level sweep and a normalized elementary-symmetric product tree; it
/// never enumerates the Cartesian product of complete nested selections.
pub fn anchored_nested_support(
    rows: &[AnchoredEffectRow],
    empirical_order: SupportOrder,
    computational_order: SupportOrder,
    floor: SurvivalFloor,
) -> Result<AnchoredNestedSupport, Error> {
    validate(rows, empirical_order, computational_order)?;
    let supported_magnitude = supported_magnitude(rows, empirical_order, computational_order);
    let literal_survival = literal_survival(rows, empirical_order, computational_order, floor);
    Ok(AnchoredNestedSupport {
        supported_magnitude,
        literal_survival,
    })
}

fn validate(
    rows: &[AnchoredEffectRow],
    empirical_order: SupportOrder,
    computational_order: SupportOrder,
) -> Result<(), Error> {
    if rows.is_empty() {
        return Err(Error::EmptyEmpiricalEvaluations);
    }
    if empirical_order.get() > rows.len() {
        return Err(Error::SupportOrderExceedsList {
            order: empirical_order.get(),
            list_length: rows.len(),
        });
    }
    for (row_index, row) in rows.iter().enumerate() {
        if !row.observed_effect.is_finite() {
            return Err(Error::InvalidObservedEffect {
                row: row_index,
                value: row.observed_effect,
            });
        }
        if computational_order.get() > row.computational_effects.len() {
            return Err(Error::ComputationalOrderExceedsRow {
                row: row_index,
                order: computational_order.get(),
                list_length: row.computational_effects.len(),
            });
        }
        for (index, &value) in row.computational_effects.iter().enumerate() {
            if !value.is_finite() {
                return Err(Error::InvalidComputationalEffect {
                    row: row_index,
                    index,
                    value,
                });
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum EventKind {
    ObservedAnchor,
    ComputationalEffect,
}

#[derive(Debug, Clone, Copy)]
struct Event {
    value: f64,
    row: usize,
    kind: EventKind,
}

fn supported_magnitude(
    rows: &[AnchoredEffectRow],
    empirical_order: SupportOrder,
    computational_order: SupportOrder,
) -> f64 {
    let mut events = Vec::with_capacity(
        rows.len()
            + rows
                .iter()
                .map(|row| row.computational_effects.len())
                .sum::<usize>(),
    );
    for (row_index, row) in rows.iter().enumerate() {
        events.push(Event {
            value: row.observed_effect,
            row: row_index,
            kind: EventKind::ObservedAnchor,
        });
        events.extend(
            row.computational_effects
                .iter()
                .copied()
                .map(|value| Event {
                    value,
                    row: row_index,
                    kind: EventKind::ComputationalEffect,
                }),
        );
    }
    events.sort_unstable_by(|left, right| {
        left.value
            .total_cmp(&right.value)
            .then_with(|| left.row.cmp(&right.row))
    });

    let lower_endpoint = events[0].value;
    let mut computational_survivors: Vec<_> = rows
        .iter()
        .map(|row| row.computational_effects.len())
        .collect();
    let mut anchor_active = vec![true; rows.len()];
    let mut row_fractions = vec![1.0; rows.len()];
    let mut tree = NormalizedElementaryTree::new(&row_fractions, empirical_order.get());
    let mut dirty = vec![false; rows.len()];
    let mut dirty_rows = Vec::new();
    let mut area = CompensatedSum::default();
    let mut start = 0;

    while start < events.len() {
        let level = events[start].value;
        let mut end = start + 1;
        while end < events.len() && events[end].value == level {
            end += 1;
        }

        for event in &events[start..end] {
            match event.kind {
                EventKind::ObservedAnchor => anchor_active[event.row] = false,
                EventKind::ComputationalEffect => {
                    computational_survivors[event.row] -= 1;
                }
            }
            if !dirty[event.row] {
                dirty[event.row] = true;
                dirty_rows.push(event.row);
            }
        }
        for &row_index in &dirty_rows {
            let row = &rows[row_index];
            let fraction = if anchor_active[row_index] {
                choose_ratio(
                    computational_survivors[row_index],
                    row.computational_effects.len(),
                    computational_order.get(),
                )
            } else {
                0.0
            };
            row_fractions[row_index] = fraction;
            tree.update(row_index, fraction);
            dirty[row_index] = false;
        }
        dirty_rows.clear();

        if end < events.len() {
            let next_level = events[end].value;
            area.add((next_level - level) * tree.value(empirical_order.get()));
        }
        start = end;
    }

    lower_endpoint + area.total()
}

fn literal_survival(
    rows: &[AnchoredEffectRow],
    empirical_order: SupportOrder,
    computational_order: SupportOrder,
    floor: SurvivalFloor,
) -> NestedLiteralSurvival {
    let row_summaries: Vec<_> = rows
        .iter()
        .map(|row| {
            let survivors = row
                .computational_effects
                .iter()
                .filter(|&&effect| effect > floor.get())
                .count();
            let anchor_clears = row.observed_effect > floor.get();
            let subset_fraction = if anchor_clears {
                choose_ratio(
                    survivors,
                    row.computational_effects.len(),
                    computational_order.get(),
                )
            } else {
                0.0
            };
            AnchoredRowSurvival {
                observed_anchor_clears_floor: anchor_clears,
                computational_survivor_count: survivors,
                computational_effect_count: row.computational_effects.len(),
                subset_fraction,
            }
        })
        .collect();
    let row_fractions: Vec<_> = row_summaries
        .iter()
        .map(|row| row.subset_fraction)
        .collect();
    let tree = NormalizedElementaryTree::new(&row_fractions, empirical_order.get());
    NestedLiteralSurvival {
        empirical_row_count: rows.len(),
        empirical_order,
        computational_order,
        survival_floor: floor,
        subset_fraction: tree.value(empirical_order.get()),
        rows: row_summaries,
    }
}

/// Product tree for normalized elementary symmetric polynomials. A node with
/// `n` leaves stores `e_k / choose(n, k)`, keeping every coefficient in [0, 1].
struct NormalizedElementaryTree {
    leaf_count: usize,
    order: usize,
    counts: Vec<usize>,
    coefficients: Vec<f64>,
    scratch: Vec<f64>,
}

impl NormalizedElementaryTree {
    fn new(values: &[f64], order: usize) -> Self {
        let leaf_count = values.len().next_power_of_two();
        let node_count = 2 * leaf_count;
        let width = order + 1;
        let mut tree = Self {
            leaf_count,
            order,
            counts: vec![0; node_count],
            coefficients: vec![0.0; node_count * width],
            scratch: vec![0.0; width],
        };
        for (index, &value) in values.iter().enumerate() {
            let leaf = leaf_count + index;
            tree.counts[leaf] = 1;
            tree.set_coefficient(leaf, 0, 1.0);
            if order >= 1 {
                tree.set_coefficient(leaf, 1, value);
            }
        }
        for index in values.len()..leaf_count {
            let leaf = leaf_count + index;
            tree.set_coefficient(leaf, 0, 1.0);
        }
        for node in (1..leaf_count).rev() {
            tree.rebuild(node);
        }
        tree
    }

    fn update(&mut self, row: usize, value: f64) {
        let mut node = self.leaf_count + row;
        self.set_coefficient(node, 1, value);
        node /= 2;
        while node > 0 {
            self.rebuild(node);
            node /= 2;
        }
    }

    fn value(&self, degree: usize) -> f64 {
        self.coefficient(1, degree)
    }

    fn coefficient(&self, node: usize, degree: usize) -> f64 {
        self.coefficients[node * (self.order + 1) + degree]
    }

    fn set_coefficient(&mut self, node: usize, degree: usize, value: f64) {
        let index = node * (self.order + 1) + degree;
        self.coefficients[index] = value;
    }

    fn rebuild(&mut self, node: usize) {
        let left = 2 * node;
        let right = left + 1;
        let left_count = self.counts[left];
        let right_count = self.counts[right];
        self.counts[node] = left_count + right_count;
        self.scratch.fill(0.0);
        self.scratch[0] = 1.0;

        if left_count == 0 || right_count == 0 {
            let source = if left_count == 0 { right } else { left };
            let maximum = self.order.min(self.counts[source]);
            for degree in 1..=maximum {
                self.scratch[degree] = self.coefficient(source, degree);
            }
        } else {
            let maximum = self.order.min(left_count + right_count);
            for degree in 1..=maximum {
                self.scratch[degree] =
                    self.merged_coefficient(left, right, left_count, right_count, degree);
            }
        }
        let start = node * (self.order + 1);
        self.coefficients[start..start + self.order + 1].copy_from_slice(&self.scratch);
    }

    fn merged_coefficient(
        &self,
        left: usize,
        right: usize,
        left_count: usize,
        right_count: usize,
        degree: usize,
    ) -> f64 {
        let lower = degree.saturating_sub(right_count);
        let upper = degree.min(left_count);
        let population = left_count + right_count;
        let mode =
            (((degree + 1) as u128 * (left_count + 1) as u128) / (population + 2) as u128) as usize;
        let mode = mode.clamp(lower, upper);
        let mut weight = (log_choose(left_count, mode) + log_choose(right_count, degree - mode)
            - log_choose(population, degree))
        .exp();
        let mut total =
            weight * self.coefficient(left, mode) * self.coefficient(right, degree - mode);
        let mut weight_total = weight;

        for selected_left in mode + 1..=upper {
            let previous = selected_left - 1;
            weight *= (left_count - previous) as f64 / selected_left as f64
                * (degree - previous) as f64
                / (right_count + previous + 1 - degree) as f64;
            total += weight
                * self.coefficient(left, selected_left)
                * self.coefficient(right, degree - selected_left);
            weight_total += weight;
        }

        weight = (log_choose(left_count, mode) + log_choose(right_count, degree - mode)
            - log_choose(population, degree))
        .exp();
        for selected_left in (lower..mode).rev() {
            let next = selected_left + 1;
            weight *= next as f64 / (left_count - selected_left) as f64
                * (right_count + next - degree) as f64
                / (degree - selected_left) as f64;
            total += weight
                * self.coefficient(left, selected_left)
                * self.coefficient(right, degree - selected_left);
            weight_total += weight;
        }

        total / weight_total
    }
}

fn log_choose(n: usize, k: usize) -> f64 {
    let k = k.min(n - k);
    (1..=k)
        .map(|index| ((n - k + index) as f64).ln() - (index as f64).ln())
        .sum()
}

#[derive(Default)]
struct CompensatedSum {
    sum: f64,
    correction: f64,
}

impl CompensatedSum {
    fn add(&mut self, value: f64) {
        let adjusted = value - self.correction;
        let next = self.sum + adjusted;
        self.correction = (next - self.sum) - adjusted;
        self.sum = next;
    }

    fn total(&self) -> f64 {
        self.sum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orders(empirical: usize, computational: usize) -> (SupportOrder, SupportOrder) {
        (
            SupportOrder::new(empirical).unwrap(),
            SupportOrder::new(computational).unwrap(),
        )
    }

    #[test]
    fn manuscript_two_by_three_example_is_exact() {
        let rows = [
            AnchoredEffectRow {
                observed_effect: 0.30,
                computational_effects: vec![0.50, 0.10, 0.40],
            },
            AnchoredEffectRow {
                observed_effect: 0.20,
                computational_effects: vec![0.25, 0.35, -0.05],
            },
        ];
        let (empirical, computational) = orders(2, 1);
        let result = anchored_nested_support(
            &rows,
            empirical,
            computational,
            SurvivalFloor::new(0.0).unwrap(),
        )
        .unwrap();
        assert!((result.supported_magnitude - 0.85 / 9.0).abs() < 1e-14);
        assert!((result.literal_survival.subset_fraction - 2.0 / 3.0).abs() < 1e-14);
    }

    #[test]
    fn sweep_matches_complete_nested_enumeration_with_unequal_rows() {
        let rows = [
            AnchoredEffectRow {
                observed_effect: 0.4,
                computational_effects: vec![-0.2, 0.1, 0.5],
            },
            AnchoredEffectRow {
                observed_effect: 0.3,
                computational_effects: vec![0.0, 0.2, 0.6, 0.8],
            },
            AnchoredEffectRow {
                observed_effect: -0.1,
                computational_effects: vec![-0.3, 0.7, 0.9],
            },
        ];
        for empirical in 1..=rows.len() {
            for computational in 1..=3 {
                let (empirical_order, computational_order) = orders(empirical, computational);
                let result = anchored_nested_support(
                    &rows,
                    empirical_order,
                    computational_order,
                    SurvivalFloor::new(0.0).unwrap(),
                )
                .unwrap();
                let (exact_magnitude, exact_survival) =
                    enumerate_exact(&rows, empirical, computational, 0.0);
                assert!(
                    (result.supported_magnitude - exact_magnitude).abs() < 1e-13,
                    "K_E={empirical}, K_C={computational}: sweep={}, exact={exact_magnitude}",
                    result.supported_magnitude
                );
                assert!((result.literal_survival.subset_fraction - exact_survival).abs() < 1e-13);
            }
        }
    }

    #[test]
    fn equal_floor_values_do_not_survive() {
        let rows = [AnchoredEffectRow {
            observed_effect: 0.2,
            computational_effects: vec![0.0, 0.1, 0.2],
        }];
        let (empirical, computational) = orders(1, 1);
        let result = anchored_nested_support(
            &rows,
            empirical,
            computational,
            SurvivalFloor::new(0.1).unwrap(),
        )
        .unwrap();
        assert_eq!(
            result.literal_survival.rows[0].computational_survivor_count,
            1
        );
        assert!((result.literal_survival.subset_fraction - 1.0 / 3.0).abs() < 1e-14);
    }

    #[test]
    fn one_row_boundary_matches_the_flat_anchored_list() {
        let anchor = 0.25;
        let computational = vec![-0.1, 0.2, 0.4, 0.8];
        let rows = [AnchoredEffectRow {
            observed_effect: anchor,
            computational_effects: computational.clone(),
        }];
        let empirical = SupportOrder::new(1).unwrap();
        for computational_order in 1..=computational.len() {
            let order = SupportOrder::new(computational_order).unwrap();
            let result =
                anchored_nested_support(&rows, empirical, order, SurvivalFloor::new(0.0).unwrap())
                    .unwrap();
            let clipped: Vec<_> = computational
                .iter()
                .map(|&effect| anchor.min(effect))
                .collect();
            let flat = crate::support(&clipped, order).unwrap();
            let flat_survival =
                crate::literal_survival(&clipped, order, SurvivalFloor::new(0.0).unwrap()).unwrap();
            assert!((result.supported_magnitude - flat).abs() < 1e-14);
            assert!(
                (result.literal_survival.subset_fraction - flat_survival.subset_fraction).abs()
                    < 1e-14
            );
        }
    }

    #[test]
    fn increasing_either_order_cannot_improve_the_challenge() {
        let rows = [
            AnchoredEffectRow {
                observed_effect: 0.5,
                computational_effects: vec![-0.2, 0.3, 0.7, 0.9],
            },
            AnchoredEffectRow {
                observed_effect: 0.4,
                computational_effects: vec![0.0, 0.2, 0.6, 0.8],
            },
            AnchoredEffectRow {
                observed_effect: 0.1,
                computational_effects: vec![-0.3, 0.1, 0.5, 1.0],
            },
        ];
        let floor = SurvivalFloor::new(0.0).unwrap();
        let by_order: Vec<Vec<_>> = (1..=3)
            .map(|empirical| {
                (1..=4)
                    .map(|computational| {
                        anchored_nested_support(
                            &rows,
                            SupportOrder::new(empirical).unwrap(),
                            SupportOrder::new(computational).unwrap(),
                            floor,
                        )
                        .unwrap()
                    })
                    .collect()
            })
            .collect();
        for (empirical_index, computational_orders) in by_order.iter().enumerate() {
            for (computational_index, current) in computational_orders.iter().enumerate() {
                if let Some(stronger_empirical) = by_order.get(empirical_index + 1) {
                    let stronger = &stronger_empirical[computational_index];
                    assert!(stronger.supported_magnitude <= current.supported_magnitude + 1e-14);
                    assert!(
                        stronger.literal_survival.subset_fraction
                            <= current.literal_survival.subset_fraction + 1e-14
                    );
                }
                if let Some(stronger) = computational_orders.get(computational_index + 1) {
                    assert!(stronger.supported_magnitude <= current.supported_magnitude + 1e-14);
                    assert!(
                        stronger.literal_survival.subset_fraction
                            <= current.literal_survival.subset_fraction + 1e-14
                    );
                }
            }
        }
    }

    fn enumerate_exact(
        rows: &[AnchoredEffectRow],
        empirical_order: usize,
        computational_order: usize,
        floor: f64,
    ) -> (f64, f64) {
        let mut empirical_subsets = Vec::new();
        combinations(
            rows.len(),
            empirical_order,
            0,
            &mut Vec::new(),
            &mut empirical_subsets,
        );
        let mut magnitude = 0.0;
        let mut survival = 0.0;
        let empirical_subset_count = empirical_subsets.len();
        for empirical in empirical_subsets {
            let mut minima = Vec::new();
            enumerate_rows(
                rows,
                &empirical,
                computational_order,
                0,
                f64::INFINITY,
                &mut minima,
            );
            magnitude += minima.iter().sum::<f64>() / minima.len() as f64;
            survival +=
                minima.iter().filter(|&&value| value > floor).count() as f64 / minima.len() as f64;
        }
        (
            magnitude / empirical_subset_count as f64,
            survival / empirical_subset_count as f64,
        )
    }

    fn enumerate_rows(
        rows: &[AnchoredEffectRow],
        empirical: &[usize],
        computational_order: usize,
        position: usize,
        minimum: f64,
        out: &mut Vec<f64>,
    ) {
        if position == empirical.len() {
            out.push(minimum);
            return;
        }
        let row = &rows[empirical[position]];
        let mut subsets = Vec::new();
        combinations(
            row.computational_effects.len(),
            computational_order,
            0,
            &mut Vec::new(),
            &mut subsets,
        );
        for subset in subsets {
            let row_minimum = subset.iter().fold(row.observed_effect, |value, &index| {
                value.min(row.computational_effects[index])
            });
            enumerate_rows(
                rows,
                empirical,
                computational_order,
                position + 1,
                minimum.min(row_minimum),
                out,
            );
        }
    }

    fn combinations(
        length: usize,
        order: usize,
        start: usize,
        current: &mut Vec<usize>,
        out: &mut Vec<Vec<usize>>,
    ) {
        if current.len() == order {
            out.push(current.clone());
            return;
        }
        let remaining = order - current.len();
        for index in start..=length - remaining {
            current.push(index);
            combinations(length, order, index + 1, current, out);
            current.pop();
        }
    }
}
