use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{InputError, Result};
use crate::io::read_json;

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuildConfig {
    pub schema_version: u32,
    pub datasets: Vec<DatasetConfig>,
    #[serde(default)]
    pub provenance: Vec<ProvenanceInput>,
    #[serde(default)]
    pub passthrough_files: Vec<PassthroughFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetConfig {
    pub id: String,
    pub prefix: String,
    pub query: PathBuf,
    pub env_dict: PathBuf,
    pub mapping: PathBuf,
    pub config_template: PathBuf,
    pub generated_mono_config: String,
    pub response_column: String,
    pub response_kind: ResponseKind,
    pub endpoint_fields: Vec<String>,
    #[serde(default)]
    pub generated_full_deduplicated_config: Option<String>,
    #[serde(default)]
    pub noise_ceiling_threshold: Option<f64>,
    #[serde(default)]
    pub expected: Option<ExpectedCounts>,
    #[serde(default)]
    pub expected_source_sha256: Option<ExpectedSourceHashes>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResponseKind {
    Binary,
    Continuous,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedCounts {
    #[serde(default)]
    pub unique_query_rows: Option<usize>,
    #[serde(default)]
    pub mono_environments: Option<usize>,
    #[serde(default)]
    pub mapping_rows: Option<usize>,
    #[serde(default)]
    pub scoreable_rows: Option<usize>,
    #[serde(default)]
    pub floor_rows: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedSourceHashes {
    pub query: String,
    pub env_dict: String,
    pub mapping: String,
    pub config_template: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceInput {
    pub role: String,
    pub path: PathBuf,
    #[serde(default)]
    pub copy_to_output: bool,
    #[serde(default)]
    pub output_name: Option<String>,
    #[serde(default)]
    pub expected_sha256: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PassthroughFile {
    pub path: PathBuf,
    #[serde(default)]
    pub output_name: Option<String>,
    #[serde(default)]
    pub expected_sha256: Option<String>,
}

impl BuildConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let config: Self = read_json(path)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(InputError::contract(format!(
                "unsupported configuration schema_version {}; expected {}",
                self.schema_version, CONFIG_SCHEMA_VERSION
            )));
        }
        if self.datasets.is_empty() {
            return Err(InputError::contract("configuration has no datasets"));
        }
        let mut ids = BTreeSet::new();
        let mut prefixes = BTreeSet::new();
        for dataset in &self.datasets {
            if dataset.id.trim().is_empty() || dataset.prefix.trim().is_empty() {
                return Err(InputError::contract(
                    "dataset id and prefix must be nonempty",
                ));
            }
            if !ids.insert(dataset.id.clone()) {
                return Err(InputError::contract(format!(
                    "duplicate dataset id {:?}",
                    dataset.id
                )));
            }
            if !prefixes.insert(dataset.prefix.clone()) {
                return Err(InputError::contract(format!(
                    "duplicate dataset prefix {:?}",
                    dataset.prefix
                )));
            }
            if dataset.endpoint_fields.is_empty() {
                return Err(InputError::contract(format!(
                    "dataset {} has no endpoint_fields",
                    dataset.id
                )));
            }
            if dataset.response_kind == ResponseKind::Binary
                && dataset.noise_ceiling_threshold.is_some()
            {
                return Err(InputError::contract(format!(
                    "binary dataset {} cannot define noise_ceiling_threshold",
                    dataset.id
                )));
            }
        }
        Ok(())
    }
}

pub fn resolve_path(input_root: &Path, configured: &Path) -> PathBuf {
    if configured.is_absolute() {
        configured.to_path_buf()
    } else {
        input_root.join(configured)
    }
}
