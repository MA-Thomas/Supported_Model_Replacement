use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use directed_round_robin_organizer::spec::{
    AurocSpecification, ComputationalDesign, EvidencePolicy, PrCnapSpecification,
};
use directed_round_robin_organizer::{SelectionStrategy, TournamentSpec};
use serde::{Deserialize, Serialize};
use supported_ap::{ReferenceAssessment, SearchOptions, TargetPrevalences};

use crate::io::{read_json, sha256_file};

pub const CONFIG_SCHEMA_VERSION: u32 = 1;
pub const MODELS: [&str; 5] = [
    "full_hla",
    "focal_hla",
    "old_monoallelic",
    "mono_q_full_pn",
    "full_q_mono_pn",
];

/// Roster construction is independent of exhaustive/accelerated execution.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RosterMode {
    #[default]
    RankedUniqueBiologicalKey,
    AllRegimes,
}

impl RosterMode {
    pub fn is_default(&self) -> bool {
        *self == Self::RankedUniqueBiologicalKey
    }

    pub fn rule(self) -> &'static str {
        match self {
            Self::RankedUniqueBiologicalKey => "metric_rank_then_first_unique_d_pos_d_neg_M_N",
            Self::AllRegimes => "all_regimes_in_regime_idx_order",
        }
    }

    pub fn ordering_rule(self) -> &'static str {
        match self {
            Self::RankedUniqueBiologicalKey => "primary_metric_desc_regime_idx_asc",
            Self::AllRegimes => "regime_idx_asc",
        }
    }

    pub fn diversity_key(self) -> &'static [&'static str] {
        match self {
            Self::RankedUniqueBiologicalKey => &["d_pos", "d_neg", "M", "N"],
            Self::AllRegimes => &[],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HashedInput {
    pub path: PathBuf,
    pub sha256: String,
}

impl HashedInput {
    pub fn validate(&self, label: &str) -> Result<()> {
        if !self.path.is_file() {
            bail!("{label} is not a file: {}", self.path.display());
        }
        let actual = sha256_file(&self.path)?;
        if actual != self.sha256 {
            bail!(
                "{label} hash mismatch for {}: expected {}, got {actual}",
                self.path.display(),
                self.sha256
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TensorInputs {
    pub tensor: HashedInput,
    pub observations: HashedInput,
    pub metadata: HashedInput,
}

impl TensorInputs {
    fn validate(&self, label: &str) -> Result<()> {
        self.tensor.validate(&format!("{label}.tensor"))?;
        self.observations
            .validate(&format!("{label}.observations"))?;
        self.metadata.validate(&format!("{label}.metadata"))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    pub display_label: String,
    pub metrics: HashedInput,
    pub primary: TensorInputs,
    #[serde(default)]
    pub query_q: Option<TensorInputs>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SharedTournamentConfig {
    pub evidence_policy: EvidencePolicy,
    pub computational_design: ComputationalDesign,
    pub reference_assessment: ReferenceAssessment,
    pub master_seed: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TournamentConfig {
    pub selection_strategy: SelectionStrategy,
    pub shared: SharedTournamentConfig,
    pub pr: PrCnapSpecification,
    pub roc: AurocSpecification,
    pub operational_prevalences: TargetPrevalences,
    pub operational_search: SearchOptions,
    pub operational_scope_statement: String,
    /// The currently implemented AUROC tertiary rule uses the observed mix.
    #[serde(default = "default_operational_auroc_gamma")]
    pub operational_auroc_gamma: f64,
}

fn default_operational_auroc_gamma() -> f64 {
    1.0
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineConfig {
    pub schema_version: u32,
    pub provider_identity: String,
    #[serde(default, skip_serializing_if = "RosterMode::is_default")]
    pub roster_mode: RosterMode,
    /// Selection limit for the ranked mode; exact expected full-grid count
    /// (never a truncation limit) for all-regimes mode.
    pub candidate_count: usize,
    pub log_epsilon: f64,
    pub models: BTreeMap<String, ModelConfig>,
    pub tournament: TournamentConfig,
}

impl PipelineConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let config = Self::load_structure(path)?;
        config.validate(true)?;
        Ok(config)
    }

    pub fn load_structure(path: &Path) -> Result<Self> {
        let mut config: Self = read_json(path)?;
        let base = path
            .canonicalize()?
            .parent()
            .context("configuration directory")?
            .to_owned();
        for input in config.inputs_mut() {
            if input.path.is_relative() {
                input.path = base.join(&input.path);
            }
        }
        config.validate_structure()?;
        Ok(config)
    }

    pub fn inputs_mut(&mut self) -> Vec<&mut HashedInput> {
        let mut inputs = Vec::new();
        for model in self.models.values_mut() {
            inputs.push(&mut model.metrics);
            inputs.extend([
                &mut model.primary.tensor,
                &mut model.primary.observations,
                &mut model.primary.metadata,
            ]);
            if let Some(q) = &mut model.query_q {
                inputs.extend([&mut q.tensor, &mut q.observations, &mut q.metadata]);
            }
        }
        inputs
    }

    pub fn validate_structure(&self) -> Result<()> {
        self.validate(false)
    }

    fn validate(&self, validate_files: bool) -> Result<()> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            bail!(
                "unsupported config schema {}; expected {CONFIG_SCHEMA_VERSION}",
                self.schema_version
            );
        }
        if self.provider_identity.trim().is_empty() {
            bail!("provider_identity is empty");
        }
        if self.candidate_count < 2 {
            bail!("candidate_count must be at least two");
        }
        if self.tournament.operational_auroc_gamma != 1.0 {
            bail!(
                "operational_auroc_gamma must be 1; case-mix AUROC regret for Gamma > 1 is not implemented"
            );
        }
        if !(self.log_epsilon.is_finite() && self.log_epsilon > 0.0) {
            bail!("log_epsilon must be finite and positive");
        }
        let expected: BTreeSet<_> = MODELS.into_iter().collect();
        let actual: BTreeSet<_> = self.models.keys().map(String::as_str).collect();
        if actual != expected {
            bail!(
                "models must contain exactly {:?}; got {:?}",
                expected,
                actual
            );
        }
        for (model_id, model) in &self.models {
            if model.display_label.trim().is_empty() {
                bail!("model {model_id} has an empty display_label");
            }
            if validate_files {
                model
                    .metrics
                    .validate(&format!("models.{model_id}.metrics"))?;
                model
                    .primary
                    .validate(&format!("models.{model_id}.primary"))?;
                if let Some(query_q) = &model.query_q {
                    query_q.validate(&format!("models.{model_id}.query_q"))?;
                }
            }
        }
        match &self.tournament.operational_prevalences {
            TargetPrevalences::Finite { values } if values.is_empty() => {
                bail!("operational_prevalences cannot be empty")
            }
            TargetPrevalences::ClosedInterval { lower, upper } if lower.get() > upper.get() => {
                bail!("operational prevalence interval is reversed")
            }
            _ => {}
        }
        SearchOptions::new(
            self.tournament.operational_search.grid_points,
            self.tournament.operational_search.tolerance,
            self.tournament.operational_search.max_iterations,
        )?;
        if self
            .tournament
            .operational_scope_statement
            .trim()
            .is_empty()
        {
            bail!("operational_scope_statement is empty")
        }
        for metric in ["pr", "roc"] {
            self.tournament_spec(metric)?.validate()?;
        }
        Ok(())
    }

    pub fn tournament_spec(&self, branch: &str) -> Result<TournamentSpec> {
        let metric = match branch {
            "pr" => directed_round_robin_organizer::MetricKind::PrCnap,
            "roc" => directed_round_robin_organizer::MetricKind::Auroc,
            _ => bail!("unknown metric branch {branch}"),
        };
        Ok(TournamentSpec {
            schema_version: 2,
            metric,
            verdict_field: if branch == "pr" {
                "forward.staged_verdict".into()
            } else {
                "baseline.verdict".into()
            },
            evidence_policy: self.tournament.shared.evidence_policy.clone(),
            computational_design: self.tournament.shared.computational_design.clone(),
            reference_assessment: self.tournament.shared.reference_assessment.clone(),
            master_seed: self.tournament.shared.master_seed,
            seed_derivation_version: 1,
            evaluations: vec!["nci".into()],
            conjunction_rule: "all_evaluations".into(),
            graph_maximality_rule: "source_strongly_connected_components".into(),
            selection_rule: "source_scc_maximal_vertices".into(),
            selection_strategy: self.tournament.selection_strategy,
            pr_cnap: (branch == "pr").then(|| self.tournament.pr.clone()),
            auroc: (branch == "roc").then(|| self.tournament.roc.clone()),
            annotations: BTreeMap::from([
                ("provider_domain".into(), serde_json::json!("IRIS_NCI")),
                (
                    "parameter_roster_rule".into(),
                    serde_json::json!(self.roster_mode.rule()),
                ),
                (
                    "operational_prevalences".into(),
                    serde_json::to_value(&self.tournament.operational_prevalences)
                        .context("serializing operational prevalences")?,
                ),
                (
                    "operational_scope_statement".into(),
                    serde_json::json!(self.tournament.operational_scope_statement),
                ),
            ]),
            operational_tie_break: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empirical_auroc_gamma_defaults_to_one_and_rejects_other_values() {
        let mut value: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../IRIS_scripts/nci_parameter_tournament/config.nci.v1.json"
        ))
        .unwrap();
        value["tournament"]
            .as_object_mut()
            .unwrap()
            .remove("operational_auroc_gamma");
        let mut config: PipelineConfig = serde_json::from_value(value).unwrap();
        assert_eq!(config.tournament.operational_auroc_gamma, 1.0);
        config.validate_structure().unwrap();
        for gamma in [0.5, 1.01, f64::NAN, f64::INFINITY] {
            config.tournament.operational_auroc_gamma = gamma;
            assert!(
                config
                    .validate_structure()
                    .unwrap_err()
                    .to_string()
                    .contains("operational_auroc_gamma")
            );
        }
    }

    #[test]
    fn historical_config_keeps_legacy_roster_and_all_mode_is_explicit() {
        let original: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../IRIS_scripts/nci_parameter_tournament/config.nci.v1.json"
        ))
        .unwrap();
        let old: PipelineConfig = serde_json::from_value(original.clone()).unwrap();
        old.validate_structure().unwrap();
        assert_eq!(old.roster_mode, RosterMode::RankedUniqueBiologicalKey);
        assert_eq!(old.candidate_count, 20);
        assert!(
            serde_json::to_value(&old)
                .unwrap()
                .get("roster_mode")
                .is_none()
        );
        let all: PipelineConfig = serde_json::from_str(include_str!(
            "../../../../IRIS_scripts/nci_parameter_tournament/config.nci.all_regimes.v1.json"
        ))
        .unwrap();
        all.validate_structure().unwrap();
        assert_eq!(all.roster_mode, RosterMode::AllRegimes);
        assert_eq!(all.candidate_count, 4200);
        let mut restored = all.clone();
        restored.roster_mode = RosterMode::default();
        restored.candidate_count = 20;
        // Keep the old config bytes (including historical CNAP wording) bound
        // to completed artifacts; the new config documents metric-specific regret.
        restored.tournament.operational_scope_statement =
            old.tournament.operational_scope_statement.clone();
        assert_eq!(
            serde_json::to_value(restored).unwrap(),
            serde_json::to_value(&old).unwrap()
        );
        for branch in ["pr", "roc"] {
            assert_ne!(
                old.tournament_spec(branch).unwrap().annotations["parameter_roster_rule"],
                all.tournament_spec(branch).unwrap().annotations["parameter_roster_rule"]
            );
        }
        let mut invalid = original;
        invalid["roster_mode"] = serde_json::json!("all_regime_typo");
        assert!(serde_json::from_value::<PipelineConfig>(invalid).is_err());
    }
}
