use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use supported_ap::{
    AuRocBackend, AuRocOptimizationOptions, ConcentrationSearchOptions, ReferenceAssessment,
    SearchOptions, TargetPrevalences,
};

use crate::{Error, Result};

pub const SPEC_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    PrCnap,
    Auroc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectedVerdict {
    Supported,
    NotSupported,
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidencePolicy {
    pub magnitude_threshold: f64,
    pub survival_floor: f64,
    pub survival_requirement: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResamplingUnitSpec {
    IndependentObservation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputationalDesign {
    pub replications: usize,
    pub computational_order: usize,
    pub replication_positive_count: Option<usize>,
    pub replication_negative_count: Option<usize>,
    pub resampling_unit: ResamplingUnitSpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrCnapSpecification {
    pub target_prevalences: TargetPrevalences,
    pub search: SearchOptions,
    pub transport_justification: String,
    #[serde(default)]
    pub retain_replication_profiles: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AurocSpecification {
    pub optimization: AuRocOptimizationOptions,
    pub concentration_search: ConcentrationSearchOptions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TournamentSpec {
    pub schema_version: u32,
    pub metric: MetricKind,
    pub verdict_field: String,
    pub evidence_policy: EvidencePolicy,
    pub computational_design: ComputationalDesign,
    pub reference_assessment: ReferenceAssessment,
    pub master_seed: u64,
    pub seed_derivation_version: u32,
    pub evaluations: Vec<String>,
    pub conjunction_rule: String,
    pub graph_maximality_rule: String,
    pub selection_rule: String,
    pub pr_cnap: Option<PrCnapSpecification>,
    pub auroc: Option<AurocSpecification>,
    #[serde(default)]
    pub annotations: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub operational_tie_break: Option<Vec<String>>,
}

impl TournamentSpec {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SPEC_SCHEMA_VERSION {
            return Err(Error::InvalidSpec(format!(
                "unsupported schema version {}",
                self.schema_version
            )));
        }
        let expected_field = match self.metric {
            MetricKind::PrCnap => "forward.staged_verdict",
            MetricKind::Auroc => "baseline.verdict",
        };
        if self.verdict_field != expected_field {
            return Err(Error::InvalidSpec(format!(
                "metric {:?} requires verdict_field {expected_field:?}",
                self.metric
            )));
        }
        let policy = &self.evidence_policy;
        if !(policy.magnitude_threshold.is_finite() && policy.magnitude_threshold >= 0.0) {
            return Err(Error::InvalidSpec("invalid magnitude threshold".into()));
        }
        if !(policy.survival_floor.is_finite() && policy.survival_floor >= 0.0) {
            return Err(Error::InvalidSpec("invalid survival floor".into()));
        }
        if !(policy.survival_requirement.is_finite()
            && policy.survival_requirement > 0.0
            && policy.survival_requirement < 1.0)
        {
            return Err(Error::InvalidSpec("invalid survival requirement".into()));
        }
        if self.computational_design.replications == 0
            || self.computational_design.computational_order == 0
            || self.computational_design.computational_order
                > self.computational_design.replications
        {
            return Err(Error::InvalidSpec(
                "invalid computational replication design".into(),
            ));
        }
        match (
            self.computational_design.replication_positive_count,
            self.computational_design.replication_negative_count,
        ) {
            (Some(positive), Some(negative)) if positive > 0 && negative > 0 => {}
            (None, None) => {}
            _ => {
                return Err(Error::InvalidSpec(
                    "replication class counts must both be positive or both omitted".into(),
                ));
            }
        }
        if self.seed_derivation_version == 0 {
            return Err(Error::InvalidSpec(
                "seed_derivation_version must be positive".into(),
            ));
        }
        ensure_nonempty_unique("evaluation", &self.evaluations)?;
        if self.conjunction_rule != "all_evaluations" {
            return Err(Error::InvalidSpec(
                "conjunction_rule must be all_evaluations".into(),
            ));
        }
        if self.graph_maximality_rule != "source_strongly_connected_components" {
            return Err(Error::InvalidSpec(
                "graph_maximality_rule must be source_strongly_connected_components".into(),
            ));
        }
        if self.selection_rule != "source_scc_maximal_vertices" {
            return Err(Error::InvalidSpec("unsupported selection rule".into()));
        }
        for key in self.annotations.keys() {
            validate_id("annotation key", key)?;
        }
        match self.metric {
            MetricKind::PrCnap if self.pr_cnap.is_some() && self.auroc.is_none() => {
                let pr = self.pr_cnap.as_ref().expect("checked");
                if pr.transport_justification.trim().is_empty() {
                    return Err(Error::InvalidSpec(
                        "PR transport justification is empty".into(),
                    ));
                }
                match &pr.target_prevalences {
                    TargetPrevalences::Finite { values } if values.is_empty() => {
                        return Err(Error::InvalidSpec(
                            "PR target prevalence set is empty".into(),
                        ));
                    }
                    TargetPrevalences::ClosedInterval { lower, upper }
                        if lower.get() > upper.get() =>
                    {
                        return Err(Error::InvalidSpec(
                            "PR target prevalence interval is reversed".into(),
                        ));
                    }
                    _ => {}
                }
                if pr.search.grid_points < 3
                    || !(pr.search.tolerance.is_finite() && pr.search.tolerance > 0.0)
                    || pr.search.max_iterations == 0
                {
                    return Err(Error::InvalidSpec("invalid PR search options".into()));
                }
            }
            MetricKind::Auroc if self.auroc.is_some() && self.pr_cnap.is_none() => {
                let auroc = self.auroc.as_ref().expect("checked");
                let optimization = auroc.optimization;
                if !(optimization.absolute_gap.is_finite()
                    && optimization.absolute_gap >= 0.0
                    && optimization.relative_gap.is_finite()
                    && optimization.relative_gap >= 0.0)
                    || optimization
                        .time_limit_seconds
                        .is_some_and(|value| !(value.is_finite() && value > 0.0))
                    || optimization.solver_threads == 0
                    || optimization.node_budget == 0
                    || optimization.restarts == 0
                    || (optimization.backend == AuRocBackend::Combinatorial
                        && optimization.solver_threads != 1)
                    || !(auroc.concentration_search.tolerance.is_finite()
                        && auroc.concentration_search.tolerance > 0.0)
                    || auroc.concentration_search.max_iterations == 0
                {
                    return Err(Error::InvalidSpec(
                        "invalid AUROC optimization or concentration-search options".into(),
                    ));
                }
            }
            _ => {
                return Err(Error::InvalidSpec(
                    "exactly the matching metric-specific specification is required".into(),
                ));
            }
        }
        match &self.reference_assessment {
            ReferenceAssessment::ExchangeableLabels { justification }
                if justification.trim().is_empty() =>
            {
                return Err(Error::InvalidSpec(
                    "exchangeable-label justification is empty".into(),
                ));
            }
            ReferenceAssessment::NotAsserted { reason } if reason.trim().is_empty() => {
                return Err(Error::InvalidSpec(
                    "reference-assessment limitation is empty".into(),
                ));
            }
            _ => {}
        }
        Ok(())
    }
}

pub(crate) fn validate_id(field: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(Error::InvalidSpec(format!(
            "{field} {value:?} is not a stable machine identifier"
        )));
    }
    Ok(())
}

fn ensure_nonempty_unique(label: &str, values: &[String]) -> Result<()> {
    if values.is_empty() {
        return Err(Error::InvalidSpec(format!("{label} list is empty")));
    }
    let mut unique = BTreeSet::new();
    for value in values {
        validate_id(label, value)?;
        if !unique.insert(value) {
            return Err(Error::InvalidSpec(format!("duplicate {label} {value}")));
        }
    }
    Ok(())
}
