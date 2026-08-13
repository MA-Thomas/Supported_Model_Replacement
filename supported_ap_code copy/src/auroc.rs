use std::num::NonZeroU32;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    Error, Execution, NestedRowResampling, PairedEvaluation, ProjectedResampling,
    ReferenceAssessment, SupportOrder,
};

mod capped;
mod gain;
mod optimize;
mod profile;
mod solver;

pub use optimize::{AuRocBackend, OptimizationCertificate, OptimizationStatus};
pub use profile::{
    AuRocBreakdown, AuRocBreakdownEstimate, AuRocEvidence, AuRocProfilePoint, FirstFailure,
    NestedAuRocBreakdownEstimate, NestedAuRocProfilePoint, PolicyVerdict,
    estimate_nested_auroc_breakdown, estimate_observed_auroc_breakdown,
    estimate_projected_auroc_breakdown,
};

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ConcentrationFactor(f64);

impl ConcentrationFactor {
    pub fn new(value: f64) -> Result<Self, Error> {
        if value.is_finite() && value >= 1.0 {
            Ok(Self(value))
        } else {
            Err(Error::InvalidConcentrationFactor(value))
        }
    }

    pub const fn get(self) -> f64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for ConcentrationFactor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(f64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

pub type AuRocPolicy = crate::ReplacementPolicy;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuRocOptimizationOptions {
    pub absolute_gap: f64,
    pub relative_gap: f64,
    pub time_limit_seconds: Option<f64>,
    pub solver_threads: u32,
    /// Which implementation computes the adversarial value.
    #[serde(default)]
    pub backend: AuRocBackend,
    /// Branch-and-bound node ceiling. Exhaustion widens the reported interval
    /// rather than failing, so a tight budget degrades precision, not validity.
    #[serde(default = "default_node_budget")]
    pub node_budget: usize,
    /// Alternating restarts used to build the feasible upper bound.
    #[serde(default = "default_restarts")]
    pub restarts: usize,
}

const fn default_node_budget() -> usize {
    50_000
}

const fn default_restarts() -> usize {
    1
}

impl AuRocOptimizationOptions {
    pub fn new(
        absolute_gap: f64,
        relative_gap: f64,
        time_limit_seconds: Option<f64>,
        solver_threads: u32,
    ) -> Result<Self, Error> {
        let options = Self {
            absolute_gap,
            relative_gap,
            time_limit_seconds,
            solver_threads,
            backend: AuRocBackend::Combinatorial,
            node_budget: default_node_budget(),
            restarts: default_restarts(),
        };
        options.validate()?;
        Ok(options)
    }

    pub(crate) fn validate(self) -> Result<(), Error> {
        if !(self.absolute_gap.is_finite()
            && self.absolute_gap >= 0.0
            && self.relative_gap.is_finite()
            && self.relative_gap >= 0.0)
        {
            return Err(Error::InvalidOptimizationTolerance);
        }
        if self
            .time_limit_seconds
            .is_some_and(|value| !(value.is_finite() && value > 0.0))
        {
            return Err(Error::InvalidOptimizationTimeLimit);
        }
        if self.node_budget == 0 || self.restarts == 0 {
            return Err(Error::InvalidOptimizationBudget);
        }
        NonZeroU32::new(self.solver_threads).ok_or(Error::InvalidOptimizationThreadCount)?;
        if self.backend == AuRocBackend::Combinatorial && self.solver_threads != 1 {
            return Err(Error::UnsupportedCombinatorialThreadCount(
                self.solver_threads,
            ));
        }
        Ok(())
    }
}

impl Default for AuRocOptimizationOptions {
    fn default() -> Self {
        Self {
            absolute_gap: 1e-9,
            relative_gap: 1e-9,
            time_limit_seconds: None,
            solver_threads: 1,
            backend: AuRocBackend::Combinatorial,
            node_budget: default_node_budget(),
            restarts: default_restarts(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConcentrationSearchOptions {
    pub tolerance: f64,
    pub max_iterations: usize,
}

impl ConcentrationSearchOptions {
    pub fn new(tolerance: f64, max_iterations: usize) -> Result<Self, Error> {
        let search = Self {
            tolerance,
            max_iterations,
        };
        search.validate()?;
        Ok(search)
    }

    pub(crate) fn validate(self) -> Result<(), Error> {
        if self.tolerance.is_finite() && self.tolerance > 0.0 && self.max_iterations > 0 {
            Ok(())
        } else {
            Err(Error::InvalidConcentrationSearch)
        }
    }
}

impl Default for ConcentrationSearchOptions {
    fn default() -> Self {
        Self {
            tolerance: 1e-4,
            max_iterations: 64,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedAuRocAssessment {
    pub resampling: ProjectedResampling,
    pub optimization: AuRocOptimizationOptions,
    pub concentration_search: ConcentrationSearchOptions,
    pub reference_assessment: ReferenceAssessment,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedAuRocAssessment {
    pub empirical_order: SupportOrder,
    pub optimization: AuRocOptimizationOptions,
    pub concentration_search: ConcentrationSearchOptions,
    pub execution: Execution,
    pub reference_assessment: ReferenceAssessment,
}

/// Full nested AUROC assessment for several empirical evaluations, each with
/// its own observed anchor and computational resampling design.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NestedAuRocAssessment {
    pub empirical_order: SupportOrder,
    pub computational_order: SupportOrder,
    pub rows: Vec<NestedRowResampling>,
    pub optimization: AuRocOptimizationOptions,
    pub concentration_search: ConcentrationSearchOptions,
    pub execution: Execution,
    pub reference_assessment: ReferenceAssessment,
}

impl NestedAuRocAssessment {
    pub(crate) fn validate(&self, evaluation_count: usize) -> Result<(), Error> {
        if evaluation_count == 0 {
            return Err(Error::EmptyEmpiricalEvaluations);
        }
        if self.rows.len() != evaluation_count {
            return Err(Error::NestedRowDesignCountMismatch {
                evaluations: evaluation_count,
                designs: self.rows.len(),
            });
        }
        if self.empirical_order.get() > evaluation_count {
            return Err(Error::SupportOrderExceedsList {
                order: self.empirical_order.get(),
                list_length: evaluation_count,
            });
        }
        for (row, design) in self.rows.iter().enumerate() {
            if self.computational_order.get() > design.replications.get() {
                return Err(Error::ComputationalOrderExceedsRow {
                    row,
                    order: self.computational_order.get(),
                    list_length: design.replications.get(),
                });
            }
        }
        self.optimization.validate()?;
        self.concentration_search.validate()?;
        self.reference_assessment.validate()
    }
}

pub fn paired_auroc_difference(evaluation: &PairedEvaluation) -> f64 {
    let matrix = gain::GainMatrix::new(evaluation);
    let multiplicities = matrix.identity_multiplicities();
    gain::Marginals::new(&matrix, &multiplicities).uniform_difference(&multiplicities)
}

pub fn worst_case_auroc_difference(
    evaluation: &PairedEvaluation,
    gamma: ConcentrationFactor,
    options: AuRocOptimizationOptions,
) -> Result<OptimizationCertificate, Error> {
    options.validate()?;
    let matrix = gain::GainMatrix::new(evaluation);
    let multiplicities = matrix.identity_multiplicities();
    let marginals = gain::Marginals::new(&matrix, &multiplicities);
    let mut scratch = solver::SolverScratch::default();
    optimize::minimize(
        &matrix,
        &multiplicities,
        &marginals,
        gamma,
        options,
        None,
        &mut scratch,
    )
}
