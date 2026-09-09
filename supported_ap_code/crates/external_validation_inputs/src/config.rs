use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{InputError, Result};
use crate::io::read_json;

pub const CONFIG_SCHEMA_VERSION: u32 = 2;

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
    pub expression: String,
    pub env_dict: PathBuf,
    pub primary_view_id: String,
    pub primary_mapping: PathBuf,
    pub config_template: PathBuf,
    pub generated_mono_config: String,
    pub generated_full_deduplicated_config: String,
    pub response_column: String,
    pub response_kind: ResponseKind,
    pub endpoint_fields: Vec<String>,
    #[serde(default)]
    pub views: Vec<ViewConfig>,
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
    pub full_query_rows: Option<usize>,
    #[serde(default)]
    pub mono_environments: Option<usize>,
    #[serde(default)]
    pub primary_source_rows: Option<usize>,
    #[serde(default)]
    pub primary_mapping_rows: Option<usize>,
    #[serde(default)]
    pub primary_endpoints: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedSourceHashes {
    pub env_dict: String,
    pub primary_mapping: String,
    pub config_template: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ViewConfig {
    pub id: String,
    pub role: ViewRole,
    pub mapping: PathBuf,
    pub require_subset_of: String,
    #[serde(default)]
    pub selection_eligible: bool,
    #[serde(default)]
    pub bundle_eligible: bool,
    #[serde(default)]
    pub expected: Option<ExpectedViewCounts>,
    #[serde(default)]
    pub expected_sha256: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViewRole {
    Secondary,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedViewCounts {
    #[serde(default)]
    pub source_rows: Option<usize>,
    #[serde(default)]
    pub mapping_rows: Option<usize>,
    #[serde(default)]
    pub endpoints: Option<usize>,
    #[serde(default)]
    pub compute_keys: Option<usize>,
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
            if dataset.id.trim().is_empty()
                || dataset.prefix.trim().is_empty()
                || dataset.expression.trim().is_empty()
                || dataset.primary_view_id.trim().is_empty()
            {
                return Err(InputError::contract(
                    "dataset id, prefix, expression, and primary_view_id must be nonempty",
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
            let mut view_ids = BTreeSet::from([dataset.primary_view_id.clone()]);
            for view in &dataset.views {
                if view.id.trim().is_empty() {
                    return Err(InputError::contract(format!(
                        "dataset {} has a blank view id",
                        dataset.id
                    )));
                }
                if !view_ids.insert(view.id.clone()) {
                    return Err(InputError::contract(format!(
                        "dataset {} has duplicate view id {:?}",
                        dataset.id, view.id
                    )));
                }
                if view.require_subset_of != dataset.primary_view_id {
                    return Err(InputError::contract(format!(
                        "dataset {} secondary view {} must require_subset_of primary view {}",
                        dataset.id, view.id, dataset.primary_view_id
                    )));
                }
                if view.selection_eligible || view.bundle_eligible {
                    return Err(InputError::contract(format!(
                        "dataset {} secondary view {} cannot be selection- or bundle-eligible in schema v2",
                        dataset.id, view.id
                    )));
                }
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
