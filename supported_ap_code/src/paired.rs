use serde::{Deserialize, Serialize};

use crate::execution::map_indices;
use crate::metric::{Ranking, tie_averaged_ap, tie_averaged_cnap};
use crate::prevalence::{PrevalenceSearchCertificate, ProfileExtremum, interval_extrema};
use crate::prevalence_profile::PreparedDifference;
use crate::projected::{ProfilePoint, ProjectedResampling};
use crate::random::rng_for;
use crate::resample::{Pools, stratified_multiplicities};
use crate::support::{mean, monte_carlo_standard_error, support};
use crate::{
    AnchoredEffectRow, AnchoredNestedSupport, ClassCounts, ComputationalReplicationCount, Error,
    Execution, FiniteEvidenceVerdict, LiteralSurvival, NestedLiteralSurvival, Prevalence,
    ReplacementPolicy, ResamplingUnit, ScoreTransportAssumption, SearchOptions, SupportOrder,
    TargetPrevalences, anchored_nested_support, literal_survival,
};

const PAIRED_RESAMPLING_DOMAIN: u64 = 0x5041_4952_4544_4254;
const NESTED_PAIRED_RESAMPLING_DOMAIN: u64 = 0x4e45_5354_5041_4952;

#[derive(Debug, Clone)]
pub struct PairedEvaluation {
    pub(crate) scores_a: Vec<f64>,
    pub(crate) scores_b: Vec<f64>,
    pub(crate) labels: Vec<bool>,
    pub(crate) ranking_a: Ranking,
    pub(crate) ranking_b: Ranking,
    pub(crate) pools: Pools,
}

impl PairedEvaluation {
    pub fn new(scores_a: &[f64], scores_b: &[f64], labels: &[bool]) -> Result<Self, Error> {
        if scores_a.len() != labels.len() || scores_b.len() != labels.len() {
            return Err(Error::PairedLengthMismatch);
        }
        Ok(Self {
            scores_a: scores_a.to_vec(),
            scores_b: scores_b.to_vec(),
            labels: labels.to_vec(),
            ranking_a: Ranking::new(scores_a)?,
            ranking_b: Ranking::new(scores_b)?,
            pools: Pools::new(labels)?,
        })
    }

    pub fn class_counts(&self) -> ClassCounts {
        self.pools.counts()
    }

    pub fn tie_averaged_difference(&self, prevalence: Prevalence) -> f64 {
        let multiplicities = vec![1usize; self.labels.len()];
        difference_for_multiplicities(self, &multiplicities, prevalence)
    }

    /// Computes the forward and reverse retained paired effects for an
    /// externally constructed computational evaluation.
    ///
    /// `multiplicities[i]` is the number of times observation `i` appears in
    /// that evaluation. This entry point lets a scientifically declared design
    /// (for example, a cluster bootstrap) construct the computational units
    /// while reusing the crate's exact tie averaging and prevalence search.
    pub fn retained_effects_for_multiplicities(
        &self,
        multiplicities: &[usize],
        target_prevalences: &TargetPrevalences,
        search: SearchOptions,
    ) -> Result<(RetainedEffect, RetainedEffect), Error> {
        if multiplicities.len() != self.labels.len() {
            return Err(Error::PairedLengthMismatch);
        }
        target_prevalences.validate()?;
        search.validate()?;
        let positive = self
            .labels
            .iter()
            .zip(multiplicities)
            .filter_map(|(&label, &count)| label.then_some(count))
            .sum::<usize>();
        let negative = self
            .labels
            .iter()
            .zip(multiplicities)
            .filter_map(|(&label, &count)| (!label).then_some(count))
            .sum::<usize>();
        if positive == 0 || negative == 0 {
            return Err(Error::MissingClass);
        }
        let summary = summarize_retained_paired(self, multiplicities, target_prevalences, search);
        Ok((retained(summary.forward), retained(summary.reverse)))
    }
}

/// Whether the exchangeable-label diagnostic reference law is scientifically
/// asserted. This metadata never controls the finite-evidence verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReferenceAssessment {
    ExchangeableLabels { justification: String },
    NotAsserted { reason: String },
}

impl ReferenceAssessment {
    pub fn exchangeable_labels(justification: impl Into<String>) -> Result<Self, Error> {
        let value = Self::ExchangeableLabels {
            justification: justification.into(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn not_asserted(reason: impl Into<String>) -> Result<Self, Error> {
        let value = Self::NotAsserted {
            reason: reason.into(),
        };
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        match self {
            Self::ExchangeableLabels { justification } if justification.trim().is_empty() => {
                Err(Error::MissingExchangeabilityJustification)
            }
            Self::NotAsserted { reason } if reason.trim().is_empty() => {
                Err(Error::MissingReferenceLimitation)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectedApAssessment {
    pub target_prevalences: TargetPrevalences,
    pub transport: ScoreTransportAssumption,
    pub search: SearchOptions,
    pub resampling: ProjectedResampling,
    pub reference_assessment: ReferenceAssessment,
}

impl ProjectedApAssessment {
    pub fn validate(&self) -> Result<(), Error> {
        self.target_prevalences.validate()?;
        self.transport.validate()?;
        self.search.validate()?;
        self.resampling.validate()?;
        self.reference_assessment.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservedApAssessment {
    pub target_prevalences: TargetPrevalences,
    pub transport: ScoreTransportAssumption,
    pub search: SearchOptions,
    pub empirical_order: SupportOrder,
    pub execution: Execution,
    pub reference_assessment: ReferenceAssessment,
}

impl ObservedApAssessment {
    pub fn validate(&self, evaluation_count: usize) -> Result<(), Error> {
        self.target_prevalences.validate()?;
        self.transport.validate()?;
        self.search.validate()?;
        self.reference_assessment.validate()?;
        if evaluation_count == 0 {
            return Err(Error::EmptyEmpiricalEvaluations);
        }
        if self.empirical_order.get() > evaluation_count {
            return Err(Error::SupportOrderExceedsList {
                order: self.empirical_order.get(),
                list_length: evaluation_count,
            });
        }
        Ok(())
    }
}

/// Computational design for one empirical row in a full nested assessment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NestedRowResampling {
    pub replications: ComputationalReplicationCount,
    pub replication_counts: ClassCounts,
    pub seed: u64,
    pub resampling_unit: ResamplingUnit,
}

/// Assessment specification for several empirical evaluations, each retaining
/// its observed anchor and its own computational effect list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NestedApAssessment {
    pub target_prevalences: TargetPrevalences,
    pub transport: ScoreTransportAssumption,
    pub search: SearchOptions,
    pub empirical_order: SupportOrder,
    pub computational_order: SupportOrder,
    pub rows: Vec<NestedRowResampling>,
    pub execution: Execution,
    pub reference_assessment: ReferenceAssessment,
}

impl NestedApAssessment {
    pub fn validate(&self, evaluation_count: usize) -> Result<(), Error> {
        self.target_prevalences.validate()?;
        self.transport.validate()?;
        self.search.validate()?;
        self.reference_assessment.validate()?;
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
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApEvidence {
    Projected,
    Observed,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RetainedEffect {
    /// Conservative lower bound for interval searches; finite-set value otherwise.
    pub value: f64,
    /// Best sampled witness, not necessarily an argmin of the whole interval.
    pub limiting_prevalence: Prevalence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<PrevalenceSearchCertificate>,
}

impl RetainedEffect {
    fn validate_search(self) -> Result<(), Error> {
        Prevalence::new(self.limiting_prevalence.get())?;
        if let Some(bounds) = self.search {
            if !bounds.lower_bound.is_finite()
                || !bounds.upper_bound.is_finite()
                || !bounds.sampled_value.is_finite()
                || self.value != bounds.lower_bound
                || bounds.lower_bound > bounds.sampled_value
                || bounds.sampled_value > bounds.upper_bound
            {
                return Err(Error::InvalidPrevalenceSearchCertificate);
            }
        }
        Ok(())
    }

    fn upper_bound(self) -> f64 {
        self.search.map_or(self.value, |s| s.upper_bound)
    }

    fn anchored(self, other: Self) -> Self {
        let mut result = if self.value <= other.value {
            self
        } else {
            other
        };
        if let (Some(a), Some(b)) = (self.search, other.search) {
            let witness = if a.sampled_value <= b.sampled_value {
                self
            } else {
                other
            };
            result.limiting_prevalence = witness.limiting_prevalence;
            result.search = Some(PrevalenceSearchCertificate {
                lower_bound: a.lower_bound.min(b.lower_bound),
                upper_bound: a.upper_bound.min(b.upper_bound),
                sampled_value: a.sampled_value.min(b.sampled_value),
                // Both searches contributed; don't disguise anchoring as a new search.
                evaluations: a.evaluations + b.evaluations,
                iterations: a.iterations + b.iterations,
                stop_reason: if a.stop_reason == crate::PrevalenceSearchStop::AccuracyReached {
                    b.stop_reason
                } else {
                    a.stop_reason
                },
            });
        }
        result
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationProfile {
    pub class_counts: ClassCounts,
    pub forward: RetainedEffect,
    pub reverse: RetainedEffect,
    pub model_a_empirical_ap: f64,
    pub model_b_empirical_ap: f64,
    pub model_a_profile: Vec<MarginalProfilePoint>,
    pub model_b_profile: Vec<MarginalProfilePoint>,
    pub diagnostic_difference_profile: Vec<ProfilePoint>,
    pub interval_diagnostics_are_grid_approximations: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MarginalProfilePoint {
    pub prevalence: Prevalence,
    pub ap: f64,
    pub cnap: f64,
}

/// Numerical decision bounds for a continuous prevalence challenge. A budget
/// stop is not evidence of failure: `Unresolved` explicitly distinguishes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PrevalenceDecisionBounds {
    pub supported_magnitude_lower: f64,
    pub supported_magnitude_upper: f64,
    pub literal_survival_lower: f64,
    pub literal_survival_upper: f64,
    pub verdict: crate::PolicyVerdict,
}

impl PrevalenceDecisionBounds {
    fn new(
        magnitude_lower: f64,
        magnitude_upper: f64,
        survival_lower: f64,
        survival_upper: f64,
        policy: ReplacementPolicy,
    ) -> Self {
        let verdict = if magnitude_lower > policy.magnitude_threshold.get()
            && survival_lower > policy.survival_requirement.get()
        {
            crate::PolicyVerdict::VerifiedPass
        } else if magnitude_upper <= policy.magnitude_threshold.get()
            || survival_upper <= policy.survival_requirement.get()
        {
            crate::PolicyVerdict::VerifiedFailure
        } else {
            crate::PolicyVerdict::Unresolved
        };
        Self {
            supported_magnitude_lower: magnitude_lower,
            supported_magnitude_upper: magnitude_upper,
            literal_survival_lower: survival_lower,
            literal_survival_upper: survival_upper,
            verdict,
        }
    }

    fn finite_verdict(self) -> FiniteEvidenceVerdict {
        if self.verdict == crate::PolicyVerdict::VerifiedPass {
            FiniteEvidenceVerdict::SupportedReplacement
        } else {
            FiniteEvidenceVerdict::NoVerdict
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectionalFiniteEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prevalence_search: Option<PrevalenceDecisionBounds>,
    pub supported_magnitude: f64,
    pub mean_retained_effect: f64,
    pub replication_disagreement_cost: f64,
    pub literal_survival: LiteralSurvival,
    pub verdict: FiniteEvidenceVerdict,
    pub monte_carlo_standard_error: Option<f64>,
    pub retained_effects: Vec<RetainedEffect>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairedResamplingTrace {
    pub prevalences: Vec<Prevalence>,
    pub observed_difference_profile: Vec<ProfilePoint>,
    pub replication_difference_profiles: Vec<Vec<f64>>,
    pub forward_retained_effects: Vec<RetainedEffect>,
    pub reverse_retained_effects: Vec<RetainedEffect>,
    pub computational_order: SupportOrder,
    pub computational_replications: crate::ComputationalReplicationCount,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectedApResult {
    pub evidence: ApEvidence,
    pub policy: ReplacementPolicy,
    pub computational_order: SupportOrder,
    pub observed_evaluation: EvaluationProfile,
    pub forward: DirectionalFiniteEvidence,
    pub reverse: DirectionalFiniteEvidence,
    pub diagnostic_range_lower: f64,
    pub diagnostic_range_upper: f64,
    pub reference_assessment: ReferenceAssessment,
    pub mean_diagnostic_difference_profile: Vec<ProfilePoint>,
    pub trace: Option<PairedResamplingTrace>,
}

/// V17 separates the observed empirical gate from the additional
/// within-evaluation computational challenge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StagedDirectionalProjectedAp {
    pub observed_gate: DirectionalFiniteEvidence,
    pub full_assessment: Option<DirectionalFiniteEvidence>,
    pub secondary_unanchored_projection: Option<DirectionalFiniteEvidence>,
    pub staged_verdict: FiniteEvidenceVerdict,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StagedProjectedApResult {
    pub evidence: ApEvidence,
    pub policy: ReplacementPolicy,
    pub empirical_order: SupportOrder,
    pub computational_order: SupportOrder,
    pub observed_evaluation: EvaluationProfile,
    pub forward: StagedDirectionalProjectedAp,
    pub reverse: StagedDirectionalProjectedAp,
    pub reference_assessment: ReferenceAssessment,
    pub mean_diagnostic_difference_profile: Option<Vec<ProfilePoint>>,
    pub trace: Option<PairedResamplingTrace>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservedApResult {
    pub evidence: ApEvidence,
    pub policy: ReplacementPolicy,
    pub empirical_order: SupportOrder,
    pub evaluation_count: usize,
    pub evaluations: Vec<EvaluationProfile>,
    pub forward: DirectionalFiniteEvidence,
    pub reverse: DirectionalFiniteEvidence,
    pub diagnostic_range_lower: f64,
    pub diagnostic_range_upper: f64,
    pub reference_assessment: ReferenceAssessment,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NestedRetainedEffectRow {
    pub observed: RetainedEffect,
    pub computational: Vec<RetainedEffect>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectionalNestedFiniteEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prevalence_search: Option<PrevalenceDecisionBounds>,
    pub supported_magnitude: f64,
    pub literal_survival: NestedLiteralSurvival,
    pub verdict: FiniteEvidenceVerdict,
    pub retained_effect_rows: Vec<NestedRetainedEffectRow>,
}

impl DirectionalNestedFiniteEvidence {
    /// Reduce externally generated computational rows without discarding their
    /// prevalence-search bounds (for example, after a cluster bootstrap).
    pub fn from_retained_effect_rows(
        rows: Vec<NestedRetainedEffectRow>,
        empirical_order: SupportOrder,
        computational_order: SupportOrder,
        policy: ReplacementPolicy,
    ) -> Result<Self, Error> {
        directional_nested_summary(rows, empirical_order, computational_order, policy)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StagedDirectionalNestedAp {
    pub observed_gate: DirectionalFiniteEvidence,
    pub full_assessment: Option<DirectionalNestedFiniteEvidence>,
    pub staged_verdict: FiniteEvidenceVerdict,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StagedNestedApResult {
    pub policy: ReplacementPolicy,
    pub empirical_order: SupportOrder,
    pub computational_order: SupportOrder,
    pub evaluation_count: usize,
    pub evaluations: Vec<EvaluationProfile>,
    pub forward: StagedDirectionalNestedAp,
    pub reverse: StagedDirectionalNestedAp,
    pub reference_assessment: ReferenceAssessment,
}

struct PairedReplicateSummary {
    forward: ProfileExtremum,
    reverse: ProfileExtremum,
    diagnostic_values: Vec<f64>,
}

struct PairedRetainedSummary {
    forward: ProfileExtremum,
    reverse: ProfileExtremum,
}

pub fn assess_projected_ap(
    evaluation: &PairedEvaluation,
    assessment: &ProjectedApAssessment,
    policy: ReplacementPolicy,
    retain_trace: bool,
) -> Result<ProjectedApResult, Error> {
    assessment.validate()?;
    let diagnostic_prevalences = assessment
        .target_prevalences
        .diagnostic_points(assessment.search);
    let observed_multiplicities = vec![1usize; evaluation.labels.len()];
    let observed = summarize_paired(
        evaluation,
        &observed_multiplicities,
        &assessment.target_prevalences,
        assessment.search,
        &diagnostic_prevalences,
    );
    let summaries = map_indices(
        assessment.resampling.replications.get(),
        assessment.resampling.execution,
        |replicate| {
            let mut rng = rng_for(
                assessment.resampling.seed,
                PAIRED_RESAMPLING_DOMAIN,
                &[replicate],
            );
            let mut multiplicities = vec![0usize; evaluation.labels.len()];
            stratified_multiplicities(
                &evaluation.pools,
                assessment.resampling.replication_counts,
                &mut rng,
                &mut multiplicities,
            );
            summarize_paired(
                evaluation,
                &multiplicities,
                &assessment.target_prevalences,
                assessment.search,
                &diagnostic_prevalences,
            )
        },
    );

    let forward_effects = summaries
        .iter()
        .map(|summary| retained(summary.forward))
        .collect::<Vec<_>>();
    let reverse_effects = summaries
        .iter()
        .map(|summary| retained(summary.reverse))
        .collect::<Vec<_>>();
    let forward = directional_summary(
        forward_effects.clone(),
        assessment.resampling.computational_order,
        policy,
        true,
    )?;
    let reverse = directional_summary(
        reverse_effects.clone(),
        assessment.resampling.computational_order,
        policy,
        true,
    )?;
    let mean_profile = mean_profile(&summaries, diagnostic_prevalences.len());
    let observed_profile = evaluation_profile(
        evaluation,
        &observed,
        &diagnostic_prevalences,
        matches!(
            assessment.target_prevalences,
            TargetPrevalences::ClosedInterval { .. }
        ),
    );
    let trace = retain_trace.then(|| PairedResamplingTrace {
        prevalences: diagnostic_prevalences.clone(),
        observed_difference_profile: observed_profile.diagnostic_difference_profile.clone(),
        replication_difference_profiles: summaries
            .iter()
            .map(|summary| summary.diagnostic_values.clone())
            .collect(),
        forward_retained_effects: forward_effects,
        reverse_retained_effects: reverse_effects,
        computational_order: assessment.resampling.computational_order,
        computational_replications: assessment.resampling.replications,
    });

    Ok(ProjectedApResult {
        evidence: ApEvidence::Projected,
        policy,
        computational_order: assessment.resampling.computational_order,
        observed_evaluation: observed_profile,
        diagnostic_range_lower: forward.supported_magnitude,
        diagnostic_range_upper: -reverse.supported_magnitude,
        forward,
        reverse,
        reference_assessment: assessment.reference_assessment.clone(),
        mean_diagnostic_difference_profile: diagnostic_prevalences
            .into_iter()
            .zip(mean_profile)
            .map(|(prevalence, value)| ProfilePoint { prevalence, value })
            .collect(),
        trace,
    })
}

/// V17 staged assessment for one empirical evaluation and its computational
/// replications. The empirical order is necessarily one. Computational
/// replications remain distinct from the observed evaluation: every Stage 2
/// row contribution is clipped by the mandatory observed anchor before support
/// and literal survival are calculated.
pub fn assess_staged_projected_ap(
    evaluation: &PairedEvaluation,
    assessment: &ProjectedApAssessment,
    policy: ReplacementPolicy,
    retain_trace: bool,
) -> Result<StagedProjectedApResult, Error> {
    assessment.validate()?;
    let empirical_order = SupportOrder::new(1).expect("one is a valid support order");
    let diagnostic_prevalences = assessment
        .target_prevalences
        .diagnostic_points(assessment.search);
    let observed = summarize_paired(
        evaluation,
        &vec![1usize; evaluation.labels.len()],
        &assessment.target_prevalences,
        assessment.search,
        &diagnostic_prevalences,
    );
    let observed_evaluation = evaluation_profile(
        evaluation,
        &observed,
        &diagnostic_prevalences,
        matches!(
            assessment.target_prevalences,
            TargetPrevalences::ClosedInterval { .. }
        ),
    );
    let forward_gate = directional_summary(
        vec![observed_evaluation.forward],
        empirical_order,
        policy,
        false,
    )?;
    let reverse_gate = directional_summary(
        vec![observed_evaluation.reverse],
        empirical_order,
        policy,
        false,
    )?;
    let gate_passes = |gate: &DirectionalFiniteEvidence| {
        gate.verdict == FiniteEvidenceVerdict::SupportedReplacement
    };
    if !gate_passes(&forward_gate) && !gate_passes(&reverse_gate) {
        let stopped = |observed_gate| StagedDirectionalProjectedAp {
            observed_gate,
            full_assessment: None,
            secondary_unanchored_projection: None,
            staged_verdict: FiniteEvidenceVerdict::NoVerdict,
        };
        return Ok(StagedProjectedApResult {
            evidence: ApEvidence::Projected,
            policy,
            empirical_order,
            computational_order: assessment.resampling.computational_order,
            forward: stopped(forward_gate),
            reverse: stopped(reverse_gate),
            observed_evaluation,
            reference_assessment: assessment.reference_assessment.clone(),
            mean_diagnostic_difference_profile: None,
            trace: None,
        });
    }

    let projected = assess_projected_ap(evaluation, assessment, policy, retain_trace)?;

    let staged = |observed_gate: DirectionalFiniteEvidence,
                  anchor: RetainedEffect,
                  unanchored: DirectionalFiniteEvidence|
     -> Result<StagedDirectionalProjectedAp, Error> {
        if !gate_passes(&observed_gate) {
            return Ok(StagedDirectionalProjectedAp {
                observed_gate,
                full_assessment: None,
                secondary_unanchored_projection: None,
                staged_verdict: FiniteEvidenceVerdict::NoVerdict,
            });
        }
        let anchored_effects = unanchored
            .retained_effects
            .iter()
            .map(|effect| anchor.anchored(*effect))
            .collect();
        let anchored = directional_summary(
            anchored_effects,
            assessment.resampling.computational_order,
            policy,
            true,
        )?;
        let staged_verdict = anchored.verdict;
        Ok(StagedDirectionalProjectedAp {
            observed_gate,
            full_assessment: Some(anchored),
            secondary_unanchored_projection: Some(unanchored),
            staged_verdict,
        })
    };

    Ok(StagedProjectedApResult {
        evidence: ApEvidence::Projected,
        policy,
        empirical_order,
        computational_order: assessment.resampling.computational_order,
        forward: staged(forward_gate, observed_evaluation.forward, projected.forward)?,
        reverse: staged(reverse_gate, observed_evaluation.reverse, projected.reverse)?,
        observed_evaluation,
        reference_assessment: projected.reference_assessment,
        mean_diagnostic_difference_profile: Some(projected.mean_diagnostic_difference_profile),
        trace: projected.trace,
    })
}

/// Full V17 staged assessment for several empirical evaluations with an
/// independent within-row computational challenge. The observed empirical gate
/// is evaluated before any computational replications are generated.
pub fn assess_staged_nested_ap(
    evaluations: &[PairedEvaluation],
    assessment: &NestedApAssessment,
    policy: ReplacementPolicy,
) -> Result<StagedNestedApResult, Error> {
    assessment.validate(evaluations.len())?;
    let diagnostic_prevalences = assessment
        .target_prevalences
        .diagnostic_points(assessment.search);
    let observed = map_indices(evaluations.len(), assessment.execution, |index| {
        let evaluation = &evaluations[index];
        summarize_paired(
            evaluation,
            &vec![1usize; evaluation.labels.len()],
            &assessment.target_prevalences,
            assessment.search,
            &diagnostic_prevalences,
        )
    });
    let forward_gate = directional_summary(
        observed
            .iter()
            .map(|summary| retained(summary.forward))
            .collect(),
        assessment.empirical_order,
        policy,
        false,
    )?;
    let reverse_gate = directional_summary(
        observed
            .iter()
            .map(|summary| retained(summary.reverse))
            .collect(),
        assessment.empirical_order,
        policy,
        false,
    )?;
    let interval = matches!(
        assessment.target_prevalences,
        TargetPrevalences::ClosedInterval { .. }
    );
    let evaluation_profiles = evaluations
        .iter()
        .zip(&observed)
        .map(|(evaluation, summary)| {
            evaluation_profile(evaluation, summary, &diagnostic_prevalences, interval)
        })
        .collect();
    let gate_passes = |gate: &DirectionalFiniteEvidence| {
        gate.verdict == FiniteEvidenceVerdict::SupportedReplacement
    };
    if !gate_passes(&forward_gate) && !gate_passes(&reverse_gate) {
        let stopped = |observed_gate| StagedDirectionalNestedAp {
            observed_gate,
            full_assessment: None,
            staged_verdict: FiniteEvidenceVerdict::NoVerdict,
        };
        return Ok(StagedNestedApResult {
            policy,
            empirical_order: assessment.empirical_order,
            computational_order: assessment.computational_order,
            evaluation_count: evaluations.len(),
            evaluations: evaluation_profiles,
            forward: stopped(forward_gate),
            reverse: stopped(reverse_gate),
            reference_assessment: assessment.reference_assessment.clone(),
        });
    }

    let mut offsets = Vec::with_capacity(evaluations.len() + 1);
    offsets.push(0usize);
    for design in &assessment.rows {
        offsets.push(offsets.last().copied().unwrap() + design.replications.get());
    }
    let total_replications = *offsets.last().unwrap();
    let computational = map_indices(total_replications, assessment.execution, |task| {
        let row = offsets.partition_point(|&offset| offset <= task) - 1;
        let replicate = task - offsets[row];
        let evaluation = &evaluations[row];
        let design = assessment.rows[row];
        let mut rng = rng_for(
            design.seed,
            NESTED_PAIRED_RESAMPLING_DOMAIN,
            &[row, replicate],
        );
        let mut multiplicities = vec![0usize; evaluation.labels.len()];
        stratified_multiplicities(
            &evaluation.pools,
            design.replication_counts,
            &mut rng,
            &mut multiplicities,
        );
        summarize_retained_paired(
            evaluation,
            &multiplicities,
            &assessment.target_prevalences,
            assessment.search,
        )
    });

    let retained_rows = |forward: bool| {
        observed
            .iter()
            .enumerate()
            .map(|(row, observed_summary)| NestedRetainedEffectRow {
                observed: retained(if forward {
                    observed_summary.forward
                } else {
                    observed_summary.reverse
                }),
                computational: computational[offsets[row]..offsets[row + 1]]
                    .iter()
                    .map(|summary| {
                        retained(if forward {
                            summary.forward
                        } else {
                            summary.reverse
                        })
                    })
                    .collect(),
            })
            .collect::<Vec<_>>()
    };
    let staged = |observed_gate: DirectionalFiniteEvidence,
                  rows: Vec<NestedRetainedEffectRow>|
     -> Result<StagedDirectionalNestedAp, Error> {
        if !gate_passes(&observed_gate) {
            return Ok(StagedDirectionalNestedAp {
                observed_gate,
                full_assessment: None,
                staged_verdict: FiniteEvidenceVerdict::NoVerdict,
            });
        }
        let full = directional_nested_summary(
            rows,
            assessment.empirical_order,
            assessment.computational_order,
            policy,
        )?;
        let staged_verdict = full.verdict;
        Ok(StagedDirectionalNestedAp {
            observed_gate,
            full_assessment: Some(full),
            staged_verdict,
        })
    };

    Ok(StagedNestedApResult {
        policy,
        empirical_order: assessment.empirical_order,
        computational_order: assessment.computational_order,
        evaluation_count: evaluations.len(),
        evaluations: evaluation_profiles,
        forward: staged(forward_gate, retained_rows(true))?,
        reverse: staged(reverse_gate, retained_rows(false))?,
        reference_assessment: assessment.reference_assessment.clone(),
    })
}

pub fn assess_observed_ap(
    evaluations: &[PairedEvaluation],
    assessment: &ObservedApAssessment,
    policy: ReplacementPolicy,
) -> Result<ObservedApResult, Error> {
    assessment.validate(evaluations.len())?;
    let diagnostic_prevalences = assessment
        .target_prevalences
        .diagnostic_points(assessment.search);
    let summaries = map_indices(evaluations.len(), assessment.execution, |index| {
        let evaluation = &evaluations[index];
        summarize_paired(
            evaluation,
            &vec![1usize; evaluation.labels.len()],
            &assessment.target_prevalences,
            assessment.search,
            &diagnostic_prevalences,
        )
    });
    let forward = directional_summary(
        summaries
            .iter()
            .map(|summary| retained(summary.forward))
            .collect(),
        assessment.empirical_order,
        policy,
        false,
    )?;
    let reverse = directional_summary(
        summaries
            .iter()
            .map(|summary| retained(summary.reverse))
            .collect(),
        assessment.empirical_order,
        policy,
        false,
    )?;
    let interval = matches!(
        assessment.target_prevalences,
        TargetPrevalences::ClosedInterval { .. }
    );
    let profiles = evaluations
        .iter()
        .zip(&summaries)
        .map(|(evaluation, summary)| {
            evaluation_profile(evaluation, summary, &diagnostic_prevalences, interval)
        })
        .collect();

    Ok(ObservedApResult {
        evidence: ApEvidence::Observed,
        policy,
        empirical_order: assessment.empirical_order,
        evaluation_count: evaluations.len(),
        evaluations: profiles,
        diagnostic_range_lower: forward.supported_magnitude,
        diagnostic_range_upper: -reverse.supported_magnitude,
        forward,
        reverse,
        reference_assessment: assessment.reference_assessment.clone(),
    })
}

fn directional_summary(
    retained_effects: Vec<RetainedEffect>,
    order: SupportOrder,
    policy: ReplacementPolicy,
    projected: bool,
) -> Result<DirectionalFiniteEvidence, Error> {
    let effects = retained_effects
        .iter()
        .map(|effect| effect.value)
        .collect::<Vec<_>>();
    let mut supported_magnitude = support(&effects, order)?;
    let average = mean(&effects);
    let mut literal_survival = literal_survival(&effects, order, policy.survival_floor)?;
    let prevalence_search = if retained_effects.iter().any(|e| e.search.is_some()) {
        let upper: Vec<_> = retained_effects.iter().map(|e| e.upper_bound()).collect();
        let lower_magnitude = crate::prevalence_evidence::support(&effects, order.get()).lo;
        let upper_magnitude = crate::prevalence_evidence::support(&upper, order.get()).hi;
        let lower_survival = crate::prevalence_evidence::survival(
            literal_survival.survivor_count,
            effects.len(),
            order.get(),
        )
        .lo;
        let upper_survival = crate::prevalence_evidence::survival(
            upper
                .iter()
                .filter(|&&x| x > policy.survival_floor.get())
                .count(),
            upper.len(),
            order.get(),
        )
        .hi;
        supported_magnitude = lower_magnitude;
        literal_survival.subset_fraction = lower_survival;
        Some(PrevalenceDecisionBounds::new(
            lower_magnitude,
            upper_magnitude,
            lower_survival,
            upper_survival,
            policy,
        ))
    } else {
        None
    };
    let verdict = prevalence_search.map_or_else(
        || policy.verdict(supported_magnitude, literal_survival.subset_fraction),
        PrevalenceDecisionBounds::finite_verdict,
    );
    Ok(DirectionalFiniteEvidence {
        prevalence_search,
        supported_magnitude,
        mean_retained_effect: average,
        replication_disagreement_cost: average - supported_magnitude,
        literal_survival,
        verdict,
        monte_carlo_standard_error: projected
            .then(|| monte_carlo_standard_error(&effects, order))
            .transpose()?,
        retained_effects,
    })
}

fn directional_nested_summary(
    retained_effect_rows: Vec<NestedRetainedEffectRow>,
    empirical_order: SupportOrder,
    computational_order: SupportOrder,
    policy: ReplacementPolicy,
) -> Result<DirectionalNestedFiniteEvidence, Error> {
    for row in &retained_effect_rows {
        row.observed.validate_search()?;
        for effect in &row.computational {
            effect.validate_search()?;
        }
    }
    let rows = retained_effect_rows
        .iter()
        .map(|row| AnchoredEffectRow {
            observed_effect: row.observed.value,
            computational_effects: row
                .computational
                .iter()
                .map(|effect| effect.value)
                .collect(),
        })
        .collect::<Vec<_>>();
    let AnchoredNestedSupport {
        mut supported_magnitude,
        mut literal_survival,
    } = anchored_nested_support(
        &rows,
        empirical_order,
        computational_order,
        policy.survival_floor,
    )?;
    let prevalence_search = if retained_effect_rows
        .iter()
        .any(|r| r.observed.search.is_some() || r.computational.iter().any(|e| e.search.is_some()))
    {
        let upper_rows: Vec<_> = retained_effect_rows
            .iter()
            .map(|row| AnchoredEffectRow {
                observed_effect: row.observed.upper_bound(),
                computational_effects: row.computational.iter().map(|e| e.upper_bound()).collect(),
            })
            .collect();
        let (lower_magnitude, lower_survival) = crate::prevalence_evidence::nested(
            &rows,
            empirical_order.get(),
            computational_order.get(),
            policy.survival_floor.get(),
        );
        let (upper_magnitude, upper_survival) = crate::prevalence_evidence::nested(
            &upper_rows,
            empirical_order.get(),
            computational_order.get(),
            policy.survival_floor.get(),
        );
        supported_magnitude = lower_magnitude.lo;
        literal_survival.subset_fraction = lower_survival.lo;
        Some(PrevalenceDecisionBounds::new(
            lower_magnitude.lo,
            upper_magnitude.hi,
            lower_survival.lo,
            upper_survival.hi,
            policy,
        ))
    } else {
        None
    };
    let verdict = prevalence_search.map_or_else(
        || policy.verdict(supported_magnitude, literal_survival.subset_fraction),
        PrevalenceDecisionBounds::finite_verdict,
    );
    Ok(DirectionalNestedFiniteEvidence {
        prevalence_search,
        supported_magnitude,
        literal_survival,
        verdict,
        retained_effect_rows,
    })
}

fn retained(extremum: ProfileExtremum) -> RetainedEffect {
    RetainedEffect {
        value: extremum.value,
        limiting_prevalence: extremum.prevalence,
        search: extremum.search,
    }
}

fn evaluation_profile(
    evaluation: &PairedEvaluation,
    summary: &PairedReplicateSummary,
    diagnostic_prevalences: &[Prevalence],
    interval: bool,
) -> EvaluationProfile {
    let multiplicities = vec![1usize; evaluation.labels.len()];
    let (positive_a, negative_a, counts_a) = evaluation
        .ranking_a
        .block_counts(&evaluation.labels, &multiplicities);
    let (positive_b, negative_b, counts_b) = evaluation
        .ranking_b
        .block_counts(&evaluation.labels, &multiplicities);
    debug_assert_eq!(counts_a, counts_b);
    let empirical_prevalence =
        Prevalence::new(counts_a.positive() as f64 / counts_a.total() as f64)
            .expect("an evaluation with both classes has prevalence in (0,1)");
    let marginal_profile = |positive: &[usize], negative: &[usize]| {
        diagnostic_prevalences
            .iter()
            .copied()
            .map(|prevalence| MarginalProfilePoint {
                prevalence,
                ap: tie_averaged_ap(positive, negative, counts_a, prevalence),
                cnap: tie_averaged_cnap(positive, negative, counts_a, prevalence),
            })
            .collect()
    };
    EvaluationProfile {
        class_counts: counts_a,
        forward: retained(summary.forward),
        reverse: retained(summary.reverse),
        model_a_empirical_ap: tie_averaged_ap(
            &positive_a,
            &negative_a,
            counts_a,
            empirical_prevalence,
        ),
        model_b_empirical_ap: tie_averaged_ap(
            &positive_b,
            &negative_b,
            counts_b,
            empirical_prevalence,
        ),
        model_a_profile: marginal_profile(&positive_a, &negative_a),
        model_b_profile: marginal_profile(&positive_b, &negative_b),
        diagnostic_difference_profile: diagnostic_prevalences
            .iter()
            .copied()
            .zip(summary.diagnostic_values.iter().copied())
            .map(|(prevalence, value)| ProfilePoint { prevalence, value })
            .collect(),
        interval_diagnostics_are_grid_approximations: interval,
    }
}

fn mean_profile(summaries: &[PairedReplicateSummary], width: usize) -> Vec<f64> {
    let mut profile = vec![0.0; width];
    for summary in summaries {
        for (total, value) in profile.iter_mut().zip(&summary.diagnostic_values) {
            *total += value;
        }
    }
    for value in &mut profile {
        *value /= summaries.len() as f64;
    }
    profile
}

fn summarize_paired(
    evaluation: &PairedEvaluation,
    multiplicities: &[usize],
    target_prevalences: &TargetPrevalences,
    search: SearchOptions,
    diagnostic_prevalences: &[Prevalence],
) -> PairedReplicateSummary {
    let (positive_a, negative_a, counts_a) = evaluation
        .ranking_a
        .block_counts(&evaluation.labels, multiplicities);
    let (positive_b, negative_b, counts_b) = evaluation
        .ranking_b
        .block_counts(&evaluation.labels, multiplicities);
    debug_assert_eq!(counts_a, counts_b);
    let (forward, reverse, diagnostic_values) = summarize_counts(
        (&positive_a, &negative_a),
        (&positive_b, &negative_b),
        counts_a,
        target_prevalences,
        search,
    );
    debug_assert_eq!(diagnostic_values.len(), diagnostic_prevalences.len());
    PairedReplicateSummary {
        forward,
        reverse,
        diagnostic_values,
    }
}

fn summarize_retained_paired(
    evaluation: &PairedEvaluation,
    multiplicities: &[usize],
    target_prevalences: &TargetPrevalences,
    search: SearchOptions,
) -> PairedRetainedSummary {
    let (positive_a, negative_a, counts_a) = evaluation
        .ranking_a
        .block_counts(&evaluation.labels, multiplicities);
    let (positive_b, negative_b, counts_b) = evaluation
        .ranking_b
        .block_counts(&evaluation.labels, multiplicities);
    debug_assert_eq!(counts_a, counts_b);
    let (forward, reverse, _) = summarize_counts(
        (&positive_a, &negative_a),
        (&positive_b, &negative_b),
        counts_a,
        target_prevalences,
        search,
    );
    PairedRetainedSummary { forward, reverse }
}

fn summarize_counts(
    a: (&[usize], &[usize]),
    b: (&[usize], &[usize]),
    totals: ClassCounts,
    target: &TargetPrevalences,
    search: SearchOptions,
) -> (ProfileExtremum, ProfileExtremum, Vec<f64>) {
    match target {
        TargetPrevalences::ClosedInterval { .. } => {
            interval_extrema(target, search, &PreparedDifference::new(a, b, totals))
        }
        TargetPrevalences::Finite { values } => {
            // One evaluation supplies both directions and diagnostics. Preserve
            // the existing finite-set numerical kernel and decision contract.
            let diagnostics: Vec<_> = values
                .iter()
                .map(|&pi| {
                    tie_averaged_cnap(a.0, a.1, totals, pi)
                        - tie_averaged_cnap(b.0, b.1, totals, pi)
                })
                .collect();
            let minimum = diagnostics
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.total_cmp(b.1))
                .unwrap()
                .0;
            let maximum = diagnostics
                .iter()
                .enumerate()
                .min_by(|a, b| b.1.total_cmp(a.1))
                .unwrap()
                .0;
            (
                ProfileExtremum {
                    value: diagnostics[minimum],
                    prevalence: values[minimum],
                    search: None,
                },
                ProfileExtremum {
                    value: -diagnostics[maximum],
                    prevalence: values[maximum],
                    search: None,
                },
                diagnostics,
            )
        }
    }
}

fn difference_for_multiplicities(
    evaluation: &PairedEvaluation,
    multiplicities: &[usize],
    prevalence: Prevalence,
) -> f64 {
    let (positive_a, negative_a, counts_a) = evaluation
        .ranking_a
        .block_counts(&evaluation.labels, multiplicities);
    let (positive_b, negative_b, counts_b) = evaluation
        .ranking_b
        .block_counts(&evaluation.labels, multiplicities);
    tie_averaged_cnap(&positive_a, &negative_a, counts_a, prevalence)
        - tie_averaged_cnap(&positive_b, &negative_b, counts_b, prevalence)
}
