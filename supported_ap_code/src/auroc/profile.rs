use rand::Rng;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

use super::gain::{GainMatrix, Marginals, Multiplicities};
use super::optimize::minimize;
use super::solver::SolverScratch;
use super::{
    AuRocOptimizationOptions, AuRocPolicy, ConcentrationFactor, ConcentrationSearchOptions,
    NestedAuRocAssessment, ObservedAuRocAssessment, OptimizationCertificate,
    ProjectedAuRocAssessment,
};
use crate::execution::map_indices;
use crate::random::rng_for;
use crate::{
    AnchoredEffectRow, ClassCounts, Error, Execution, NestedLiteralSurvival, PairedEvaluation,
    ReferenceAssessment, SupportOrder, anchored_nested_support, literal_survival, support,
};

const AUROC_PROJECTED_RESAMPLING_DOMAIN: u64 = 0x4155_524f_4342_4f4f;
const AUROC_NESTED_RESAMPLING_DOMAIN: u64 = 0x4155_524f_434e_5354;

/// Geometric ladder used to bracket the breakdown before bisecting.
///
/// The old search bracketed `[1, max(n_+, n_-)]`, so with a full-design
/// evaluation its first midpoint sat near `Gamma = 2849` — deep in the region
/// where every relaxation is loose and nothing is learned. Breakdown factors
/// live just above one, so the ladder finds a tight bracket in a handful of
/// evaluations and bisection then has little left to do.
const LADDER: [f64; 12] = [
    1.01, 1.02, 1.05, 1.10, 1.15, 1.20, 1.35, 1.50, 2.00, 3.00, 5.00, 10.0,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuRocEvidence {
    Projected,
    Observed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FirstFailure {
    Magnitude,
    LiteralSurvival,
    Both,
}

/// Whether a profile point settles the replacement policy.
///
/// The appendix makes certification one-sided: a feasible weighting can
/// demonstrate failure, but establishing that a requirement still holds needs a
/// valid global lower bound. `Unresolved` is therefore a real outcome and must
/// not be collapsed into either verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyVerdict {
    VerifiedPass,
    VerifiedFailure,
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuRocBreakdown {
    ObservedGateFailure {
        first_failure: FirstFailure,
    },
    AtBaseline {
        first_failure: FirstFailure,
    },
    Finite {
        last_passing: ConcentrationFactor,
        first_failing: ConcentrationFactor,
        first_failure: FirstFailure,
        /// True when the bracket interior contains points the solver could not
        /// resolve within budget. The breakdown lies inside the bracket either
        /// way; this records that the bracket is wider than the search
        /// tolerance because of optimisation limits, not step size.
        unresolved_interior: bool,
    },
    NoBreakdownOnEmpiricalSupport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuRocProfilePoint {
    pub concentration_factor: ConcentrationFactor,
    pub supported_magnitude: f64,
    pub supported_magnitude_lower: f64,
    pub supported_magnitude_upper: f64,
    pub literal_survival_fraction: f64,
    pub literal_survival_lower: f64,
    pub literal_survival_upper: f64,
    pub survivor_count: usize,
    pub list_length: usize,
    pub maximum_optimization_gap: f64,
    pub branch_nodes: usize,
    pub magnitude_requirement_met: bool,
    pub survival_requirement_met: bool,
    pub verdict: PolicyVerdict,
    #[serde(skip)]
    pub(crate) replication_effects: Vec<f64>,
}

impl AuRocProfilePoint {
    fn first_failure(&self) -> FirstFailure {
        match (
            self.magnitude_requirement_met,
            self.survival_requirement_met,
        ) {
            (false, true) => FirstFailure::Magnitude,
            (true, false) => FirstFailure::LiteralSurvival,
            _ => FirstFailure::Both,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuRocBreakdownEstimate {
    pub evidence: AuRocEvidence,
    pub breakdown: AuRocBreakdown,
    pub policy: AuRocPolicy,
    pub empirical_order: SupportOrder,
    pub computational_order: Option<SupportOrder>,
    pub reference_assessment: ReferenceAssessment,
    pub evaluation_counts: Vec<ClassCounts>,
    pub observed_gate: AuRocProfilePoint,
    pub baseline: AuRocProfilePoint,
    pub boundary: AuRocProfilePoint,
    pub evaluated_profile: Vec<AuRocProfilePoint>,
}

/// One concentration-profile point for the full nested AUROC challenge.
///
/// The lower and upper fields propagate the optimizer certificates through the
/// coordinatewise-monotone nested operator. Only those certified bounds, never
/// the midpoint, determine `verdict`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NestedAuRocProfilePoint {
    pub concentration_factor: ConcentrationFactor,
    pub supported_magnitude: f64,
    pub supported_magnitude_lower: f64,
    pub supported_magnitude_upper: f64,
    pub literal_survival: NestedLiteralSurvival,
    pub literal_survival_lower: NestedLiteralSurvival,
    pub literal_survival_upper: NestedLiteralSurvival,
    pub maximum_optimization_gap: f64,
    pub branch_nodes: usize,
    pub magnitude_requirement_met: bool,
    pub survival_requirement_met: bool,
    pub verdict: PolicyVerdict,
}

impl NestedAuRocProfilePoint {
    fn first_failure(&self) -> FirstFailure {
        match (
            self.magnitude_requirement_met,
            self.survival_requirement_met,
        ) {
            (false, true) => FirstFailure::Magnitude,
            (true, false) => FirstFailure::LiteralSurvival,
            _ => FirstFailure::Both,
        }
    }
}

/// Staged nested AUROC breakdown assessment.
///
/// If the observed empirical gate is not a verified pass, no computational
/// rows are generated and the full-profile fields remain absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NestedAuRocBreakdownEstimate {
    pub breakdown: AuRocBreakdown,
    pub policy: AuRocPolicy,
    pub empirical_order: SupportOrder,
    pub computational_order: SupportOrder,
    pub reference_assessment: ReferenceAssessment,
    pub evaluation_counts: Vec<ClassCounts>,
    pub observed_gate: AuRocProfilePoint,
    pub baseline: Option<NestedAuRocProfilePoint>,
    pub boundary: Option<NestedAuRocProfilePoint>,
    pub evaluated_profile: Vec<NestedAuRocProfilePoint>,
}

/// One evaluation of the adversarial problem: counts plus their marginals.
///
/// `Marginals` is independent of `Gamma`, so it is built once per replication
/// and reused across the whole search. That reuse is why cost scales with the
/// number of replications rather than the number of searched factors.
pub(crate) struct Replication<'a> {
    matrix: &'a GainMatrix,
    multiplicities: Multiplicities,
    marginals: Marginals,
    scratch: Mutex<SolverScratch>,
}

impl<'a> Replication<'a> {
    fn new(matrix: &'a GainMatrix, multiplicities: Multiplicities) -> Self {
        let marginals = Marginals::new(matrix, &multiplicities);
        Self {
            matrix,
            multiplicities,
            marginals,
            scratch: Mutex::new(SolverScratch::default()),
        }
    }
}

pub fn estimate_projected_auroc_breakdown(
    evaluation: &PairedEvaluation,
    assessment: &ProjectedAuRocAssessment,
    policy: AuRocPolicy,
) -> Result<AuRocBreakdownEstimate, Error> {
    assessment.resampling.validate()?;
    assessment.optimization.validate()?;
    assessment.concentration_search.validate()?;
    assessment.reference_assessment.validate()?;
    let matrix = GainMatrix::new(evaluation);
    let observed = vec![Replication::new(&matrix, matrix.identity_multiplicities())];
    let empirical_order = SupportOrder::new(1).expect("one is a valid support order");
    let baseline_factor = ConcentrationFactor::new(1.0)?;
    let observed_gate = evaluate(
        &observed,
        None,
        baseline_factor,
        empirical_order,
        policy,
        assessment.optimization,
        assessment.resampling.execution,
    )?;
    if observed_gate.verdict != PolicyVerdict::VerifiedPass {
        return Ok(assemble(
            AuRocEvidence::Projected,
            AuRocBreakdown::ObservedGateFailure {
                first_failure: observed_gate.first_failure(),
            },
            observed_gate.clone(),
            observed_gate.clone(),
            vec![observed_gate.clone()],
            policy,
            empirical_order,
            Some(assessment.resampling.computational_order),
            assessment.reference_assessment.clone(),
            vec![evaluation.class_counts()],
            observed_gate,
        ));
    }
    let replications = projected_replications(&matrix, &assessment.resampling);
    let saturation = saturation_factor(&replications).max(saturation_factor(&observed));
    search_breakdown(
        AuRocEvidence::Projected,
        policy,
        empirical_order,
        Some(assessment.resampling.computational_order),
        assessment.concentration_search,
        assessment.reference_assessment.clone(),
        vec![evaluation.class_counts()],
        observed_gate,
        saturation,
        |gamma| {
            evaluate(
                &replications,
                Some(&observed[0]),
                gamma,
                assessment.resampling.computational_order,
                policy,
                assessment.optimization,
                assessment.resampling.execution,
            )
        },
    )
}

pub fn estimate_observed_auroc_breakdown(
    evaluations: &[PairedEvaluation],
    assessment: &ObservedAuRocAssessment,
    policy: AuRocPolicy,
) -> Result<AuRocBreakdownEstimate, Error> {
    if evaluations.is_empty() {
        return Err(Error::EmptyEmpiricalEvaluations);
    }
    if assessment.empirical_order.get() > evaluations.len() {
        return Err(Error::SupportOrderExceedsList {
            order: assessment.empirical_order.get(),
            list_length: evaluations.len(),
        });
    }
    assessment.optimization.validate()?;
    assessment.concentration_search.validate()?;
    assessment.reference_assessment.validate()?;
    let matrices: Vec<_> = evaluations.iter().map(GainMatrix::new).collect();
    let replications: Vec<_> = matrices
        .iter()
        .map(|matrix| Replication::new(matrix, matrix.identity_multiplicities()))
        .collect();
    let saturation = saturation_factor(&replications);
    let observed_gate = evaluate(
        &replications,
        None,
        ConcentrationFactor::new(1.0)?,
        assessment.empirical_order,
        policy,
        assessment.optimization,
        assessment.execution,
    )?;
    search_breakdown(
        AuRocEvidence::Observed,
        policy,
        assessment.empirical_order,
        None,
        assessment.concentration_search,
        assessment.reference_assessment.clone(),
        evaluations
            .iter()
            .map(PairedEvaluation::class_counts)
            .collect(),
        observed_gate,
        saturation,
        |gamma| {
            evaluate(
                &replications,
                None,
                gamma,
                assessment.empirical_order,
                policy,
                assessment.optimization,
                assessment.execution,
            )
        },
    )
}

/// Estimates the AUROC case-mix breakdown for the complete
/// empirical--computational nested challenge.
///
/// The observed order-`K_E` gate is evaluated before computational
/// multiplicities are drawn. Once that gate passes, every row retains its own
/// observed anchor and its own order-`K_C` computational challenge.
pub fn estimate_nested_auroc_breakdown(
    evaluations: &[PairedEvaluation],
    assessment: &NestedAuRocAssessment,
    policy: AuRocPolicy,
) -> Result<NestedAuRocBreakdownEstimate, Error> {
    assessment.validate(evaluations.len())?;
    let matrices: Vec<_> = evaluations.iter().map(GainMatrix::new).collect();
    let observed: Vec<_> = matrices
        .iter()
        .map(|matrix| Replication::new(matrix, matrix.identity_multiplicities()))
        .collect();
    let baseline_factor = ConcentrationFactor::new(1.0)?;
    let observed_gate = evaluate(
        &observed,
        None,
        baseline_factor,
        assessment.empirical_order,
        policy,
        assessment.optimization,
        assessment.execution,
    )?;
    let evaluation_counts = evaluations
        .iter()
        .map(PairedEvaluation::class_counts)
        .collect();
    if observed_gate.verdict != PolicyVerdict::VerifiedPass {
        return Ok(NestedAuRocBreakdownEstimate {
            breakdown: AuRocBreakdown::ObservedGateFailure {
                first_failure: observed_gate.first_failure(),
            },
            policy,
            empirical_order: assessment.empirical_order,
            computational_order: assessment.computational_order,
            reference_assessment: assessment.reference_assessment.clone(),
            evaluation_counts,
            observed_gate,
            baseline: None,
            boundary: None,
            evaluated_profile: Vec::new(),
        });
    }

    let (computational, offsets) = nested_replications(&matrices, assessment);
    let saturation = saturation_factor(&computational).max(saturation_factor(&observed));
    search_nested_breakdown(
        policy,
        assessment,
        evaluation_counts,
        observed_gate,
        saturation,
        |gamma| {
            evaluate_nested(
                &computational,
                &observed,
                &offsets,
                gamma,
                assessment.empirical_order,
                assessment.computational_order,
                policy,
                assessment.optimization,
                assessment.execution,
            )
        },
    )
}

/// Smallest factor at which every replication's cap is certain to saturate.
fn saturation_factor(replications: &[Replication<'_>]) -> f64 {
    replications
        .iter()
        .map(|replication| {
            let positive = replication.multiplicities.positive_total()
                / f64::from(replication.multiplicities.positive_minimum().max(1));
            let negative = replication.multiplicities.negative_total()
                / f64::from(replication.multiplicities.negative_minimum().max(1));
            positive.max(negative)
        })
        .fold(1.0, f64::max)
}

fn nested_replications<'a>(
    matrices: &'a [GainMatrix],
    assessment: &NestedAuRocAssessment,
) -> (Vec<Replication<'a>>, Vec<usize>) {
    let mut offsets = Vec::with_capacity(assessment.rows.len() + 1);
    offsets.push(0usize);
    for design in &assessment.rows {
        offsets.push(offsets.last().copied().unwrap() + design.replications.get());
    }
    let total = *offsets.last().unwrap();
    let replications = map_indices(total, assessment.execution, |task| {
        let row = offsets.partition_point(|&offset| offset <= task) - 1;
        let replicate = task - offsets[row];
        let design = assessment.rows[row];
        let matrix = &matrices[row];
        let mut rng = rng_for(
            design.seed,
            AUROC_NESTED_RESAMPLING_DOMAIN,
            &[row, replicate],
        );
        Replication::new(
            matrix,
            draw_multiplicities(
                matrix,
                design.replication_counts.positive(),
                design.replication_counts.negative(),
                &mut rng,
            ),
        )
    });
    (replications, offsets)
}

fn projected_replications<'a>(
    matrix: &'a GainMatrix,
    resampling: &crate::ProjectedResampling,
) -> Vec<Replication<'a>> {
    map_indices(
        resampling.replications.get(),
        resampling.execution,
        |replicate| {
            let mut rng = rng_for(
                resampling.seed,
                AUROC_PROJECTED_RESAMPLING_DOMAIN,
                &[replicate],
            );
            Replication::new(
                matrix,
                draw_multiplicities(
                    matrix,
                    resampling.replication_counts.positive(),
                    resampling.replication_counts.negative(),
                    &mut rng,
                ),
            )
        },
    )
}

/// Class-stratified resampling with replacement, recorded as counts.
pub(crate) fn draw_multiplicities<R: Rng + ?Sized>(
    matrix: &GainMatrix,
    positive_draws: usize,
    negative_draws: usize,
    rng: &mut R,
) -> Multiplicities {
    let mut positive = vec![0u32; matrix.positive_count()];
    let mut negative = vec![0u32; matrix.negative_count()];
    for _ in 0..positive_draws {
        positive[rng.gen_range(0..matrix.positive_count())] += 1;
    }
    for _ in 0..negative_draws {
        negative[rng.gen_range(0..matrix.negative_count())] += 1;
    }
    Multiplicities::new(positive.into_boxed_slice(), negative.into_boxed_slice())
}

#[allow(clippy::too_many_arguments)]
fn search_breakdown<F>(
    evidence: AuRocEvidence,
    policy: AuRocPolicy,
    empirical_order: SupportOrder,
    computational_order: Option<SupportOrder>,
    search: ConcentrationSearchOptions,
    reference_assessment: ReferenceAssessment,
    evaluation_counts: Vec<ClassCounts>,
    observed_gate: AuRocProfilePoint,
    saturation: f64,
    mut evaluate: F,
) -> Result<AuRocBreakdownEstimate, Error>
where
    F: FnMut(ConcentrationFactor) -> Result<AuRocProfilePoint, Error>,
{
    let baseline = evaluate(ConcentrationFactor::new(1.0)?)?;
    let mut profile = vec![baseline.clone()];

    if baseline.verdict != PolicyVerdict::VerifiedPass {
        return Ok(assemble(
            evidence,
            AuRocBreakdown::AtBaseline {
                first_failure: baseline.first_failure(),
            },
            baseline.clone(),
            baseline,
            profile,
            policy,
            empirical_order,
            computational_order,
            reference_assessment,
            evaluation_counts,
            observed_gate,
        ));
    }

    // Geometric pre-scan for a tight bracket.
    let mut passing = baseline.clone();
    let mut failing: Option<AuRocProfilePoint> = None;
    let mut unresolved_interior = false;
    for &step in LADDER.iter() {
        if step > saturation {
            break;
        }
        let point = evaluate(ConcentrationFactor::new(step)?)?;
        profile.push(point.clone());
        match point.verdict {
            PolicyVerdict::VerifiedPass => passing = point,
            PolicyVerdict::VerifiedFailure => {
                failing = Some(point);
                break;
            }
            PolicyVerdict::Unresolved => {
                unresolved_interior = true;
            }
        }
    }
    if failing.is_none() {
        let point = evaluate(ConcentrationFactor::new(saturation.max(1.0))?)?;
        profile.push(point.clone());
        match point.verdict {
            PolicyVerdict::VerifiedFailure => failing = Some(point),
            PolicyVerdict::VerifiedPass => {
                return Ok(assemble(
                    evidence,
                    AuRocBreakdown::NoBreakdownOnEmpiricalSupport,
                    baseline,
                    point,
                    profile,
                    policy,
                    empirical_order,
                    computational_order,
                    reference_assessment,
                    evaluation_counts,
                    observed_gate,
                ));
            }
            PolicyVerdict::Unresolved => unresolved_interior = true,
        }
    }

    let Some(mut failing) = failing else {
        return Ok(assemble(
            evidence,
            AuRocBreakdown::NoBreakdownOnEmpiricalSupport,
            baseline,
            passing.clone(),
            profile,
            policy,
            empirical_order,
            computational_order,
            reference_assessment,
            evaluation_counts,
            observed_gate,
        ));
    };

    for _ in 0..search.max_iterations {
        if failing.concentration_factor.get() - passing.concentration_factor.get()
            <= search.tolerance
        {
            break;
        }
        let midpoint = ConcentrationFactor::new(
            (passing.concentration_factor.get() + failing.concentration_factor.get()) / 2.0,
        )?;
        let candidate = evaluate(midpoint)?;
        profile.push(candidate.clone());
        match candidate.verdict {
            PolicyVerdict::VerifiedPass => passing = candidate,
            PolicyVerdict::VerifiedFailure => failing = candidate,
            // Neither half can be excluded, so refinement stops here and the
            // bracket is reported as wider than the search tolerance.
            PolicyVerdict::Unresolved => {
                unresolved_interior = true;
                break;
            }
        }
    }

    let breakdown = AuRocBreakdown::Finite {
        last_passing: passing.concentration_factor,
        first_failing: failing.concentration_factor,
        first_failure: failing.first_failure(),
        unresolved_interior,
    };
    Ok(assemble(
        evidence,
        breakdown,
        baseline,
        failing,
        profile,
        policy,
        empirical_order,
        computational_order,
        reference_assessment,
        evaluation_counts,
        observed_gate,
    ))
}

#[allow(clippy::too_many_arguments)]
fn assemble(
    evidence: AuRocEvidence,
    breakdown: AuRocBreakdown,
    baseline: AuRocProfilePoint,
    boundary: AuRocProfilePoint,
    mut profile: Vec<AuRocProfilePoint>,
    policy: AuRocPolicy,
    empirical_order: SupportOrder,
    computational_order: Option<SupportOrder>,
    reference_assessment: ReferenceAssessment,
    evaluation_counts: Vec<ClassCounts>,
    observed_gate: AuRocProfilePoint,
) -> AuRocBreakdownEstimate {
    profile.sort_unstable_by(|left, right| {
        left.concentration_factor
            .get()
            .total_cmp(&right.concentration_factor.get())
    });
    profile.dedup_by(|left, right| left.concentration_factor == right.concentration_factor);
    AuRocBreakdownEstimate {
        evidence,
        breakdown,
        policy,
        empirical_order,
        computational_order,
        reference_assessment,
        evaluation_counts,
        observed_gate,
        baseline,
        boundary,
        evaluated_profile: profile,
    }
}

fn search_nested_breakdown<F>(
    policy: AuRocPolicy,
    assessment: &NestedAuRocAssessment,
    evaluation_counts: Vec<ClassCounts>,
    observed_gate: AuRocProfilePoint,
    saturation: f64,
    mut evaluate: F,
) -> Result<NestedAuRocBreakdownEstimate, Error>
where
    F: FnMut(ConcentrationFactor) -> Result<NestedAuRocProfilePoint, Error>,
{
    let baseline = evaluate(ConcentrationFactor::new(1.0)?)?;
    let mut profile = vec![baseline.clone()];

    if baseline.verdict != PolicyVerdict::VerifiedPass {
        return Ok(assemble_nested(
            AuRocBreakdown::AtBaseline {
                first_failure: baseline.first_failure(),
            },
            baseline.clone(),
            baseline,
            profile,
            policy,
            assessment,
            evaluation_counts,
            observed_gate,
        ));
    }

    let mut passing = baseline.clone();
    let mut failing: Option<NestedAuRocProfilePoint> = None;
    let mut unresolved_interior = false;
    for &step in LADDER.iter() {
        if step > saturation {
            break;
        }
        let point = evaluate(ConcentrationFactor::new(step)?)?;
        profile.push(point.clone());
        match point.verdict {
            PolicyVerdict::VerifiedPass => passing = point,
            PolicyVerdict::VerifiedFailure => {
                failing = Some(point);
                break;
            }
            PolicyVerdict::Unresolved => unresolved_interior = true,
        }
    }
    if failing.is_none() {
        let point = evaluate(ConcentrationFactor::new(saturation.max(1.0))?)?;
        profile.push(point.clone());
        match point.verdict {
            PolicyVerdict::VerifiedFailure => failing = Some(point),
            PolicyVerdict::VerifiedPass => {
                return Ok(assemble_nested(
                    AuRocBreakdown::NoBreakdownOnEmpiricalSupport,
                    baseline,
                    point,
                    profile,
                    policy,
                    assessment,
                    evaluation_counts,
                    observed_gate,
                ));
            }
            PolicyVerdict::Unresolved => unresolved_interior = true,
        }
    }

    let Some(mut failing) = failing else {
        return Ok(assemble_nested(
            AuRocBreakdown::NoBreakdownOnEmpiricalSupport,
            baseline,
            passing.clone(),
            profile,
            policy,
            assessment,
            evaluation_counts,
            observed_gate,
        ));
    };

    for _ in 0..assessment.concentration_search.max_iterations {
        if failing.concentration_factor.get() - passing.concentration_factor.get()
            <= assessment.concentration_search.tolerance
        {
            break;
        }
        let midpoint = ConcentrationFactor::new(
            (passing.concentration_factor.get() + failing.concentration_factor.get()) / 2.0,
        )?;
        let candidate = evaluate(midpoint)?;
        profile.push(candidate.clone());
        match candidate.verdict {
            PolicyVerdict::VerifiedPass => passing = candidate,
            PolicyVerdict::VerifiedFailure => failing = candidate,
            PolicyVerdict::Unresolved => {
                unresolved_interior = true;
                break;
            }
        }
    }

    let breakdown = AuRocBreakdown::Finite {
        last_passing: passing.concentration_factor,
        first_failing: failing.concentration_factor,
        first_failure: failing.first_failure(),
        unresolved_interior,
    };
    Ok(assemble_nested(
        breakdown,
        baseline,
        failing,
        profile,
        policy,
        assessment,
        evaluation_counts,
        observed_gate,
    ))
}

#[allow(clippy::too_many_arguments)]
fn assemble_nested(
    breakdown: AuRocBreakdown,
    baseline: NestedAuRocProfilePoint,
    boundary: NestedAuRocProfilePoint,
    mut profile: Vec<NestedAuRocProfilePoint>,
    policy: AuRocPolicy,
    assessment: &NestedAuRocAssessment,
    evaluation_counts: Vec<ClassCounts>,
    observed_gate: AuRocProfilePoint,
) -> NestedAuRocBreakdownEstimate {
    profile.sort_unstable_by(|left, right| {
        left.concentration_factor
            .get()
            .total_cmp(&right.concentration_factor.get())
    });
    profile.dedup_by(|left, right| left.concentration_factor == right.concentration_factor);
    NestedAuRocBreakdownEstimate {
        breakdown,
        policy,
        empirical_order: assessment.empirical_order,
        computational_order: assessment.computational_order,
        reference_assessment: assessment.reference_assessment.clone(),
        evaluation_counts,
        observed_gate,
        baseline: Some(baseline),
        boundary: Some(boundary),
        evaluated_profile: profile,
    }
}

/// Evaluates one profile point across every replication.
///
/// Two passes. The first is decision-directed against the survival floor, which
/// is all most replications need. Only if the assembled interval fails to settle
/// the policy does the second pass re-solve to the declared gap.
fn evaluate(
    replications: &[Replication<'_>],
    observed_anchor: Option<&Replication<'_>>,
    gamma: ConcentrationFactor,
    order: SupportOrder,
    policy: AuRocPolicy,
    optimization: AuRocOptimizationOptions,
    execution: Execution,
) -> Result<AuRocProfilePoint, Error> {
    let point = solve_all(
        replications,
        observed_anchor,
        gamma,
        order,
        policy,
        optimization,
        execution,
        Some(policy.survival_floor.get()),
    )?;
    if point.verdict != PolicyVerdict::Unresolved {
        return Ok(point);
    }
    solve_all(
        replications,
        observed_anchor,
        gamma,
        order,
        policy,
        optimization,
        execution,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn solve_all(
    replications: &[Replication<'_>],
    observed_anchor: Option<&Replication<'_>>,
    gamma: ConcentrationFactor,
    order: SupportOrder,
    policy: AuRocPolicy,
    optimization: AuRocOptimizationOptions,
    execution: Execution,
    resolve_against: Option<f64>,
) -> Result<AuRocProfilePoint, Error> {
    let certificates = map_indices(replications.len(), execution, |index| {
        let replication = &replications[index];
        let mut scratch = replication
            .scratch
            .lock()
            .expect("AUROC solver scratch mutex was poisoned");
        minimize(
            replication.matrix,
            &replication.multiplicities,
            &replication.marginals,
            gamma,
            optimization,
            resolve_against,
            &mut scratch,
        )
    });
    let certificates: Vec<_> = certificates.into_iter().collect::<Result<_, _>>()?;

    let anchor_certificate = if let Some(anchor) = observed_anchor {
        let mut scratch = anchor
            .scratch
            .lock()
            .expect("AUROC solver scratch mutex was poisoned");
        Some(minimize(
            anchor.matrix,
            &anchor.multiplicities,
            &anchor.marginals,
            gamma,
            optimization,
            resolve_against,
            &mut scratch,
        )?)
    } else {
        None
    };

    let lower: Vec<_> = certificates
        .iter()
        .map(|certificate| {
            anchor_certificate
                .as_ref()
                .map_or(certificate.global_lower_bound, |anchor| {
                    anchor
                        .global_lower_bound
                        .min(certificate.global_lower_bound)
                })
        })
        .collect();
    let upper: Vec<_> = certificates
        .iter()
        .map(|certificate| {
            anchor_certificate
                .as_ref()
                .map_or(certificate.feasible_upper_bound, |anchor| {
                    anchor
                        .feasible_upper_bound
                        .min(certificate.feasible_upper_bound)
                })
        })
        .collect();
    let midpoint: Vec<_> = certificates
        .iter()
        .map(|certificate| {
            anchor_certificate.as_ref().map_or_else(
                || certificate.point_estimate(),
                |anchor| anchor.point_estimate().min(certificate.point_estimate()),
            )
        })
        .collect();

    // S_K and psi are coordinatewise nondecreasing in the replication effects,
    // so bounding every effect bounds both profiles.
    let magnitude_lower = support(&lower, order)?;
    let magnitude_upper = support(&upper, order)?;
    let magnitude = support(&midpoint, order)?;
    let survival_lower = literal_survival(&lower, order, policy.survival_floor)?;
    let survival_upper = literal_survival(&upper, order, policy.survival_floor)?;
    let survival = literal_survival(&midpoint, order, policy.survival_floor)?;

    let delta = policy.magnitude_threshold.get();
    let required = policy.survival_requirement.get();
    let verified_pass = magnitude_lower > delta && survival_lower.subset_fraction > required;
    let verified_failure = magnitude_upper <= delta || survival_upper.subset_fraction <= required;
    let verdict = if verified_pass {
        PolicyVerdict::VerifiedPass
    } else if verified_failure {
        PolicyVerdict::VerifiedFailure
    } else {
        PolicyVerdict::Unresolved
    };

    Ok(AuRocProfilePoint {
        concentration_factor: gamma,
        supported_magnitude: magnitude,
        supported_magnitude_lower: magnitude_lower,
        supported_magnitude_upper: magnitude_upper,
        literal_survival_fraction: survival.subset_fraction,
        literal_survival_lower: survival_lower.subset_fraction,
        literal_survival_upper: survival_upper.subset_fraction,
        survivor_count: survival.survivor_count,
        list_length: survival.list_length,
        maximum_optimization_gap: certificates
            .iter()
            .map(|certificate| certificate.absolute_gap)
            .chain(
                anchor_certificate
                    .iter()
                    .map(|certificate| certificate.absolute_gap),
            )
            .fold(0.0, f64::max),
        branch_nodes: certificates
            .iter()
            .map(|certificate| certificate.branch_nodes)
            .chain(
                anchor_certificate
                    .iter()
                    .map(|certificate| certificate.branch_nodes),
            )
            .sum(),
        // Reported against the upper profile: these record what the evidence
        // *fails* to rule out, which is the direction a reader needs.
        magnitude_requirement_met: magnitude_upper > delta,
        survival_requirement_met: survival_upper.subset_fraction > required,
        verdict,
        replication_effects: midpoint,
    })
}

#[allow(clippy::too_many_arguments)]
fn evaluate_nested(
    computational: &[Replication<'_>],
    observed: &[Replication<'_>],
    offsets: &[usize],
    gamma: ConcentrationFactor,
    empirical_order: SupportOrder,
    computational_order: SupportOrder,
    policy: AuRocPolicy,
    optimization: AuRocOptimizationOptions,
    execution: Execution,
) -> Result<NestedAuRocProfilePoint, Error> {
    let point = solve_nested(
        computational,
        observed,
        offsets,
        gamma,
        empirical_order,
        computational_order,
        policy,
        optimization,
        execution,
        Some(policy.survival_floor.get()),
    )?;
    if point.verdict != PolicyVerdict::Unresolved {
        return Ok(point);
    }
    solve_nested(
        computational,
        observed,
        offsets,
        gamma,
        empirical_order,
        computational_order,
        policy,
        optimization,
        execution,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn solve_nested(
    computational: &[Replication<'_>],
    observed: &[Replication<'_>],
    offsets: &[usize],
    gamma: ConcentrationFactor,
    empirical_order: SupportOrder,
    computational_order: SupportOrder,
    policy: AuRocPolicy,
    optimization: AuRocOptimizationOptions,
    execution: Execution,
    resolve_against: Option<f64>,
) -> Result<NestedAuRocProfilePoint, Error> {
    let observed_count = observed.len();
    let certificates = map_indices(observed_count + computational.len(), execution, |task| {
        let replication = if task < observed_count {
            &observed[task]
        } else {
            &computational[task - observed_count]
        };
        let mut scratch = replication
            .scratch
            .lock()
            .expect("AUROC solver scratch mutex was poisoned");
        minimize(
            replication.matrix,
            &replication.multiplicities,
            &replication.marginals,
            gamma,
            optimization,
            resolve_against,
            &mut scratch,
        )
    });
    let certificates: Vec<_> = certificates.into_iter().collect::<Result<_, _>>()?;
    let (observed_certificates, computational_certificates) = certificates.split_at(observed_count);

    let lower_rows = certificate_rows(
        observed_certificates,
        computational_certificates,
        offsets,
        |certificate| certificate.global_lower_bound,
    );
    let midpoint_rows = certificate_rows(
        observed_certificates,
        computational_certificates,
        offsets,
        OptimizationCertificate::point_estimate,
    );
    let upper_rows = certificate_rows(
        observed_certificates,
        computational_certificates,
        offsets,
        |certificate| certificate.feasible_upper_bound,
    );
    let lower = anchored_nested_support(
        &lower_rows,
        empirical_order,
        computational_order,
        policy.survival_floor,
    )?;
    let midpoint = anchored_nested_support(
        &midpoint_rows,
        empirical_order,
        computational_order,
        policy.survival_floor,
    )?;
    let upper = anchored_nested_support(
        &upper_rows,
        empirical_order,
        computational_order,
        policy.survival_floor,
    )?;

    let delta = policy.magnitude_threshold.get();
    let required = policy.survival_requirement.get();
    let verified_pass =
        lower.supported_magnitude > delta && lower.literal_survival.subset_fraction > required;
    let verified_failure =
        upper.supported_magnitude <= delta || upper.literal_survival.subset_fraction <= required;
    let magnitude_requirement_met = upper.supported_magnitude > delta;
    let survival_requirement_met = upper.literal_survival.subset_fraction > required;
    let verdict = if verified_pass {
        PolicyVerdict::VerifiedPass
    } else if verified_failure {
        PolicyVerdict::VerifiedFailure
    } else {
        PolicyVerdict::Unresolved
    };

    Ok(NestedAuRocProfilePoint {
        concentration_factor: gamma,
        supported_magnitude: midpoint.supported_magnitude,
        supported_magnitude_lower: lower.supported_magnitude,
        supported_magnitude_upper: upper.supported_magnitude,
        literal_survival: midpoint.literal_survival,
        literal_survival_lower: lower.literal_survival,
        literal_survival_upper: upper.literal_survival,
        maximum_optimization_gap: certificates
            .iter()
            .map(|certificate| certificate.absolute_gap)
            .fold(0.0, f64::max),
        branch_nodes: certificates
            .iter()
            .map(|certificate| certificate.branch_nodes)
            .sum(),
        magnitude_requirement_met,
        survival_requirement_met,
        verdict,
    })
}

fn certificate_rows<F>(
    observed: &[OptimizationCertificate],
    computational: &[OptimizationCertificate],
    offsets: &[usize],
    value: F,
) -> Vec<AnchoredEffectRow>
where
    F: Fn(&OptimizationCertificate) -> f64,
{
    observed
        .iter()
        .enumerate()
        .map(|(row, anchor)| AnchoredEffectRow {
            observed_effect: value(anchor),
            computational_effects: computational[offsets[row]..offsets[row + 1]]
                .iter()
                .map(&value)
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ComputationalReplicationCount, MagnitudeThreshold, ProjectedResampling,
        ReferenceAssessment, ResamplingUnit, SupportOrder, SurvivalFloor, SurvivalRequirement,
    };

    fn policy(delta: f64, floor: f64, gamma: f64) -> AuRocPolicy {
        AuRocPolicy::new(
            MagnitudeThreshold::new(delta).unwrap(),
            SurvivalFloor::new(floor).unwrap(),
            SurvivalRequirement::new(gamma).unwrap(),
        )
    }

    #[test]
    fn baseline_failure_has_breakdown_one() {
        let evaluation = PairedEvaluation::new(
            &[0.9, 0.8, 0.2, 0.1],
            &[0.9, 0.8, 0.2, 0.1],
            &[true, false, true, false],
        )
        .unwrap();
        let assessment = ProjectedAuRocAssessment {
            resampling: ProjectedResampling::new(
                ComputationalReplicationCount::new(8).unwrap(),
                SupportOrder::new(2).unwrap(),
                evaluation.class_counts(),
                19,
                Execution::Sequential,
                ResamplingUnit::IndependentObservation,
            )
            .unwrap(),
            optimization: super::super::AuRocOptimizationOptions::default(),
            concentration_search: ConcentrationSearchOptions::default(),
            reference_assessment: ReferenceAssessment::not_asserted("unit test").unwrap(),
        };
        let result =
            estimate_projected_auroc_breakdown(&evaluation, &assessment, policy(0.0, 0.0, 0.5))
                .unwrap();
        assert!(matches!(
            result.breakdown,
            AuRocBreakdown::ObservedGateFailure { .. }
        ));
        assert_eq!(result.evaluated_profile.len(), 1);
        assert_eq!(result.baseline.supported_magnitude, 0.0);
    }

    #[test]
    fn observed_profiles_are_monotone_on_evaluated_search_points() {
        let evaluations = vec![
            PairedEvaluation::new(
                &[0.9, 0.7, 0.6, 0.2, 0.1, 0.0],
                &[0.8, 0.6, 0.3, 0.7, 0.2, 0.1],
                &[true, true, true, false, false, false],
            )
            .unwrap(),
            PairedEvaluation::new(
                &[0.95, 0.8, 0.5, 0.4, 0.2, 0.1],
                &[0.7, 0.6, 0.4, 0.8, 0.3, 0.2],
                &[true, true, true, false, false, false],
            )
            .unwrap(),
        ];
        let assessment = ObservedAuRocAssessment {
            empirical_order: SupportOrder::new(2).unwrap(),
            optimization: super::super::AuRocOptimizationOptions::default(),
            concentration_search: ConcentrationSearchOptions::new(0.05, 16).unwrap(),
            execution: Execution::Sequential,
            reference_assessment: ReferenceAssessment::not_asserted("unit test").unwrap(),
        };
        let result =
            estimate_observed_auroc_breakdown(&evaluations, &assessment, policy(0.01, 0.0, 0.2))
                .unwrap();
        for pair in result.evaluated_profile.windows(2) {
            assert!(
                pair[1].supported_magnitude_upper <= pair[0].supported_magnitude_upper + 1e-8,
                "{:?}",
                result.evaluated_profile
            );
            assert!(pair[1].literal_survival_upper <= pair[0].literal_survival_upper + 1e-12);
        }
    }

    #[test]
    fn every_profile_point_brackets_its_midpoint() {
        let evaluations = vec![
            PairedEvaluation::new(
                &[0.9, 0.7, 0.6, 0.2, 0.1, 0.0],
                &[0.8, 0.6, 0.3, 0.7, 0.2, 0.1],
                &[true, true, true, false, false, false],
            )
            .unwrap(),
            PairedEvaluation::new(
                &[0.95, 0.8, 0.5, 0.4, 0.2, 0.1],
                &[0.7, 0.6, 0.4, 0.8, 0.3, 0.2],
                &[true, true, true, false, false, false],
            )
            .unwrap(),
        ];
        let assessment = ObservedAuRocAssessment {
            empirical_order: SupportOrder::new(2).unwrap(),
            optimization: super::super::AuRocOptimizationOptions::default(),
            concentration_search: ConcentrationSearchOptions::new(0.05, 16).unwrap(),
            execution: Execution::Sequential,
            reference_assessment: ReferenceAssessment::not_asserted("unit test").unwrap(),
        };
        let result =
            estimate_observed_auroc_breakdown(&evaluations, &assessment, policy(0.0, 0.0, 0.2))
                .unwrap();
        for point in &result.evaluated_profile {
            assert!(point.supported_magnitude_lower <= point.supported_magnitude + 1e-12);
            assert!(point.supported_magnitude <= point.supported_magnitude_upper + 1e-12);
            assert!(point.literal_survival_lower <= point.literal_survival_upper + 1e-12);
        }
    }

    #[test]
    fn one_row_nested_evaluation_matches_the_projected_operator() {
        let evaluation = PairedEvaluation::new(
            &[0.9, 0.7, 0.6, 0.2, 0.1, 0.0],
            &[0.8, 0.6, 0.3, 0.7, 0.2, 0.1],
            &[true, true, true, false, false, false],
        )
        .unwrap();
        let matrix = GainMatrix::new(&evaluation);
        let observed = vec![Replication::new(&matrix, matrix.identity_multiplicities())];
        let resampling = ProjectedResampling::new(
            ComputationalReplicationCount::new(8).unwrap(),
            SupportOrder::new(3).unwrap(),
            evaluation.class_counts(),
            91,
            Execution::Sequential,
            ResamplingUnit::IndependentObservation,
        )
        .unwrap();
        let computational = projected_replications(&matrix, &resampling);
        let gamma = ConcentrationFactor::new(1.5).unwrap();
        let comparison_policy = policy(0.01, 0.0, 0.2);
        let flat = evaluate(
            &computational,
            Some(&observed[0]),
            gamma,
            resampling.computational_order,
            comparison_policy,
            AuRocOptimizationOptions::default(),
            Execution::Sequential,
        )
        .unwrap();
        let nested = evaluate_nested(
            &computational,
            &observed,
            &[0, computational.len()],
            gamma,
            SupportOrder::new(1).unwrap(),
            resampling.computational_order,
            comparison_policy,
            AuRocOptimizationOptions::default(),
            Execution::Sequential,
        )
        .unwrap();

        assert!((flat.supported_magnitude - nested.supported_magnitude).abs() < 1e-12);
        assert!((flat.supported_magnitude_lower - nested.supported_magnitude_lower).abs() < 1e-12);
        assert!((flat.supported_magnitude_upper - nested.supported_magnitude_upper).abs() < 1e-12);
        assert!(
            (flat.literal_survival_fraction - nested.literal_survival.subset_fraction).abs()
                < 1e-12
        );
        assert!(
            (flat.literal_survival_lower - nested.literal_survival_lower.subset_fraction).abs()
                < 1e-12
        );
        assert!(
            (flat.literal_survival_upper - nested.literal_survival_upper.subset_fraction).abs()
                < 1e-12
        );
        assert_eq!(flat.verdict, nested.verdict);
    }
}
