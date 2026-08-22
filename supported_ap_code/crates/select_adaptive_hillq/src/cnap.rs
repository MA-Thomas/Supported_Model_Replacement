//! Authoritative paired-CNAP selection against the model-matched fixed maximum.
//!
//! The scientific mathematics remains in `supported-ap`. This module only
//! validates and adapts the frozen directed-round-robin PR contract, aligns the
//! fixed maximum score vector to the selector's endpoint order, derives common
//! selection seeds, and extracts compact selection diagnostics.

use std::collections::HashMap;

use directed_round_robin_organizer::identity::seed_from_hash_material;
use directed_round_robin_organizer::input::LoadedBundle;
use directed_round_robin_organizer::spec::{MetricKind, ResamplingUnitSpec};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use supported_ap::{
    ClassCounts, ComputationalReplicationCount, Execution, FiniteEvidenceVerdict,
    MagnitudeThreshold, ObservedApAssessment, PairedEvaluation, ProjectedApAssessment,
    ProjectedResampling, ReplacementPolicy, ResamplingUnit, ScoreTransportAssumption, SupportOrder,
    SurvivalFloor, SurvivalRequirement, assess_observed_ap, assess_staged_projected_ap,
};

use crate::error::{Result, SelectionError};

/// Canonical score-ranking/tie-block hash. Paired CNAP depends on score order
/// and exact ties, not score magnitudes, so identical signatures are exact
/// reusable assessments within the same model/cohort/reference comparison.
pub fn ranking_signature(scores: &[f64]) -> [u8; 32] {
    let mut order: Vec<usize> = (0..scores.len()).collect();
    order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]).then(a.cmp(&b)));
    let mut hash = Sha256::new();
    hash.update(b"select-adaptive-hillq-ranking-v1\0");
    for (position, &index) in order.iter().enumerate() {
        let new_block = position == 0 || scores[index] != scores[order[position - 1]];
        hash.update([u8::from(new_block)]);
        hash.update((index as u64).to_be_bytes());
    }
    hash.finalize().into()
}

pub fn signature_hex(signature: &[u8; 32]) -> String {
    let mut encoded = String::with_capacity(64);
    for byte in signature {
        use std::fmt::Write;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

pub fn parse_signature_hex(value: &str) -> Result<[u8; 32]> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SelectionError::msg(format!(
            "invalid ranking signature: {value}"
        )));
    }
    let mut signature = [0u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let text = std::str::from_utf8(chunk)
            .map_err(|_| SelectionError::msg("ranking signature is not UTF-8"))?;
        signature[index] = u8::from_str_radix(text, 16)
            .map_err(|_| SelectionError::msg(format!("invalid ranking signature: {value}")))?;
    }
    Ok(signature)
}

#[derive(Debug, Clone)]
pub struct CnapContract {
    pub policy: ReplacementPolicy,
    pub observed: ObservedApAssessment,
    pub projected_template: ProjectedTemplate,
    pub master_seed: u64,
    pub seed_derivation_version: u32,
}

#[derive(Debug, Clone)]
pub struct ProjectedTemplate {
    pub target_prevalences: supported_ap::TargetPrevalences,
    pub transport: ScoreTransportAssumption,
    pub search: supported_ap::SearchOptions,
    pub reference_assessment: supported_ap::ReferenceAssessment,
    pub replications: ComputationalReplicationCount,
    pub computational_order: SupportOrder,
    pub replication_positive_count: Option<usize>,
    pub replication_negative_count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CnapOutcome {
    pub observed_retained_difference: f64,
    pub limiting_prevalence: f64,
    pub observed_gate_supported: bool,
    pub staged_supported: bool,
    pub supported_magnitude: Option<f64>,
    pub survival_subset_fraction: Option<f64>,
    pub mean_retained_effect: Option<f64>,
    pub adaptive_empirical_ap: f64,
    pub maximum_empirical_ap: f64,
}

impl CnapOutcome {
    pub fn observed_only(
        observed_retained_difference: f64,
        limiting_prevalence: f64,
        observed_gate_supported: bool,
        adaptive_empirical_ap: f64,
        maximum_empirical_ap: f64,
    ) -> Self {
        Self {
            observed_retained_difference,
            limiting_prevalence,
            observed_gate_supported,
            staged_supported: false,
            supported_magnitude: None,
            survival_subset_fraction: None,
            mean_retained_effect: None,
            adaptive_empirical_ap,
            maximum_empirical_ap,
        }
    }

    /// Scalar used only after the staged verdict supplies eligibility.
    pub fn ranking_magnitude(&self) -> f64 {
        if self.staged_supported {
            self.supported_magnitude.unwrap_or(-1.0e300)
        } else {
            -1.0e300
        }
    }

    pub fn ranking_survival(&self) -> f64 {
        self.survival_subset_fraction.unwrap_or(0.0)
    }
}

fn map_supported_error(error: impl std::fmt::Display) -> SelectionError {
    SelectionError::msg(format!("supported CNAP assessment failed: {error}"))
}

impl CnapContract {
    pub fn from_pr_bundle(bundle: &LoadedBundle) -> Result<Self> {
        let replications = bundle.spec.computational_design.replications;
        Self::from_pr_bundle_with_replications(bundle, replications)
    }

    /// Inherit the complete validated PR contract while declaring a separate
    /// finite replication roster for parameter selection. This deliberately
    /// does not mutate the bundle's downstream tournament specification.
    pub fn from_pr_bundle_with_replications(
        bundle: &LoadedBundle,
        selection_replications: usize,
    ) -> Result<Self> {
        if bundle.spec.metric != MetricKind::PrCnap {
            return Err(SelectionError::msg(
                "adaptive PR selection requires a validated pr_cnap bundle",
            ));
        }
        let pr = bundle
            .spec
            .pr_cnap
            .as_ref()
            .ok_or_else(|| SelectionError::msg("PR bundle lacks pr_cnap specification"))?;
        let evidence = &bundle.spec.evidence_policy;
        let policy = ReplacementPolicy::new(
            MagnitudeThreshold::new(evidence.magnitude_threshold).map_err(map_supported_error)?,
            SurvivalFloor::new(evidence.survival_floor).map_err(map_supported_error)?,
            SurvivalRequirement::new(evidence.survival_requirement).map_err(map_supported_error)?,
        );
        let transport = ScoreTransportAssumption::new(&pr.transport_justification)
            .map_err(map_supported_error)?;
        let observed = ObservedApAssessment {
            target_prevalences: pr.target_prevalences.clone(),
            transport: transport.clone(),
            search: pr.search,
            empirical_order: SupportOrder::new(1).map_err(map_supported_error)?,
            execution: Execution::Sequential,
            reference_assessment: bundle.spec.reference_assessment.clone(),
        };
        let design = &bundle.spec.computational_design;
        if selection_replications < design.computational_order {
            return Err(SelectionError::msg(format!(
                "selection replications ({selection_replications}) must be at least computational order ({})",
                design.computational_order
            )));
        }
        if design.resampling_unit != ResamplingUnitSpec::IndependentObservation {
            return Err(SelectionError::msg(
                "selector supports only the frozen independent-observation CNAP design",
            ));
        }
        Ok(Self {
            policy,
            observed,
            projected_template: ProjectedTemplate {
                target_prevalences: pr.target_prevalences.clone(),
                transport,
                search: pr.search,
                reference_assessment: bundle.spec.reference_assessment.clone(),
                replications: ComputationalReplicationCount::new(selection_replications)
                    .map_err(map_supported_error)?,
                computational_order: SupportOrder::new(design.computational_order)
                    .map_err(map_supported_error)?,
                replication_positive_count: design.replication_positive_count,
                replication_negative_count: design.replication_negative_count,
            },
            master_seed: bundle.spec.master_seed,
            seed_derivation_version: bundle.spec.seed_derivation_version,
        })
    }

    pub fn replications(&self) -> usize {
        self.projected_template.replications.get()
    }

    pub fn computational_order(&self) -> usize {
        self.projected_template.computational_order.get()
    }

    pub fn projected_assessment(
        &self,
        seed: u64,
        observed_counts: ClassCounts,
    ) -> Result<ProjectedApAssessment> {
        let t = &self.projected_template;
        let replication_counts = match (t.replication_positive_count, t.replication_negative_count)
        {
            (Some(positive), Some(negative)) => {
                ClassCounts::new(positive, negative).map_err(map_supported_error)?
            }
            (None, None) => observed_counts,
            _ => {
                return Err(SelectionError::msg(
                    "PR replication class counts must both be specified or both omitted",
                ));
            }
        };
        let resampling = ProjectedResampling::new(
            t.replications,
            t.computational_order,
            replication_counts,
            seed,
            // Grid points are parallelized outside supported-ap. Keeping the
            // inner calculation sequential prevents nested oversubscription.
            Execution::Sequential,
            ResamplingUnit::IndependentObservation,
        )
        .map_err(map_supported_error)?;
        Ok(ProjectedApAssessment {
            target_prevalences: t.target_prevalences.clone(),
            transport: t.transport.clone(),
            search: t.search,
            resampling,
            reference_assessment: t.reference_assessment.clone(),
        })
    }

    pub fn selection_seed(&self, cohort: &str) -> Result<u64> {
        #[derive(Serialize)]
        struct SeedMaterial<'a> {
            domain: &'static str,
            master_seed: u64,
            cohort: &'a str,
            seed_derivation_version: u32,
        }
        seed_from_hash_material(&SeedMaterial {
            domain: "select_adaptive_hillq_paired_cnap_vs_max_v1",
            master_seed: self.master_seed,
            cohort,
            seed_derivation_version: self.seed_derivation_version,
        })
        .map_err(|error| SelectionError::msg(format!("selection seed derivation failed: {error}")))
    }
}

pub fn observed_gate(
    adaptive_scores: &[f64],
    maximum_scores: &[f64],
    labels: &[bool],
    contract: &CnapContract,
) -> Result<(PairedEvaluation, CnapOutcome)> {
    let paired = PairedEvaluation::new(adaptive_scores, maximum_scores, labels)
        .map_err(map_supported_error)?;
    let result = assess_observed_ap(
        std::slice::from_ref(&paired),
        &contract.observed,
        contract.policy,
    )
    .map_err(map_supported_error)?;
    let profile = &result.evaluations[0];
    Ok((
        paired,
        CnapOutcome::observed_only(
            profile.forward.value,
            profile.forward.limiting_prevalence.get(),
            result.forward.verdict == FiniteEvidenceVerdict::SupportedReplacement,
            profile.model_a_empirical_ap,
            profile.model_b_empirical_ap,
        ),
    ))
}

pub fn complete_forward_assessment(
    paired: &PairedEvaluation,
    mut outcome: CnapOutcome,
    contract: &CnapContract,
    seed: u64,
) -> Result<CnapOutcome> {
    if !outcome.observed_gate_supported {
        return Ok(outcome);
    }
    let assessment = contract.projected_assessment(seed, paired.class_counts())?;
    let result = assess_staged_projected_ap(paired, &assessment, contract.policy, false)
        .map_err(map_supported_error)?;
    let forward = &result.forward;
    outcome.staged_supported =
        forward.staged_verdict == FiniteEvidenceVerdict::SupportedReplacement;
    if let Some(full) = &forward.full_assessment {
        outcome.supported_magnitude = Some(full.supported_magnitude);
        outcome.survival_subset_fraction = Some(full.literal_survival.subset_fraction);
        outcome.mean_retained_effect = Some(full.mean_retained_effect);
    }
    Ok(outcome)
}

pub fn aligned_maximum_reference(
    bundle: &LoadedBundle,
    cohort: &str,
    model: &str,
    endpoint_ids: &[String],
    task_labels: &[i8],
) -> Result<(String, Vec<f64>, String)> {
    let evaluation = bundle
        .evaluations
        .get(cohort)
        .ok_or_else(|| SelectionError::msg(format!("PR bundle lacks evaluation {cohort}")))?;
    let system_id = format!("{model}__max");
    let source_scores = evaluation.scores.get(&system_id).ok_or_else(|| {
        SelectionError::msg(format!(
            "PR bundle lacks model-matched reference {system_id}"
        ))
    })?;
    let by_id: HashMap<&str, usize> = evaluation
        .endpoint_ids
        .iter()
        .enumerate()
        .map(|(index, id)| (id.as_str(), index))
        .collect();
    if endpoint_ids.len() != evaluation.endpoint_ids.len()
        || task_labels.len() != endpoint_ids.len()
    {
        return Err(SelectionError::msg(format!(
            "endpoint count differs between selector task and PR bundle for {model}/{cohort}"
        )));
    }
    let mut aligned = Vec::with_capacity(endpoint_ids.len());
    for (position, endpoint_id) in endpoint_ids.iter().enumerate() {
        let index = *by_id.get(endpoint_id.as_str()).ok_or_else(|| {
            SelectionError::msg(format!(
                "selector endpoint {endpoint_id} is absent from PR bundle {cohort}"
            ))
        })?;
        let expected_label = i8::from(evaluation.labels[index]);
        if task_labels[position] != expected_label {
            return Err(SelectionError::msg(format!(
                "selector and PR bundle labels differ at {cohort}/{endpoint_id}"
            )));
        }
        aligned.push(source_scores[index]);
    }
    let score_hash = evaluation
        .score_vector_hashes
        .get(&system_id)
        .cloned()
        .ok_or_else(|| SelectionError::msg(format!("missing score hash for {system_id}")))?;
    Ok((system_id, aligned, score_hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use supported_ap::{Prevalence, ReferenceAssessment, SearchOptions, TargetPrevalences};

    fn tiny_contract() -> CnapContract {
        let target_prevalences = TargetPrevalences::closed_interval(
            Prevalence::new(0.1).unwrap(),
            Prevalence::new(0.3).unwrap(),
        )
        .unwrap();
        let transport = ScoreTransportAssumption::new("test transport").unwrap();
        let reference_assessment = ReferenceAssessment::not_asserted("test limitation").unwrap();
        let search = SearchOptions::new(17, 1e-8, 32).unwrap();
        CnapContract {
            policy: ReplacementPolicy::new(
                MagnitudeThreshold::new(0.0).unwrap(),
                SurvivalFloor::new(0.0).unwrap(),
                SurvivalRequirement::new(0.8).unwrap(),
            ),
            observed: ObservedApAssessment {
                target_prevalences: target_prevalences.clone(),
                transport: transport.clone(),
                search,
                empirical_order: SupportOrder::new(1).unwrap(),
                execution: Execution::Sequential,
                reference_assessment: reference_assessment.clone(),
            },
            projected_template: ProjectedTemplate {
                target_prevalences,
                transport,
                search,
                reference_assessment,
                replications: ComputationalReplicationCount::new(12).unwrap(),
                computational_order: SupportOrder::new(2).unwrap(),
                replication_positive_count: None,
                replication_negative_count: None,
            },
            master_seed: 17,
            seed_derivation_version: 1,
        }
    }

    #[test]
    fn forward_orientation_is_adaptive_over_maximum() {
        let labels = [true, false, true, false];
        let adaptive = [4.0, 1.0, 3.0, 0.0];
        let maximum = [1.0, 4.0, 0.0, 3.0];
        let contract = tiny_contract();
        let (_, outcome) = observed_gate(&adaptive, &maximum, &labels, &contract).unwrap();
        assert!(outcome.observed_retained_difference > 0.0);
        assert!(outcome.observed_gate_supported);
        assert!(outcome.adaptive_empirical_ap > outcome.maximum_empirical_ap);
    }

    #[test]
    fn failed_observed_gate_never_runs_full_stage() {
        let labels = [true, false, true, false];
        let adaptive = [1.0, 4.0, 0.0, 3.0];
        let maximum = [4.0, 1.0, 3.0, 0.0];
        let contract = tiny_contract();
        let (paired, observed) = observed_gate(&adaptive, &maximum, &labels, &contract).unwrap();
        assert!(!observed.observed_gate_supported);
        let completed = complete_forward_assessment(&paired, observed, &contract, 123).unwrap();
        assert!(!completed.staged_supported);
        assert!(completed.supported_magnitude.is_none());
    }
}
