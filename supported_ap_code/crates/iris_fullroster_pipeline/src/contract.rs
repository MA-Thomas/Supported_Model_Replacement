use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use directed_round_robin_organizer::spec::{
    AurocSpecification, ComputationalDesign, EvidencePolicy, PrCnapSpecification,
};
use serde::{Deserialize, Serialize};

use crate::io::{read_json, sha256_file};

pub const CONFIG_SCHEMA_VERSION: u32 = 1;
pub const TRANSFER_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const LOCKED_LOG_EPSILON: f64 = 1e-12;
pub const MODELS: [&str; 5] = [
    "full_hla",
    "focal_hla",
    "old_monoallelic",
    "mono_q_full_pn",
    "full_q_mono_pn",
];
pub const COHORTS: [&str; 3] = ["pdac", "covid_spike", "covid_nonspike"];
pub const BRANCHES: [&str; 2] = ["pr", "roc"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PipelineConfig {
    pub schema_version: u32,
    pub provider_identity: String,
    pub input_bundle: PathBuf,
    pub input_manifest_sha256: String,
    pub log_epsilon: f64,
    pub models: BTreeMap<String, ModelConfig>,
    pub cohorts: BTreeMap<String, CohortConfig>,
    pub fixed_l2: Vec<L2Spec>,
    pub tournament: TournamentConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfig {
    pub display_label: String,
    pub nci_summary: PathBuf,
    pub nci_summary_sha256: String,
    pub primary: Representation,
    #[serde(default)]
    pub query_q: Option<Representation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CohortConfig {
    pub dataset: Dataset,
    pub input_prefix: String,
    pub tensors: BTreeMap<Representation, TensorRun>,
    pub endpoint_fields: Vec<String>,
    #[serde(default)]
    pub response_field: Option<String>,
    #[serde(default)]
    pub label_field: Option<String>,
    #[serde(default)]
    pub threshold: Option<f64>,
    #[serde(default)]
    pub comparison_operator: Option<ComparisonOperator>,
    pub measurement_error_policy: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TensorRun {
    pub directory: PathBuf,
    pub tensor_sha256: String,
    pub observations_sha256: String,
    pub metadata_sha256: String,
}

impl TensorRun {
    pub fn tensor(&self) -> PathBuf {
        self.directory.join("f_tensor.parquet")
    }

    pub fn observations(&self) -> PathBuf {
        self.directory.join("f_tensor.observations.parquet")
    }

    pub fn metadata(&self) -> PathBuf {
        self.directory.join("f_tensor.metadata.json")
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Representation {
    Full,
    Focal,
    Mono,
}

impl Representation {
    pub fn mapping_kind(self) -> &'static str {
        match self {
            Self::Mono => "mono",
            Self::Full | Self::Focal => "full",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Dataset {
    Pdac,
    CovidSpike,
    CovidNonspike,
}

impl Dataset {
    pub fn transfer_name(self) -> &'static str {
        match self {
            Self::Pdac => "PDAC",
            Self::CovidSpike => "COVID_SPIKE",
            Self::CovidNonspike => "COVID_NONSPIKE",
        }
    }
}

impl ComparisonOperator {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::GreaterThan => ">",
            Self::GreaterThanOrEqual => ">=",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonOperator {
    GreaterThan,
    GreaterThanOrEqual,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct L2Spec {
    pub method: L2Method,
    #[serde(default)]
    pub k: Option<usize>,
    #[serde(default)]
    pub fraction: Option<f64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum L2Method {
    Max,
    Mean,
    Median,
    Logsumexp,
    Logmeanexp,
    TopKMean,
    TopFractionMean,
    TopKLogsumexp,
}

impl L2Spec {
    pub fn id(&self) -> String {
        let base = match self.method {
            L2Method::Max => "max",
            L2Method::Mean => "mean",
            L2Method::Median => "median",
            L2Method::Logsumexp => "logsumexp",
            L2Method::Logmeanexp => "logmeanexp",
            L2Method::TopKMean => "top_k_mean",
            L2Method::TopFractionMean => "top_frac_mean",
            L2Method::TopKLogsumexp => "top_k_logsumexp",
        };
        if let Some(k) = self.k {
            format!("{base}_k{k}")
        } else if let Some(fraction) = self.fraction {
            let token = format!("{fraction}").replace('.', "p");
            format!("{base}_frac{token}")
        } else {
            base.to_owned()
        }
    }

    pub fn validate(&self) -> Result<()> {
        match self.method {
            L2Method::TopKMean | L2Method::TopKLogsumexp => {
                if self.k.is_none_or(|k| k == 0) || self.fraction.is_some() {
                    bail!("{} requires one positive k and no fraction", self.id());
                }
            }
            L2Method::TopFractionMean => {
                if self.k.is_some()
                    || self
                        .fraction
                        .is_none_or(|f| !f.is_finite() || f <= 0.0 || f > 1.0)
                {
                    bail!("{} requires one fraction in (0,1] and no k", self.id());
                }
            }
            _ if self.k.is_some() || self.fraction.is_some() => {
                bail!("{} accepts neither k nor fraction", self.id());
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TournamentConfig {
    pub accepted_measurement_error: serde_json::Value,
    pub covid_spike_label_specification: serde_json::Value,
    pub shared: SharedTournamentPolicy,
    pub pr: PrCnapSpecification,
    pub roc: AurocSpecification,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SharedTournamentPolicy {
    pub evidence_policy: EvidencePolicy,
    pub computational_design: ComputationalDesign,
    pub reference_assessment: supported_ap::ReferenceAssessment,
    pub master_seed: u64,
}

impl PipelineConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let config = Self::load_provenance(path)?;
        config.validate_source_files()?;
        Ok(config)
    }

    pub(crate) fn load_provenance(path: &Path) -> Result<Self> {
        let mut config: Self = read_json(path)?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        absolutize(base, &mut config.input_bundle);
        for model in config.models.values_mut() {
            absolutize(base, &mut model.nci_summary);
        }
        for cohort in config.cohorts.values_mut() {
            for tensor in cohort.tensors.values_mut() {
                absolutize(base, &mut tensor.directory);
            }
        }
        config.validate_portable_contract()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_portable_contract()?;
        self.validate_source_files()
    }

    fn validate_portable_contract(&self) -> Result<()> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            bail!(
                "unsupported pipeline schema_version {}",
                self.schema_version
            );
        }
        if self.provider_identity.trim().is_empty() {
            bail!("provider_identity is empty");
        }
        if self.log_epsilon.to_bits() != LOCKED_LOG_EPSILON.to_bits() {
            bail!("log_epsilon must be exactly {LOCKED_LOG_EPSILON}");
        }
        require_exact_keys("models", self.models.keys(), MODELS)?;
        require_exact_keys3("cohorts", self.cohorts.keys(), COHORTS)?;
        for (model_id, model) in &self.models {
            if model.display_label.trim().is_empty() {
                bail!("model {model_id} has an empty display label");
            }
            let expected_q = match model_id.as_str() {
                "mono_q_full_pn" => Some(Representation::Mono),
                "full_q_mono_pn" => Some(Representation::Full),
                _ => None,
            };
            if model.query_q != expected_q {
                bail!("model {model_id} has the wrong external-Q representation");
            }
            let expected_primary = match model_id.as_str() {
                "full_hla" | "mono_q_full_pn" => Representation::Full,
                "focal_hla" => Representation::Focal,
                "old_monoallelic" | "full_q_mono_pn" => Representation::Mono,
                _ => unreachable!("exact model registry was validated"),
            };
            if model.primary != expected_primary {
                bail!("model {model_id} has the wrong primary P/N representation");
            }
        }
        for (cohort_id, cohort) in &self.cohorts {
            if cohort.input_prefix.trim().is_empty()
                || cohort.endpoint_fields.is_empty()
                || cohort.measurement_error_policy.trim().is_empty()
            {
                bail!("cohort {cohort_id} has an incomplete endpoint contract");
            }
            if cohort.response_field.is_some() == cohort.label_field.is_some() {
                bail!("cohort {cohort_id} needs exactly one response_field or label_field");
            }
            if cohort.response_field.is_some()
                && (cohort.threshold.is_none() || cohort.comparison_operator.is_none())
            {
                bail!("cohort {cohort_id} response contract is incomplete");
            }
            if cohort.response_field.as_deref().is_some_and(str::is_empty)
                || cohort.label_field.as_deref().is_some_and(str::is_empty)
                || cohort
                    .threshold
                    .is_some_and(|threshold| !threshold.is_finite())
            {
                bail!("cohort {cohort_id} has an invalid label field or threshold");
            }
            if cohort.label_field.is_some()
                && (cohort.threshold.is_some() || cohort.comparison_operator.is_some())
            {
                bail!("cohort {cohort_id} label contract must not declare a threshold");
            }
            let (expected_dataset, expected_fields): (Dataset, &[&str]) = match cohort_id.as_str() {
                "pdac" => (Dataset::Pdac, &["patient_id", "long_peptide"]),
                "covid_spike" => (
                    Dataset::CovidSpike,
                    &["patient_id", "mutation", "long_peptide"],
                ),
                "covid_nonspike" => (Dataset::CovidNonspike, &["patient_id", "long_peptide"]),
                _ => unreachable!("exact cohort registry was validated"),
            };
            if cohort.dataset != expected_dataset
                || cohort
                    .endpoint_fields
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    != expected_fields
            {
                bail!("cohort {cohort_id} has the wrong dataset or endpoint analysis unit");
            }
            for representation in [
                Representation::Full,
                Representation::Focal,
                Representation::Mono,
            ] {
                cohort.tensors.get(&representation).with_context(|| {
                    format!("cohort {cohort_id} lacks {representation:?} tensor run")
                })?;
            }
        }
        if self.fixed_l2.len() != 12 {
            bail!("fixed_l2 must contain exactly twelve explicit operators");
        }
        let mut ids = BTreeSet::new();
        for spec in &self.fixed_l2 {
            spec.validate()?;
            if !ids.insert(spec.id()) {
                bail!("duplicate fixed L2 operator {}", spec.id());
            }
        }
        let expected: BTreeSet<String> = [
            "max",
            "mean",
            "median",
            "logsumexp",
            "logmeanexp",
            "top_frac_mean_frac0p01",
            "top_frac_mean_frac0p02",
            "top_frac_mean_frac0p05",
            "top_k_mean_k2",
            "top_k_mean_k3",
            "top_k_logsumexp_k2",
            "top_k_logsumexp_k10",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        if ids != expected {
            bail!("fixed_l2 does not match the frozen twelve-operator roster");
        }
        let spike = &self.tournament.covid_spike_label_specification;
        let spike_cohort = &self.cohorts["covid_spike"];
        if spike["id"].as_str() != Some("threshold_zero")
            || spike["response_field"].as_str() != spike_cohort.response_field.as_deref()
            || spike["threshold"].as_f64().map(f64::to_bits)
                != spike_cohort.threshold.map(f64::to_bits)
            || spike["comparison_operator"].as_str()
                != spike_cohort
                    .comparison_operator
                    .map(ComparisonOperator::symbol)
        {
            bail!("COVID SPIKE label specification disagrees with the cohort label contract");
        }
        Ok(())
    }

    fn validate_source_files(&self) -> Result<()> {
        if sha256_file(&self.input_bundle.join("manifest.json"))? != self.input_manifest_sha256 {
            bail!("input package manifest hash differs from the frozen configuration");
        }
        for (model_id, model) in &self.models {
            verify_hash(&model.nci_summary, &model.nci_summary_sha256)
                .with_context(|| format!("NCI summary for {model_id}"))?;
        }
        for cohort in self.cohorts.values() {
            for representation in [
                Representation::Full,
                Representation::Focal,
                Representation::Mono,
            ] {
                let run = &cohort.tensors[&representation];
                verify_hash(&run.tensor(), &run.tensor_sha256)?;
                verify_hash(&run.observations(), &run.observations_sha256)?;
                verify_hash(&run.metadata(), &run.metadata_sha256)?;
            }
        }
        Ok(())
    }

    pub fn evaluation_label_contracts(&self) -> serde_json::Value {
        serde_json::Value::Object(
            self.cohorts
                .iter()
                .map(|(cohort_id, cohort)| {
                    (
                        cohort_id.clone(),
                        serde_json::json!({
                            "endpoint_fields": cohort.endpoint_fields,
                            "response_field": cohort.response_field,
                            "label_field": cohort.label_field,
                            "threshold": cohort.threshold,
                            "comparison_operator": cohort.comparison_operator.map(ComparisonOperator::symbol),
                            "measurement_error_policy": cohort.measurement_error_policy,
                        }),
                    )
                })
                .collect(),
        )
    }
}

fn absolutize(base: &Path, path: &mut PathBuf) {
    if path.is_relative() {
        *path = base.join(&*path);
    }
}

fn verify_hash(path: &Path, expected: &str) -> Result<()> {
    if !path.is_file() {
        bail!("missing file {}", path.display());
    }
    let observed = sha256_file(path)?;
    if observed != expected {
        bail!(
            "SHA-256 mismatch for {}: expected {expected}, observed {observed}",
            path.display()
        );
    }
    Ok(())
}

fn require_exact_keys<'a>(
    label: &str,
    observed: impl Iterator<Item = &'a String>,
    expected: [&str; 5],
) -> Result<()> {
    let observed: BTreeSet<&str> = observed.map(String::as_str).collect();
    let expected: BTreeSet<&str> = expected.into_iter().collect();
    if observed != expected {
        bail!("{label} registry differs: observed={observed:?}, expected={expected:?}");
    }
    Ok(())
}

fn require_exact_keys3<'a>(
    label: &str,
    observed: impl Iterator<Item = &'a String>,
    expected: [&str; 3],
) -> Result<()> {
    let observed: BTreeSet<&str> = observed.map(String::as_str).collect();
    let expected: BTreeSet<&str> = expected.into_iter().collect();
    if observed != expected {
        bail!("{label} registry differs: observed={observed:?}, expected={expected:?}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_template_has_the_complete_typed_contract() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../IRIS_scripts/iris_fullroster_pipeline.example.json");
        let config: PipelineConfig = serde_json::from_reader(std::fs::File::open(path).unwrap())
            .expect("production template must deserialize through the Rust contract");
        assert_eq!(config.models.len(), 5);
        assert_eq!(config.cohorts.len(), 3);
        assert_eq!(config.fixed_l2.len(), 12);
        assert_eq!(config.log_epsilon.to_bits(), LOCKED_LOG_EPSILON.to_bits());
        let contracts = config.evaluation_label_contracts();
        assert_eq!(
            contracts["covid_nonspike"]["comparison_operator"],
            serde_json::json!(">")
        );
        assert_eq!(
            contracts["covid_spike"]["endpoint_fields"],
            serde_json::json!(["patient_id", "mutation", "long_peptide"])
        );
    }
}
