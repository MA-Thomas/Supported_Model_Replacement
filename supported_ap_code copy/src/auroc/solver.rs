//! Certified adversarial minimisation of the paired AUROC difference.
//!
//! # Reformulation
//!
//! With `E` the excluded positive mass and `H` the excluded negative mass, the
//! objective of Equation (auroc-z-gamma) is
//!
//! ```text
//!   Z = u+ u- [ T  -  sum_E e_i R_i  -  sum_H f_k C_k  +  sum_{E x H} e_i f_k G_ik ]
//! ```
//!
//! where `T` is the grand sum and `R`, `C` the row and column sums of the
//! multiplicity-weighted gain matrix. The first three terms are separable and
//! solved exactly by a greedy fill. Only the final cross term couples the two
//! classes, and it is bounded by `|E| |H|` because `|G_ik| <= 1`.
//!
//! # Two complementary lower bounds
//!
//! *Exclusion side.* Dropping the cross term costs exactly
//! `u+ u- |E| |H| = (Gamma - 1)^2` — free of the data and free of `n_+`, `n_-`.
//! Tight near `Gamma = 1`, useless for large `Gamma`.
//!
//! *Inclusion side.* Letting every retained case of one class choose its own
//! worst opposing cases relaxes the requirement of a common opposing weighting.
//! Weak near `Gamma = 1`, strong for large `Gamma`, and exact once the cap
//! saturates. Because the gain matrix takes only five distinct values, each
//! case's relaxation is read off a five-bin mass histogram in constant time.
//!
//! The maximum of the two is used. This matters: it makes solver behaviour
//! roughly uniform in `Gamma` rather than dependent on the breakdown factor
//! happening to sit near one.
//!
//! # Certification is one-sided
//!
//! Appendix requirement: demonstrating *failure* needs only a feasible
//! weighting, whereas certifying that a requirement continues to *hold* needs a
//! valid global lower bound. Alternating optimisation supplies the former and
//! never the latter, so branch and bound runs only when the cheap bounds do not
//! already resolve the decision. When its budget is exhausted the reported
//! interval widens; it never silently reports the incumbent as optimal.

use std::time::{Duration, Instant};

use super::capped::{CappedClass, Exclusion, fill_exclusion, maximum_over_mass, minimum_over_mass};
use super::gain::{GAIN_LEVELS, GainMatrix, Marginals, Multiplicities, level_value};

/// Effect interval with the feasible weighting that attained the upper end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SolveOutcome {
    pub(crate) lower: f64,
    pub(crate) upper: f64,
    pub(crate) exact: bool,
    pub(crate) nodes: usize,
    pub(crate) time_limit_reached: bool,
    pub(crate) decision_resolved: bool,
}

impl SolveOutcome {
    pub(crate) fn exact_value(value: f64) -> Self {
        Self {
            lower: value,
            upper: value,
            exact: true,
            nodes: 0,
            time_limit_reached: false,
            decision_resolved: false,
        }
    }
}

/// Controls for one adversarial solve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SolveControls {
    pub(crate) absolute_gap: f64,
    pub(crate) relative_gap: f64,
    /// Maximum branch-and-bound nodes. Exhaustion widens the interval.
    pub(crate) node_budget: usize,
    /// Alternating restarts. The first is always the uniform start.
    pub(crate) restarts: usize,
    /// Threshold the caller must resolve, when decision-directed.
    ///
    /// Search stops as soon as the interval lies strictly on one side of it.
    pub(crate) resolve_against: Option<f64>,
    /// Optional wall-clock limit for this complete solve.
    pub(crate) time_limit_seconds: Option<f64>,
}

/// One adversarial problem: matrix, multiplicities, marginals, and caps.
pub(crate) struct Instance<'a> {
    pub(crate) matrix: &'a GainMatrix,
    pub(crate) multiplicities: &'a Multiplicities,
    pub(crate) marginals: &'a Marginals,
    pub(crate) positive: CappedClass,
    pub(crate) negative: CappedClass,
}

impl<'a> Instance<'a> {
    pub(crate) fn new(
        matrix: &'a GainMatrix,
        multiplicities: &'a Multiplicities,
        marginals: &'a Marginals,
        gamma: f64,
    ) -> Self {
        Self {
            matrix,
            multiplicities,
            marginals,
            positive: CappedClass::new(multiplicities.positive_total(), gamma),
            negative: CappedClass::new(multiplicities.negative_total(), gamma),
        }
    }

    #[inline]
    fn scale(&self) -> f64 {
        self.positive.cap * self.negative.cap
    }

    #[inline]
    fn total(&self) -> f64 {
        self.marginals.total() as f64 / 2.0
    }

    #[inline]
    fn row_sum(&self, positive: u32) -> f64 {
        self.marginals.row_sums()[positive as usize] as f64 / 2.0
    }

    #[inline]
    fn col_sum(&self, negative: u32) -> f64 {
        self.marginals.col_sums()[negative as usize] as f64 / 2.0
    }
}

/// Reusable buffers, so the `Gamma` search does not reallocate per factor.
#[derive(Debug, Default)]
pub(crate) struct SolverScratch {
    positive_coefficients: Vec<f64>,
    negative_coefficients: Vec<f64>,
    positive_relaxation: Vec<f64>,
    negative_relaxation: Vec<f64>,
    corrected_columns: Vec<f64>,
    order: Vec<u32>,
    positive_exclusion: Exclusion,
    negative_exclusion: Exclusion,
    best_positive: Exclusion,
    best_negative: Exclusion,
}

impl SolverScratch {
    fn ensure(&mut self, positives: usize, negatives: usize) {
        self.positive_coefficients.resize(positives, 0.0);
        self.positive_relaxation.resize(positives, 0.0);
        self.negative_coefficients.resize(negatives, 0.0);
        self.negative_relaxation.resize(negatives, 0.0);
        self.corrected_columns.resize(negatives, 0.0);
    }
}

/// Solves one adversarial problem, returning a certified interval.
pub(crate) fn solve(
    instance: &Instance<'_>,
    controls: SolveControls,
    scratch: &mut SolverScratch,
) -> SolveOutcome {
    let started = Instant::now();
    let positives = instance.matrix.positive_count();
    let negatives = instance.matrix.negative_count();
    scratch.ensure(positives, negatives);

    // Gamma = 1 forces uniform weights; the value is the ordinary difference.
    if instance.positive.exclude_mass <= 0.0 && instance.negative.exclude_mass <= 0.0 {
        return SolveOutcome::exact_value(
            instance
                .marginals
                .uniform_difference(instance.multiplicities),
        );
    }
    // Once every supported case can absorb unit weight, the adversary reaches
    // the least favourable pair. Using the peak multiplicity here is unsound:
    // the minimum-gain pair may occur at a less frequently repeated case.
    if instance
        .positive
        .saturates(instance.multiplicities.positive_minimum())
        && instance
            .negative
            .saturates(instance.multiplicities.negative_minimum())
    {
        return SolveOutcome::exact_value(instance.marginals.minimum_entry(instance.matrix));
    }

    let upper = alternating_incumbent(instance, controls.restarts, scratch);
    let lower = cheap_lower_bound(instance, scratch);
    let mut outcome = SolveOutcome {
        lower: lower.min(upper),
        upper,
        exact: false,
        nodes: 0,
        time_limit_reached: false,
        decision_resolved: false,
    };
    if resolved(&outcome, &controls) {
        outcome.decision_resolved = controls
            .resolve_against
            .is_some_and(|threshold| outcome.lower > threshold || outcome.upper <= threshold);
        return outcome;
    }
    branch_and_bound(instance, controls, scratch, &mut outcome, started);
    outcome
}

/// True when the interval already determines the caller's decision.
fn resolved(outcome: &SolveOutcome, controls: &SolveControls) -> bool {
    let gap = outcome.upper - outcome.lower;
    if gap <= controls.absolute_gap || gap / outcome.upper.abs().max(1.0) <= controls.relative_gap {
        return true;
    }
    // Decision-directed stopping: only the side of the threshold matters.
    match controls.resolve_against {
        Some(threshold) => outcome.lower > threshold || outcome.upper <= threshold,
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Objective and best responses
// ---------------------------------------------------------------------------

/// Exact objective for a pair of exclusion vertices.
fn objective(instance: &Instance<'_>, positive: &Exclusion, negative: &Exclusion) -> f64 {
    let positive_counts = instance.multiplicities.positive();
    let negative_counts = instance.multiplicities.negative();
    let mut value = instance.total();
    for (index, mass) in positive.iter(positive_counts) {
        value -= mass * instance.row_sum(index);
    }
    for (index, mass) in negative.iter(negative_counts) {
        value -= mass * instance.col_sum(index);
    }
    for (positive_index, positive_mass) in positive.iter(positive_counts) {
        let row = instance.matrix.row(positive_index as usize);
        for (negative_index, negative_mass) in negative.iter(negative_counts) {
            let gain = f64::from(row[negative_index as usize]) / 2.0;
            value += positive_mass * negative_mass * gain;
        }
    }
    value * instance.scale()
}

/// Negative-class coefficients given a positive exclusion: `C_k - sum_E e_i G_ik`.
///
/// Reads only the excluded rows, which is why the near-`Gamma = 1` regime is
/// cheap: the excluded set is small exactly when the policy still passes.
fn negative_coefficients(instance: &Instance<'_>, positive: &Exclusion, out: &mut [f64]) {
    let support = instance.marginals.negative_support();
    for &index in support {
        out[index as usize] = instance.col_sum(index);
    }
    for (positive_index, mass) in positive.iter(instance.multiplicities.positive()) {
        let row = instance.matrix.row(positive_index as usize);
        for &index in support {
            out[index as usize] -= mass * f64::from(row[index as usize]) / 2.0;
        }
    }
}

/// Positive-class coefficients given a negative exclusion: `R_i - sum_H f_k G_ik`.
fn positive_coefficients(instance: &Instance<'_>, negative: &Exclusion, out: &mut [f64]) {
    let support = instance.marginals.positive_support();
    let negative_counts = instance.multiplicities.negative();
    for &index in support {
        let row = instance.matrix.row(index as usize);
        let mut value = instance.row_sum(index);
        for (negative_index, mass) in negative.iter(negative_counts) {
            value -= mass * f64::from(row[negative_index as usize]) / 2.0;
        }
        out[index as usize] = value;
    }
}

/// Alternating best response. Feasible throughout, so always an upper bound.
fn alternating_incumbent(
    instance: &Instance<'_>,
    restarts: usize,
    scratch: &mut SolverScratch,
) -> f64 {
    let positive_support = instance.marginals.positive_support();
    let negative_support = instance.marginals.negative_support();
    let positive_counts = instance.multiplicities.positive();
    let negative_counts = instance.multiplicities.negative();

    let mut best = f64::INFINITY;
    for restart in 0..restarts.max(1) {
        let mut positive_exclusion = std::mem::take(&mut scratch.positive_exclusion);
        let mut negative_exclusion = std::mem::take(&mut scratch.negative_exclusion);
        positive_exclusion.clear();

        if restart > 0 {
            // Deterministic alternative start: exclude by raw row marginal,
            // which differs from the uniform start whenever the coupling
            // matters. No RNG, so results stay reproducible.
            for &index in positive_support {
                scratch.positive_coefficients[index as usize] =
                    -instance.row_sum(index) * (restart as f64);
            }
            fill_exclusion(
                positive_support,
                &scratch.positive_coefficients,
                positive_counts,
                instance.positive,
                &mut scratch.order,
                &mut positive_exclusion,
            );
        }

        let mut previous = f64::INFINITY;
        for _ in 0..64 {
            negative_coefficients(
                instance,
                &positive_exclusion,
                &mut scratch.negative_coefficients,
            );
            fill_exclusion(
                negative_support,
                &scratch.negative_coefficients,
                negative_counts,
                instance.negative,
                &mut scratch.order,
                &mut negative_exclusion,
            );
            positive_coefficients(
                instance,
                &negative_exclusion,
                &mut scratch.positive_coefficients,
            );
            fill_exclusion(
                positive_support,
                &scratch.positive_coefficients,
                positive_counts,
                instance.positive,
                &mut scratch.order,
                &mut positive_exclusion,
            );
            let value = objective(instance, &positive_exclusion, &negative_exclusion);
            if value >= previous - 1e-15 {
                previous = previous.min(value);
                break;
            }
            previous = value;
        }
        if previous < best {
            best = previous;
            scratch.best_positive = positive_exclusion.clone();
            scratch.best_negative = negative_exclusion.clone();
        }
        scratch.positive_exclusion = positive_exclusion;
        scratch.negative_exclusion = negative_exclusion;
    }
    best
}

// ---------------------------------------------------------------------------
// Lower bounds
// ---------------------------------------------------------------------------

/// Independent relaxation value for one case, from its gain-level histogram.
///
/// Fills `mass` units of opposing multiplicity starting at the most adverse
/// gain level. Exact for the relaxed problem, and O(GAIN_LEVELS).
#[inline]
fn relaxation_from_levels(levels: &[u64; GAIN_LEVELS], mass: f64) -> f64 {
    let mut remaining = mass;
    let mut total = 0.0;
    for (level, &level_mass) in levels.iter().enumerate() {
        if remaining <= 0.0 {
            break;
        }
        let available = (level_mass as f64).min(remaining);
        total += available * (level_value(level) as f64 / 2.0);
        remaining -= available;
    }
    total
}

/// Maximum of the exclusion-side and both inclusion-side relaxations.
fn cheap_lower_bound(instance: &Instance<'_>, scratch: &mut SolverScratch) -> f64 {
    let positive_support = instance.marginals.positive_support();
    let negative_support = instance.marginals.negative_support();
    let positive_counts = instance.multiplicities.positive();
    let negative_counts = instance.multiplicities.negative();
    let scale = instance.scale();

    // Exclusion side. Slack is exactly (Gamma - 1)^2.
    for &index in positive_support {
        scratch.positive_coefficients[index as usize] = instance.row_sum(index);
    }
    for &index in negative_support {
        scratch.negative_coefficients[index as usize] = instance.col_sum(index);
    }
    let rows = maximum_over_mass(
        positive_support,
        &scratch.positive_coefficients,
        positive_counts,
        instance.positive.exclude_mass,
        &mut scratch.order,
    );
    let columns = maximum_over_mass(
        negative_support,
        &scratch.negative_coefficients,
        negative_counts,
        instance.negative.exclude_mass,
        &mut scratch.order,
    );
    let exclusion = scale
        * (instance.total()
            - rows
            - columns
            - instance.positive.exclude_mass * instance.negative.exclude_mass);

    // Inclusion side, positives choosing their own worst negatives.
    let row_levels = instance.marginals.row_levels();
    for &index in positive_support {
        scratch.positive_relaxation[index as usize] =
            relaxation_from_levels(&row_levels[index as usize], instance.negative.include_mass);
    }
    let by_rows = scale
        * minimum_over_mass(
            positive_support,
            &scratch.positive_relaxation,
            positive_counts,
            instance.positive.include_mass,
            &mut scratch.order,
        );

    // Inclusion side, negatives choosing their own worst positives.
    let col_levels = instance.marginals.col_levels();
    for &index in negative_support {
        scratch.negative_relaxation[index as usize] =
            relaxation_from_levels(&col_levels[index as usize], instance.positive.include_mass);
    }
    let by_columns = scale
        * minimum_over_mass(
            negative_support,
            &scratch.negative_relaxation,
            negative_counts,
            instance.negative.include_mass,
            &mut scratch.order,
        );

    exclusion.max(by_rows).max(by_columns)
}

// ---------------------------------------------------------------------------
// Branch and bound
// ---------------------------------------------------------------------------

/// Node bound after fixing exact exclusion masses for a set of positives.
///
/// With `A` the fixed set and `D_k = sum_{i in A} m_i G_ik`,
///
/// ```text
///   Z / (u+ u-) >=  T - sum_A m_i R_i
///                     - max_free(R, free_mass)
///                     - max(C - D, |H|)
///                     - free_mass * |H|
/// ```
///
/// The residual slack `free_mass * |H|` shrinks in proportion to the exclusion
/// mass still unassigned, so the bound becomes exact once `free_mass` reaches
/// zero. That is what makes the search terminate with a genuine certificate
/// rather than a heuristic value.
fn node_bound(
    instance: &Instance<'_>,
    fixed: &[(u32, f64)],
    fixed_row_total: f64,
    free_mass: f64,
    candidates: &[u32],
    scratch: &mut SolverScratch,
) -> f64 {
    let negative_support = instance.marginals.negative_support();
    let negative_counts = instance.multiplicities.negative();
    let positive_counts = instance.multiplicities.positive();

    for &index in negative_support {
        scratch.corrected_columns[index as usize] = instance.col_sum(index);
    }
    for &(positive, mass) in fixed {
        let row = instance.matrix.row(positive as usize);
        for &index in negative_support {
            scratch.corrected_columns[index as usize] -=
                mass * f64::from(row[index as usize]) / 2.0;
        }
    }
    let columns = maximum_over_mass(
        negative_support,
        &scratch.corrected_columns,
        negative_counts,
        instance.negative.exclude_mass,
        &mut scratch.order,
    );

    for &index in candidates {
        scratch.positive_coefficients[index as usize] = instance.row_sum(index);
    }
    let rows = maximum_over_mass(
        candidates,
        &scratch.positive_coefficients,
        positive_counts,
        free_mass,
        &mut scratch.order,
    );

    instance.scale()
        * (instance.total()
            - fixed_row_total
            - rows
            - columns
            - free_mass * instance.negative.exclude_mass)
}

/// Depth-first search over positive exclusion decisions.
///
/// Candidates are ordered by descending row marginal, since those are the
/// positives an adversary most wants to remove. The incumbent from alternation
/// is usually already optimal, so the search is verifying rather than
/// discovering, which prunes far more aggressively.
fn branch_and_bound(
    instance: &Instance<'_>,
    controls: SolveControls,
    scratch: &mut SolverScratch,
    outcome: &mut SolveOutcome,
    started: Instant,
) {
    let deadline = controls
        .time_limit_seconds
        .map(|seconds| started + Duration::from_secs_f64(seconds));
    let positive_counts = instance.multiplicities.positive();
    let mut candidates: Vec<u32> = instance.marginals.positive_support().to_vec();
    candidates.sort_unstable_by(|&left, &right| {
        instance
            .row_sum(right)
            .total_cmp(&instance.row_sum(left))
            .then_with(|| left.cmp(&right))
    });
    let mut fixed: Vec<(u32, f64)> = Vec::new();
    let mut frontier = f64::INFINITY;
    let mut nodes = 0usize;
    let mut budget_exhausted = false;

    // Explicit stack: (depth into candidate list, fixed length, free mass, row total).
    struct Frame {
        cursor: usize,
        fixed_len: usize,
        free_mass: f64,
        row_total: f64,
    }
    let mut stack = vec![Frame {
        cursor: 0,
        fixed_len: 0,
        free_mass: instance.positive.exclude_mass,
        row_total: 0.0,
    }];

    while let Some(frame) = stack.pop() {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            outcome.nodes = nodes;
            outcome.exact = false;
            outcome.time_limit_reached = true;
            return;
        }
        fixed.truncate(frame.fixed_len);
        if nodes >= controls.node_budget {
            budget_exhausted = true;
            frontier = frontier.min(node_bound(
                instance,
                &fixed,
                frame.row_total,
                frame.free_mass,
                &candidates[frame.cursor.min(candidates.len())..],
                scratch,
            ));
            continue;
        }
        nodes += 1;

        let remaining = &candidates[frame.cursor.min(candidates.len())..];
        let bound = node_bound(
            instance,
            &fixed,
            frame.row_total,
            frame.free_mass,
            remaining,
            scratch,
        );

        // Exact leaf: all exclusion mass assigned, negative side solved exactly.
        if frame.free_mass <= 1e-12 {
            frontier = frontier.min(bound);
            if bound < outcome.upper {
                outcome.upper = bound;
            }
            continue;
        }
        if bound >= outcome.upper - controls.absolute_gap {
            frontier = frontier.min(bound);
            continue;
        }
        if frame.cursor >= candidates.len() {
            frontier = frontier.min(bound);
            continue;
        }

        let candidate = candidates[frame.cursor];
        let mass = f64::from(positive_counts[candidate as usize]).min(frame.free_mass);
        // Branch 1: candidate is not excluded.
        stack.push(Frame {
            cursor: frame.cursor + 1,
            fixed_len: fixed.len(),
            free_mass: frame.free_mass,
            row_total: frame.row_total,
        });
        // Branch 2: candidate is excluded, explored first.
        fixed.push((candidate, mass));
        stack.push(Frame {
            cursor: frame.cursor + 1,
            fixed_len: fixed.len(),
            free_mass: frame.free_mass - mass,
            row_total: frame.row_total + mass * instance.row_sum(candidate),
        });
    }

    outcome.lower = outcome.lower.max(frontier.min(outcome.upper));
    outcome.nodes = nodes;
    outcome.exact = !budget_exhausted;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PairedEvaluation;

    fn vertices(dimension: usize, gamma: f64) -> Vec<Vec<f64>> {
        let cap = (gamma / dimension as f64).min(1.0);
        let reciprocal = 1.0 / cap;
        let nearest = reciprocal.round();
        let full = if (reciprocal - nearest).abs() <= 1e-12 * reciprocal.abs().max(1.0) {
            nearest as usize
        } else {
            reciprocal.floor() as usize
        }
        .min(dimension);
        let residual = (1.0 - full as f64 * cap).max(0.0);
        let residual = if residual <= 1e-12 { 0.0 } else { residual };

        fn walk(
            dimension: usize,
            full: usize,
            cap: f64,
            residual: f64,
            start: usize,
            selected: &mut Vec<usize>,
            out: &mut Vec<Vec<f64>>,
        ) {
            if selected.len() == full {
                if residual == 0.0 {
                    let mut weights = vec![0.0; dimension];
                    for &index in selected.iter() {
                        weights[index] = cap;
                    }
                    out.push(weights);
                } else {
                    for extra in 0..dimension {
                        if selected.contains(&extra) {
                            continue;
                        }
                        let mut weights = vec![0.0; dimension];
                        for &index in selected.iter() {
                            weights[index] = cap;
                        }
                        weights[extra] = residual;
                        out.push(weights);
                    }
                }
                return;
            }
            for index in start..=dimension.saturating_sub(full - selected.len()) {
                selected.push(index);
                walk(dimension, full, cap, residual, index + 1, selected, out);
                selected.pop();
            }
        }

        let mut out = Vec::new();
        walk(dimension, full, cap, residual, 0, &mut Vec::new(), &mut out);
        out
    }

    fn exhaustive_expanded(
        matrix: &GainMatrix,
        multiplicities: &Multiplicities,
        gamma: f64,
    ) -> f64 {
        let positives: Vec<_> = multiplicities
            .positive()
            .iter()
            .enumerate()
            .flat_map(|(index, &count)| std::iter::repeat_n(index, count as usize))
            .collect();
        let negatives: Vec<_> = multiplicities
            .negative()
            .iter()
            .enumerate()
            .flat_map(|(index, &count)| std::iter::repeat_n(index, count as usize))
            .collect();
        let mut best = f64::INFINITY;
        for positive_weights in vertices(positives.len(), gamma) {
            for negative_weights in vertices(negatives.len(), gamma) {
                let mut value = 0.0;
                for (i, &positive_weight) in positive_weights.iter().enumerate() {
                    for (k, &negative_weight) in negative_weights.iter().enumerate() {
                        value += positive_weight
                            * negative_weight
                            * matrix.get(positives[i], negatives[k]);
                    }
                }
                best = best.min(value);
            }
        }
        best
    }

    #[test]
    fn repeated_cases_with_partial_exclusion_match_expanded_enumeration() {
        let evaluation = PairedEvaluation::new(
            &[1.0, 0.0, 0.0, 0.0],
            &[0.0, 0.0, 0.0, 0.0],
            &[true, true, false, false],
        )
        .unwrap();
        let matrix = GainMatrix::new(&evaluation);
        let multiplicities = Multiplicities::new(
            vec![2u32, 1].into_boxed_slice(),
            vec![2u32, 1].into_boxed_slice(),
        );
        let marginals = Marginals::new(&matrix, &multiplicities);

        for gamma in [1.2, 1.5, 2.0, 3.0] {
            let instance = Instance::new(&matrix, &multiplicities, &marginals, gamma);
            let mut scratch = SolverScratch::default();
            let outcome = solve(
                &instance,
                SolveControls {
                    absolute_gap: 0.0,
                    relative_gap: 0.0,
                    node_budget: 50_000,
                    restarts: 1,
                    resolve_against: None,
                    time_limit_seconds: None,
                },
                &mut scratch,
            );
            let exact = exhaustive_expanded(&matrix, &multiplicities, gamma);
            assert!(
                outcome.lower <= exact + 1e-12 && outcome.upper >= exact - 1e-12,
                "Gamma={gamma}: outcome {outcome:?} does not bracket {exact}"
            );
            assert!(
                (outcome.upper - exact).abs() < 1e-12,
                "Gamma={gamma}: outcome {outcome:?}, expected {exact}"
            );
        }
    }
}
