//! Backend dispatch and the optimisation certificate.
//!
//! Two backends compute the same quantity, Equation (auroc-z-gamma):
//!
//! * [`AuRocBackend::Combinatorial`] — the production solver. Exact greedy on
//!   one class, two complementary relaxations, and verification-only branch and
//!   bound. Scales to full same-design evaluations.
//! * [`AuRocBackend::Highs`] — the original mixed-integer linearisation, kept
//!   as a reference implementation for small problems. It materialises product
//!   variables for every pair and is therefore unusable at full design, but it
//!   is an independent check on the combinatorial solver and must stay.
//!
//! Differential tests compare the two, and both against exhaustive vertex
//! enumeration, on small evaluations.

#[cfg(feature = "highs-reference")]
use std::ffi::CStr;
#[cfg(feature = "highs-reference")]
use std::num::NonZeroU32;

#[cfg(feature = "highs-reference")]
use highs::{Col, HighsModelStatus, HighsSolutionStatus, RowProblem, Sense};
use serde::{Deserialize, Serialize};

use super::gain::{GainMatrix, Marginals, Multiplicities};
use super::solver::{Instance, SolveControls, SolverScratch, solve};
use super::{AuRocOptimizationOptions, ConcentrationFactor};
use crate::Error;

#[cfg(feature = "highs-reference")]
const VERTEX_TOLERANCE: f64 = 1e-12;

/// Largest expanded problem the reference backend will accept.
#[cfg(feature = "highs-reference")]
const HIGHS_REFERENCE_LIMIT: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuRocBackend {
    #[default]
    Combinatorial,
    Highs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationStatus {
    /// Closed form: `Gamma = 1`, or both caps saturated.
    Analytic,
    /// Proved optimal.
    Optimal,
    /// Interval within the declared absolute or relative gap.
    GapSatisfied,
    /// Interval lies strictly on one side of the threshold under test.
    ///
    /// Sufficient for the policy decision without pinning the value, which is
    /// what the appendix's one-sided certification requirement permits.
    DecisionResolved,
    /// Node budget exhausted. The interval is valid but wider than requested.
    BudgetExhausted,
    TimeLimit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationCertificate {
    pub concentration_factor: ConcentrationFactor,
    pub global_lower_bound: f64,
    pub feasible_upper_bound: f64,
    pub absolute_gap: f64,
    pub relative_gap: f64,
    pub status: OptimizationStatus,
    #[serde(default)]
    pub branch_nodes: usize,
}

impl OptimizationCertificate {
    pub fn point_estimate(&self) -> f64 {
        (self.global_lower_bound + self.feasible_upper_bound) / 2.0
    }

    /// Valid interval for the adversarial value.
    ///
    /// Callers that only need a policy decision should use this rather than a
    /// point estimate: a wide interval on one side of the threshold is a
    /// complete answer, while a point estimate discards the certificate.
    pub fn bounds(&self) -> (f64, f64) {
        (self.global_lower_bound, self.feasible_upper_bound)
    }

    pub fn is_certified(&self, options: AuRocOptimizationOptions) -> bool {
        self.absolute_gap <= options.absolute_gap
            || self.relative_gap <= options.relative_gap
            || matches!(
                self.status,
                OptimizationStatus::Analytic
                    | OptimizationStatus::Optimal
                    | OptimizationStatus::DecisionResolved
            )
    }
}

/// Minimises the paired AUROC difference over the canonical weight family.
///
/// `resolve_against` enables decision-directed stopping: when supplied, the
/// search halts as soon as the interval sits strictly on one side of that
/// threshold. Passing `None` requests a value certified to the declared gap.
pub(crate) fn minimize(
    matrix: &GainMatrix,
    multiplicities: &Multiplicities,
    marginals: &Marginals,
    gamma: ConcentrationFactor,
    options: AuRocOptimizationOptions,
    resolve_against: Option<f64>,
    scratch: &mut SolverScratch,
) -> Result<OptimizationCertificate, Error> {
    options.validate()?;
    if multiplicities.is_degenerate() {
        return Err(Error::MissingClass);
    }
    match options.backend {
        AuRocBackend::Combinatorial => Ok(combinatorial(
            matrix,
            multiplicities,
            marginals,
            gamma,
            options,
            resolve_against,
            scratch,
        )),
        AuRocBackend::Highs => {
            #[cfg(feature = "highs-reference")]
            {
                highs_reference(matrix, multiplicities, gamma, options)
            }
            #[cfg(not(feature = "highs-reference"))]
            {
                Err(Error::ReferenceBackendUnavailable)
            }
        }
    }
}

fn combinatorial(
    matrix: &GainMatrix,
    multiplicities: &Multiplicities,
    marginals: &Marginals,
    gamma: ConcentrationFactor,
    options: AuRocOptimizationOptions,
    resolve_against: Option<f64>,
    scratch: &mut SolverScratch,
) -> OptimizationCertificate {
    let instance = Instance::new(matrix, multiplicities, marginals, gamma.get());
    let controls = SolveControls {
        absolute_gap: options.absolute_gap,
        relative_gap: options.relative_gap,
        node_budget: options.node_budget,
        restarts: options.restarts,
        resolve_against,
        time_limit_seconds: options.time_limit_seconds,
    };
    let outcome = solve(&instance, controls, scratch);
    let absolute_gap = (outcome.upper - outcome.lower).max(0.0);
    let relative_gap = absolute_gap / outcome.upper.abs().max(1.0);
    let status = if outcome.nodes == 0 && absolute_gap <= 0.0 {
        OptimizationStatus::Analytic
    } else if absolute_gap <= f64::EPSILON {
        OptimizationStatus::Optimal
    } else if absolute_gap <= options.absolute_gap || relative_gap <= options.relative_gap {
        OptimizationStatus::GapSatisfied
    } else if outcome.decision_resolved {
        OptimizationStatus::DecisionResolved
    } else if outcome.time_limit_reached {
        OptimizationStatus::TimeLimit
    } else if outcome.exact {
        OptimizationStatus::Optimal
    } else {
        OptimizationStatus::BudgetExhausted
    };
    OptimizationCertificate {
        concentration_factor: gamma,
        global_lower_bound: outcome.lower,
        feasible_upper_bound: outcome.upper,
        absolute_gap,
        relative_gap,
        status,
        branch_nodes: outcome.nodes,
    }
}

/// Expands multiplicities into explicit cases for the reference backend.
#[cfg(feature = "highs-reference")]
fn expand(multiplicities: &[u32]) -> Vec<usize> {
    let mut out = Vec::new();
    for (index, &count) in multiplicities.iter().enumerate() {
        out.extend(std::iter::repeat_n(index, count as usize));
    }
    out
}

#[cfg(feature = "highs-reference")]
fn highs_reference(
    matrix: &GainMatrix,
    multiplicities: &Multiplicities,
    gamma: ConcentrationFactor,
    options: AuRocOptimizationOptions,
) -> Result<OptimizationCertificate, Error> {
    let positives = expand(multiplicities.positive());
    let negatives = expand(multiplicities.negative());
    let positive_count = positives.len();
    let negative_count = negatives.len();
    if positive_count * negative_count > HIGHS_REFERENCE_LIMIT {
        return Err(Error::ReferenceBackendTooLarge {
            pairs: positive_count * negative_count,
            limit: HIGHS_REFERENCE_LIMIT,
        });
    }
    let at = |positive: usize, negative: usize| -> f64 {
        matrix.get(positives[positive], negatives[negative])
    };

    if gamma.get() == 1.0 {
        let mut total = 0.0;
        for positive in 0..positive_count {
            for negative in 0..negative_count {
                total += at(positive, negative);
            }
        }
        return Ok(analytic(
            gamma,
            total / (positive_count * negative_count) as f64,
        ));
    }
    if gamma.get() >= positive_count.max(negative_count) as f64 {
        let mut minimum = f64::INFINITY;
        for positive in 0..positive_count {
            for negative in 0..negative_count {
                minimum = minimum.min(at(positive, negative));
            }
        }
        return Ok(analytic(gamma, minimum));
    }

    let binary_is_positive = positive_count <= negative_count;
    let (binary_count, continuous_count) = if binary_is_positive {
        (positive_count, negative_count)
    } else {
        (negative_count, positive_count)
    };
    let binary_cap = (gamma.get() / binary_count as f64).min(1.0);
    let continuous_cap = (gamma.get() / continuous_count as f64).min(1.0);
    let vertex = VertexPattern::new(binary_cap, binary_count);

    let mut problem = RowProblem::new();
    let full_selectors: Vec<_> = (0..binary_count)
        .map(|_| problem.add_integer_column(0.0, 0..=1))
        .collect();
    let residual_selectors: Option<Vec<_>> = (vertex.residual > 0.0).then(|| {
        (0..binary_count)
            .map(|_| problem.add_integer_column(0.0, 0..=1))
            .collect()
    });
    let continuous_weights: Vec<_> = (0..continuous_count)
        .map(|_| problem.add_column(0.0, 0.0..=continuous_cap))
        .collect();

    let mut products = Vec::new();
    add_products(
        &mut problem,
        &mut products,
        &full_selectors,
        vertex.cap,
        &continuous_weights,
        continuous_cap,
        &at,
        binary_is_positive,
    );
    if let Some(selectors) = &residual_selectors {
        add_products(
            &mut problem,
            &mut products,
            selectors,
            vertex.residual,
            &continuous_weights,
            continuous_cap,
            &at,
            binary_is_positive,
        );
    }

    problem.add_row(
        vertex.full_count as f64..=vertex.full_count as f64,
        full_selectors.iter().copied().map(|column| (column, 1.0)),
    );
    if let Some(residual) = &residual_selectors {
        problem.add_row(
            1.0..=1.0,
            residual.iter().copied().map(|column| (column, 1.0)),
        );
        for (&full, &partial) in full_selectors.iter().zip(residual) {
            problem.add_row(..=1.0, [(full, 1.0), (partial, 1.0)]);
        }
    }
    problem.add_row(
        1.0..=1.0,
        continuous_weights
            .iter()
            .copied()
            .map(|column| (column, 1.0)),
    );
    for product in products {
        problem.add_row(
            ..=0.0,
            [(product.product, 1.0), (product.binary, -continuous_cap)],
        );
        problem.add_row(..=0.0, [(product.product, 1.0), (product.continuous, -1.0)]);
        problem.add_row(
            -continuous_cap..,
            [
                (product.product, 1.0),
                (product.continuous, -1.0),
                (product.binary, -continuous_cap),
            ],
        );
    }

    let mut model = problem.optimise(Sense::Minimise);
    model.make_quiet();
    model.set_option("mip_abs_gap", options.absolute_gap);
    model.set_option("mip_rel_gap", options.relative_gap);
    model.set_threads(
        NonZeroU32::new(options.solver_threads).ok_or(Error::InvalidOptimizationThreadCount)?,
    );
    if let Some(seconds) = options.time_limit_seconds {
        model.set_option("time_limit", seconds);
    }
    let solved = model
        .try_solve()
        .map_err(|status| Error::OptimizationFailed(format!("HiGHS status {status:?}")))?;
    if solved.primal_solution_status() != HighsSolutionStatus::Feasible {
        return Err(Error::OptimizationFailed(format!(
            "no feasible weighting; model status {:?}",
            solved.status()
        )));
    }
    let upper_bound = solved.objective_value();
    let dual_key: &CStr = c"mip_dual_bound";
    let lower_bound = solved
        .double_info_value(dual_key)
        .map_err(|status| Error::OptimizationFailed(format!("missing dual bound: {status:?}")))?;
    if !(lower_bound.is_finite() && upper_bound.is_finite()) {
        return Err(Error::OptimizationFailed(format!(
            "non-finite optimization interval [{lower_bound}, {upper_bound}]"
        )));
    }
    let lower_bound = lower_bound.min(upper_bound);
    let absolute_gap = (upper_bound - lower_bound).max(0.0);
    let relative_gap = absolute_gap / upper_bound.abs().max(1.0);
    let status = match solved.status() {
        HighsModelStatus::Optimal if absolute_gap <= f64::EPSILON => OptimizationStatus::Optimal,
        HighsModelStatus::Optimal => OptimizationStatus::GapSatisfied,
        HighsModelStatus::ReachedTimeLimit => OptimizationStatus::TimeLimit,
        other => {
            return Err(Error::OptimizationFailed(format!(
                "unexpected model status {other:?}"
            )));
        }
    };
    Ok(OptimizationCertificate {
        concentration_factor: gamma,
        global_lower_bound: lower_bound,
        feasible_upper_bound: upper_bound,
        absolute_gap,
        relative_gap,
        status,
        branch_nodes: 0,
    })
}

#[cfg(feature = "highs-reference")]
fn analytic(gamma: ConcentrationFactor, value: f64) -> OptimizationCertificate {
    OptimizationCertificate {
        concentration_factor: gamma,
        global_lower_bound: value,
        feasible_upper_bound: value,
        absolute_gap: 0.0,
        relative_gap: 0.0,
        status: OptimizationStatus::Analytic,
        branch_nodes: 0,
    }
}

#[cfg(feature = "highs-reference")]
#[derive(Debug, Clone, Copy)]
struct VertexPattern {
    cap: f64,
    full_count: usize,
    residual: f64,
}

#[cfg(feature = "highs-reference")]
impl VertexPattern {
    fn new(cap: f64, dimension: usize) -> Self {
        let reciprocal = 1.0 / cap;
        let nearest = reciprocal.round();
        let full_count =
            if (reciprocal - nearest).abs() <= VERTEX_TOLERANCE * reciprocal.abs().max(1.0) {
                nearest as usize
            } else {
                reciprocal.floor() as usize
            }
            .min(dimension);
        let residual = (1.0 - full_count as f64 * cap).max(0.0);
        let residual = if residual <= VERTEX_TOLERANCE {
            0.0
        } else {
            residual
        };
        Self {
            cap,
            full_count,
            residual,
        }
    }
}

#[cfg(feature = "highs-reference")]
#[derive(Debug, Clone, Copy)]
struct Product {
    binary: Col,
    continuous: Col,
    product: Col,
}

#[allow(clippy::too_many_arguments)]
#[cfg(feature = "highs-reference")]
fn add_products<F: Fn(usize, usize) -> f64>(
    problem: &mut RowProblem,
    products: &mut Vec<Product>,
    selectors: &[Col],
    selector_weight: f64,
    continuous_weights: &[Col],
    continuous_cap: f64,
    at: &F,
    binary_is_positive: bool,
) {
    for (binary_index, &binary) in selectors.iter().enumerate() {
        for (continuous_index, &continuous) in continuous_weights.iter().enumerate() {
            let pair_gain = if binary_is_positive {
                at(binary_index, continuous_index)
            } else {
                at(continuous_index, binary_index)
            };
            let product = problem.add_column(selector_weight * pair_gain, 0.0..=continuous_cap);
            products.push(Product {
                binary,
                continuous,
                product,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PairedEvaluation;

    #[cfg(feature = "highs-reference")]
    fn certificates(
        evaluation: &PairedEvaluation,
        gamma: f64,
    ) -> (OptimizationCertificate, OptimizationCertificate) {
        let matrix = GainMatrix::new(evaluation);
        let multiplicities = matrix.identity_multiplicities();
        let marginals = Marginals::new(&matrix, &multiplicities);
        let factor = ConcentrationFactor::new(gamma).unwrap();
        let mut scratch = SolverScratch::default();
        let combinatorial = minimize(
            &matrix,
            &multiplicities,
            &marginals,
            factor,
            AuRocOptimizationOptions::default(),
            None,
            &mut scratch,
        )
        .unwrap();
        let reference = minimize(
            &matrix,
            &multiplicities,
            &marginals,
            factor,
            AuRocOptimizationOptions {
                backend: AuRocBackend::Highs,
                ..AuRocOptimizationOptions::default()
            },
            None,
            &mut scratch,
        )
        .unwrap();
        (combinatorial, reference)
    }

    #[test]
    #[cfg(feature = "highs-reference")]
    fn backends_agree_on_a_small_evaluation() {
        let evaluation = PairedEvaluation::new(
            &[0.9, 0.6, 0.3, 0.1, 0.8, 0.5],
            &[0.7, 0.8, 0.4, 0.2, 0.9, 0.1],
            &[true, true, true, false, false, false],
        )
        .unwrap();
        for gamma in [1.0, 1.2, 1.5, 2.0, 3.0] {
            let (combinatorial, reference) = certificates(&evaluation, gamma);
            assert!(
                (combinatorial.point_estimate() - reference.point_estimate()).abs() < 1e-7,
                "Gamma={gamma}: {combinatorial:?} vs {reference:?}"
            );
            assert!(
                combinatorial.global_lower_bound <= reference.point_estimate() + 1e-9,
                "lower bound is not valid at Gamma={gamma}"
            );
            assert!(
                combinatorial.feasible_upper_bound >= reference.point_estimate() - 1e-9,
                "upper bound is not feasible at Gamma={gamma}"
            );
        }
    }

    #[test]
    fn gamma_one_is_the_ordinary_paired_difference() {
        let evaluation = PairedEvaluation::new(
            &[0.9, 0.6, 0.3, 0.1],
            &[0.7, 0.8, 0.4, 0.2],
            &[true, true, false, false],
        )
        .unwrap();
        let matrix = GainMatrix::new(&evaluation);
        let multiplicities = matrix.identity_multiplicities();
        let marginals = Marginals::new(&matrix, &multiplicities);
        let mut scratch = SolverScratch::default();
        let combinatorial = minimize(
            &matrix,
            &multiplicities,
            &marginals,
            ConcentrationFactor::new(1.0).unwrap(),
            AuRocOptimizationOptions::default(),
            None,
            &mut scratch,
        )
        .unwrap();
        assert_eq!(combinatorial.status, OptimizationStatus::Analytic);
        assert_eq!(combinatorial.absolute_gap, 0.0);
        assert!(
            (combinatorial.point_estimate() - crate::paired_auroc_difference(&evaluation)).abs()
                < 1e-15
        );
    }
}
