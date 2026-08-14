use serde::{Deserialize, Serialize};
use supported_ap::{
    AuRocBreakdownEstimate, ClassCounts, ComputationalReplicationCount, Execution,
    FiniteEvidenceVerdict, MagnitudeThreshold, PairedEvaluation, PolicyVerdict,
    ProjectedApAssessment, ProjectedAuRocAssessment, ProjectedResampling, ReplacementPolicy,
    ResamplingUnit, ScoreTransportAssumption, SupportOrder, SurvivalFloor, SurvivalRequirement,
    assess_staged_projected_ap, estimate_projected_auroc_breakdown, paired_auroc_difference,
};

use crate::input::LoadedBundle;
use crate::plan::MatchPlan;
use crate::spec::{DirectedVerdict, MetricKind};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "metric", rename_all = "snake_case")]
pub enum JudgeReport {
    PrCnap {
        assessment: ProjectedApAssessment,
        result: Box<supported_ap::StagedProjectedApResult>,
    },
    Auroc {
        low_over_high_assessment: ProjectedAuRocAssessment,
        low_over_high_result: Box<AuRocBreakdownEstimate>,
        high_over_low_assessment: ProjectedAuRocAssessment,
        high_over_low_result: Box<AuRocBreakdownEstimate>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedDirection {
    pub winner: String,
    pub loser: String,
    pub verdict: DirectedVerdict,
    pub official_verdict: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DescriptiveMatchMetrics {
    pub empirical_metric_low: Option<f64>,
    pub empirical_metric_high: Option<f64>,
    pub observed_difference_low_over_high: f64,
    pub observed_difference_high_over_low: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgedMatch {
    pub report: JudgeReport,
    pub low_over_high: NormalizedDirection,
    pub high_over_low: NormalizedDirection,
    pub descriptive: DescriptiveMatchMetrics,
}

pub fn judge_match(bundle: &LoadedBundle, item: &MatchPlan) -> Result<JudgedMatch> {
    let evaluation = bundle
        .evaluations
        .get(&item.evaluation_id)
        .ok_or_else(|| Error::InvalidPlan(format!("unknown evaluation {}", item.evaluation_id)))?;
    let scores_low = evaluation
        .scores
        .get(&item.system_low)
        .ok_or_else(|| Error::InvalidPlan(format!("missing scores for {}", item.system_low)))?;
    let scores_high = evaluation
        .scores
        .get(&item.system_high)
        .ok_or_else(|| Error::InvalidPlan(format!("missing scores for {}", item.system_high)))?;
    let paired = PairedEvaluation::new(scores_low, scores_high, &evaluation.labels)
        .map_err(|error| Error::Judge(error.to_string()))?;
    let policy = replacement_policy(bundle)?;
    let resampling = projected_resampling(bundle, item.seed, paired.class_counts())?;
    match bundle.spec.metric {
        MetricKind::PrCnap => {
            let pr = bundle
                .spec
                .pr_cnap
                .as_ref()
                .ok_or_else(|| Error::InvalidSpec("missing PR/CNAP specification".into()))?;
            let assessment = ProjectedApAssessment {
                target_prevalences: pr.target_prevalences.clone(),
                transport: ScoreTransportAssumption::new(&pr.transport_justification)
                    .map_err(|error| Error::Judge(error.to_string()))?,
                search: pr.search,
                resampling,
                reference_assessment: bundle.spec.reference_assessment.clone(),
            };
            let result = assess_staged_projected_ap(
                &paired,
                &assessment,
                policy,
                pr.retain_replication_profiles,
            )
            .map_err(|error| Error::Judge(error.to_string()))?;
            let low_verdict = normalize_pr(result.forward.staged_verdict);
            let high_verdict = normalize_pr(result.reverse.staged_verdict);
            let descriptive = DescriptiveMatchMetrics {
                empirical_metric_low: Some(result.observed_evaluation.model_a_empirical_ap),
                empirical_metric_high: Some(result.observed_evaluation.model_b_empirical_ap),
                observed_difference_low_over_high: result.observed_evaluation.forward.value,
                observed_difference_high_over_low: result.observed_evaluation.reverse.value,
            };
            Ok(JudgedMatch {
                report: JudgeReport::PrCnap {
                    assessment,
                    result: Box::new(result),
                },
                low_over_high: direction(
                    &item.system_low,
                    &item.system_high,
                    low_verdict,
                    official_pr_verdict(low_verdict).into(),
                ),
                high_over_low: direction(
                    &item.system_high,
                    &item.system_low,
                    high_verdict,
                    official_pr_verdict(high_verdict).into(),
                ),
                descriptive,
            })
        }
        MetricKind::Auroc => {
            let auroc = bundle
                .spec
                .auroc
                .as_ref()
                .ok_or_else(|| Error::InvalidSpec("missing AUROC specification".into()))?;
            let low_assessment = ProjectedAuRocAssessment {
                resampling,
                optimization: auroc.optimization,
                concentration_search: auroc.concentration_search,
                reference_assessment: bundle.spec.reference_assessment.clone(),
            };
            let low_result = estimate_projected_auroc_breakdown(&paired, &low_assessment, policy)
                .map_err(|error| Error::Judge(error.to_string()))?;
            let reversed = PairedEvaluation::new(scores_high, scores_low, &evaluation.labels)
                .map_err(|error| Error::Judge(error.to_string()))?;
            let high_assessment = ProjectedAuRocAssessment {
                resampling,
                optimization: auroc.optimization,
                concentration_search: auroc.concentration_search,
                reference_assessment: bundle.spec.reference_assessment.clone(),
            };
            let high_result =
                estimate_projected_auroc_breakdown(&reversed, &high_assessment, policy)
                    .map_err(|error| Error::Judge(error.to_string()))?;
            let low_verdict = normalize_auroc(low_result.baseline.verdict);
            let high_verdict = normalize_auroc(high_result.baseline.verdict);
            let observed_difference = paired_auroc_difference(&paired);
            let low_official = format!("{:?}", low_result.baseline.verdict);
            let high_official = format!("{:?}", high_result.baseline.verdict);
            Ok(JudgedMatch {
                report: JudgeReport::Auroc {
                    low_over_high_assessment: low_assessment,
                    low_over_high_result: Box::new(low_result),
                    high_over_low_assessment: high_assessment,
                    high_over_low_result: Box::new(high_result),
                },
                low_over_high: direction(
                    &item.system_low,
                    &item.system_high,
                    low_verdict,
                    low_official,
                ),
                high_over_low: direction(
                    &item.system_high,
                    &item.system_low,
                    high_verdict,
                    high_official,
                ),
                descriptive: DescriptiveMatchMetrics {
                    empirical_metric_low: None,
                    empirical_metric_high: None,
                    observed_difference_low_over_high: observed_difference,
                    observed_difference_high_over_low: -observed_difference,
                },
            })
        }
    }
}

fn replacement_policy(bundle: &LoadedBundle) -> Result<ReplacementPolicy> {
    let policy = &bundle.spec.evidence_policy;
    Ok(ReplacementPolicy::new(
        MagnitudeThreshold::new(policy.magnitude_threshold)
            .map_err(|error| Error::Judge(error.to_string()))?,
        SurvivalFloor::new(policy.survival_floor)
            .map_err(|error| Error::Judge(error.to_string()))?,
        SurvivalRequirement::new(policy.survival_requirement)
            .map_err(|error| Error::Judge(error.to_string()))?,
    ))
}

fn projected_resampling(
    bundle: &LoadedBundle,
    seed: u64,
    observed_counts: ClassCounts,
) -> Result<ProjectedResampling> {
    let design = &bundle.spec.computational_design;
    let counts = match (
        design.replication_positive_count,
        design.replication_negative_count,
    ) {
        (Some(positive), Some(negative)) => ClassCounts::new(positive, negative),
        (None, None) => Ok(observed_counts),
        _ => unreachable!("spec validation enforces paired options"),
    }
    .map_err(|error| Error::Judge(error.to_string()))?;
    ProjectedResampling::new(
        ComputationalReplicationCount::new(design.replications)
            .map_err(|error| Error::Judge(error.to_string()))?,
        SupportOrder::new(design.computational_order)
            .map_err(|error| Error::Judge(error.to_string()))?,
        counts,
        seed,
        Execution::Sequential,
        ResamplingUnit::IndependentObservation,
    )
    .map_err(|error| Error::Judge(error.to_string()))
}

fn normalize_pr(verdict: FiniteEvidenceVerdict) -> DirectedVerdict {
    match verdict {
        FiniteEvidenceVerdict::SupportedReplacement => DirectedVerdict::Supported,
        FiniteEvidenceVerdict::NoVerdict => DirectedVerdict::NotSupported,
    }
}

fn official_pr_verdict(verdict: DirectedVerdict) -> &'static str {
    match verdict {
        DirectedVerdict::Supported => "supported_replacement",
        DirectedVerdict::NotSupported | DirectedVerdict::Unresolved => "no_verdict",
    }
}

fn normalize_auroc(verdict: PolicyVerdict) -> DirectedVerdict {
    match verdict {
        PolicyVerdict::VerifiedPass => DirectedVerdict::Supported,
        PolicyVerdict::VerifiedFailure => DirectedVerdict::NotSupported,
        PolicyVerdict::Unresolved => DirectedVerdict::Unresolved,
    }
}

fn direction(
    winner: &str,
    loser: &str,
    verdict: DirectedVerdict,
    official_verdict: String,
) -> NormalizedDirection {
    NormalizedDirection {
        winner: winner.to_owned(),
        loser: loser.to_owned(),
        verdict,
        official_verdict,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use supported_ap::{AuRocOptimizationOptions, ConcentrationSearchOptions, ReferenceAssessment};

    use super::*;
    use crate::identity::{canonical_label_hash, canonical_vector_hash, hash_serializable};
    use crate::input::{
        BUNDLE_SCHEMA_NAME, BUNDLE_SCHEMA_VERSION, BundleManifest, HashedPath, LoadedEvaluation,
    };
    use crate::spec::{
        AurocSpecification, ComputationalDesign, EvidencePolicy, ResamplingUnitSpec,
        SPEC_SCHEMA_VERSION, TournamentSpec,
    };
    use crate::system::{SystemRecord, SystemRegistry};

    #[test]
    fn auroc_match_judges_both_orientations_and_preserves_certificates() {
        let endpoint_ids: Vec<_> = (1..=6).map(|index| format!("e{index}")).collect();
        let labels = vec![true, true, true, false, false, false];
        let low = vec![0.9, 0.8, 0.7, 0.3, 0.2, 0.1];
        let high = vec![0.3, 0.2, 0.1, 0.9, 0.8, 0.7];
        let registry = SystemRegistry {
            schema_version: 2,
            systems: vec![system("a_max", "a"), system("b_max", "b")],
        };
        let spec = TournamentSpec {
            schema_version: SPEC_SCHEMA_VERSION,
            metric: MetricKind::Auroc,
            verdict_field: "baseline.verdict".into(),
            evidence_policy: EvidencePolicy {
                magnitude_threshold: 0.0,
                survival_floor: 0.0,
                survival_requirement: 0.2,
            },
            computational_design: ComputationalDesign {
                replications: 4,
                computational_order: 1,
                replication_positive_count: None,
                replication_negative_count: None,
                resampling_unit: ResamplingUnitSpec::IndependentObservation,
            },
            reference_assessment: ReferenceAssessment::not_asserted("unit test").unwrap(),
            master_seed: 7,
            seed_derivation_version: 1,
            evaluations: vec!["evaluation".into()],
            conjunction_rule: "all_evaluations".into(),
            graph_maximality_rule: "source_strongly_connected_components".into(),
            selection_rule: "source_scc_maximal_vertices".into(),
            pr_cnap: None,
            auroc: Some(AurocSpecification {
                optimization: AuRocOptimizationOptions::default(),
                concentration_search: ConcentrationSearchOptions::new(0.1, 8).unwrap(),
            }),
            annotations: BTreeMap::new(),
            operational_tie_break: None,
        };
        let policy_hash = hash_serializable(&spec).unwrap();
        let evaluation = LoadedEvaluation {
            evaluation_id: "evaluation".into(),
            positive_count: 3,
            negative_count: 3,
            label_vector_hash: canonical_label_hash(&endpoint_ids, &labels),
            score_vector_hashes: BTreeMap::from([
                ("a_max".into(), canonical_vector_hash(&endpoint_ids, &low)),
                ("b_max".into(), canonical_vector_hash(&endpoint_ids, &high)),
            ]),
            scores: BTreeMap::from([("a_max".into(), low), ("b_max".into(), high)]),
            endpoint_ids,
            labels,
            raw_endpoints_hash: "raw-endpoints".into(),
            raw_scores_hash: "raw-scores".into(),
        };
        let item = MatchPlan {
            match_id: "match".into(),
            evaluation_id: "evaluation".into(),
            system_low: "a_max".into(),
            system_high: "b_max".into(),
            score_hash_low: evaluation.score_vector_hashes["a_max"].clone(),
            score_hash_high: evaluation.score_vector_hashes["b_max"].clone(),
            label_vector_hash: evaluation.label_vector_hash.clone(),
            seed: 13,
            shard_id: 0,
        };
        let bundle = LoadedBundle {
            root: PathBuf::new(),
            manifest: BundleManifest {
                schema_name: BUNDLE_SCHEMA_NAME.into(),
                schema_version: BUNDLE_SCHEMA_VERSION,
                metric: MetricKind::Auroc,
                bundle_creation_time: "2026-08-13T00:00:00Z".into(),
                systems: dummy_path(),
                tournament_spec: dummy_path(),
                evaluations: vec![],
                score_provider_identity: "test".into(),
                minimum_organizer_schema_version: BUNDLE_SCHEMA_VERSION,
                bundle_content_hash: "bundle".into(),
            },
            spec,
            registry,
            evaluations: BTreeMap::from([("evaluation".into(), evaluation)]),
            policy_hash,
        };
        let judged = judge_match(&bundle, &item).unwrap();
        assert_eq!(judged.low_over_high.verdict, DirectedVerdict::Supported);
        assert_eq!(judged.high_over_low.verdict, DirectedVerdict::NotSupported);
        assert!(matches!(judged.report, JudgeReport::Auroc { .. }));
    }

    #[test]
    fn all_certified_auroc_states_map_without_reverse_inference() {
        assert_eq!(
            normalize_auroc(PolicyVerdict::VerifiedPass),
            DirectedVerdict::Supported
        );
        assert_eq!(
            normalize_auroc(PolicyVerdict::VerifiedFailure),
            DirectedVerdict::NotSupported
        );
        assert_eq!(
            normalize_auroc(PolicyVerdict::Unresolved),
            DirectedVerdict::Unresolved
        );
    }

    fn system(system_id: &str, display_label: &str) -> SystemRecord {
        SystemRecord {
            system_id: system_id.into(),
            display_label: display_label.into(),
            score_column: format!("score_{system_id}"),
            annotations: BTreeMap::new(),
        }
    }

    fn dummy_path() -> HashedPath {
        HashedPath {
            path: "dummy".into(),
            sha256: "0".repeat(64),
        }
    }
}
