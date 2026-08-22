use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::identity::{
    canonical_label_hash, canonical_vector_hash, hash_serializable, sha256_file,
};
use crate::spec::{MetricKind, TournamentSpec, TournamentSpecIdentity};
use crate::system::SystemRegistry;
use crate::{Error, Result};

pub const BUNDLE_SCHEMA_NAME: &str = "directed_round_robin_input_bundle";
pub const BUNDLE_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashedPath {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationManifest {
    pub evaluation_id: String,
    pub endpoints: HashedPath,
    pub scores: HashedPath,
    pub source_provenance: HashedPath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    pub schema_name: String,
    pub schema_version: u32,
    pub metric: MetricKind,
    pub bundle_creation_time: String,
    pub systems: HashedPath,
    pub tournament_spec: HashedPath,
    pub evaluations: Vec<EvaluationManifest>,
    pub score_provider_identity: String,
    pub minimum_organizer_schema_version: u32,
    pub bundle_content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableEvaluationHashes {
    pub evaluation_id: String,
    pub label_vector_hash: String,
    pub score_vector_hashes: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct PortableBundleIdentity<'a> {
    schema_name: &'static str,
    schema_version: u32,
    metric: MetricKind,
    tournament_spec: TournamentSpecIdentity<'a>,
    systems: Vec<&'a crate::system::SystemRecord>,
    evaluations: Vec<&'a PortableEvaluationHashes>,
}

pub fn compute_bundle_content_hash(
    spec: &TournamentSpec,
    registry: &SystemRegistry,
    evaluations: &[PortableEvaluationHashes],
) -> Result<String> {
    let mut systems: Vec<_> = registry.systems.iter().collect();
    systems.sort_by(|left, right| left.system_id.cmp(&right.system_id));
    let mut evaluations: Vec<_> = evaluations.iter().collect();
    evaluations.sort_by(|left, right| left.evaluation_id.cmp(&right.evaluation_id));
    hash_serializable(&PortableBundleIdentity {
        schema_name: BUNDLE_SCHEMA_NAME,
        schema_version: BUNDLE_SCHEMA_VERSION,
        metric: spec.metric,
        tournament_spec: spec.identity_view(),
        systems,
        evaluations,
    })
}

#[derive(Debug, Clone)]
pub struct LoadedEvaluation {
    pub evaluation_id: String,
    pub endpoint_ids: Vec<String>,
    pub labels: Vec<bool>,
    pub positive_count: usize,
    pub negative_count: usize,
    pub label_vector_hash: String,
    pub scores: BTreeMap<String, Vec<f64>>,
    pub score_vector_hashes: BTreeMap<String, String>,
    pub raw_endpoints_hash: String,
    pub raw_scores_hash: String,
}

#[derive(Debug, Clone)]
pub struct LoadedBundle {
    pub root: PathBuf,
    pub manifest: BundleManifest,
    pub spec: TournamentSpec,
    pub registry: SystemRegistry,
    pub evaluations: BTreeMap<String, LoadedEvaluation>,
    pub policy_hash: String,
}

impl LoadedBundle {
    pub fn summary(&self) -> BundleSummary {
        BundleSummary {
            bundle_content_hash: self.manifest.bundle_content_hash.clone(),
            metric: self.spec.metric,
            evaluation_count: self.evaluations.len(),
            system_count: self.registry.systems.len(),
            expected_match_count: self.evaluations.len()
                * self.registry.systems.len()
                * (self.registry.systems.len() - 1)
                / 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleSummary {
    pub bundle_content_hash: String,
    pub metric: MetricKind,
    pub evaluation_count: usize,
    pub system_count: usize,
    pub expected_match_count: usize,
}

pub fn load_bundle(root: &Path) -> Result<LoadedBundle> {
    load_bundle_internal(root, true)
}

/// Loads and validates all bundle content but does not require the declared portable content hash
/// to match. This is intended only for a score provider finalizing a new manifest.
pub fn load_bundle_for_hashing(root: &Path) -> Result<LoadedBundle> {
    load_bundle_internal(root, false)
}

fn load_bundle_internal(root: &Path, enforce_content_hash: bool) -> Result<LoadedBundle> {
    let root = root
        .canonicalize()
        .map_err(|error| crate::error::io(root, error))?;
    if !root.is_dir() {
        return Err(Error::InvalidBundle(format!(
            "bundle root {} is not a directory",
            root.display()
        )));
    }
    let manifest_path = root.join("bundle_manifest.json");
    let manifest: BundleManifest = read_json(&manifest_path)?;
    if manifest.schema_name != BUNDLE_SCHEMA_NAME
        || manifest.schema_version != BUNDLE_SCHEMA_VERSION
        || manifest.minimum_organizer_schema_version > BUNDLE_SCHEMA_VERSION
    {
        return Err(Error::InvalidBundle(
            "unsupported bundle schema name or version".into(),
        ));
    }
    if manifest.score_provider_identity.trim().is_empty() {
        return Err(Error::InvalidBundle(
            "score_provider_identity is empty".into(),
        ));
    }
    if manifest.bundle_creation_time.trim().is_empty() {
        return Err(Error::InvalidBundle("bundle_creation_time is empty".into()));
    }
    verify_hashed_path(&root, &manifest.systems)?;
    verify_hashed_path(&root, &manifest.tournament_spec)?;
    let systems_path = checked_bundle_path(&root, &manifest.systems.path)?;
    let spec_path = checked_bundle_path(&root, &manifest.tournament_spec.path)?;
    let registry: SystemRegistry = read_json(&systems_path)?;
    let spec: TournamentSpec = read_json(&spec_path)?;
    spec.validate()?;
    if manifest.metric != spec.metric {
        return Err(Error::InvalidBundle(
            "bundle manifest metric differs from tournament specification".into(),
        ));
    }
    registry.validate(&spec)?;

    if manifest.evaluations.len() != spec.evaluations.len() {
        return Err(Error::InvalidBundle(
            "manifest and specification evaluation counts differ".into(),
        ));
    }
    let manifest_ids: BTreeSet<_> = manifest
        .evaluations
        .iter()
        .map(|evaluation| evaluation.evaluation_id.as_str())
        .collect();
    let spec_ids: BTreeSet<_> = spec.evaluations.iter().map(String::as_str).collect();
    if manifest_ids != spec_ids || manifest_ids.len() != manifest.evaluations.len() {
        return Err(Error::InvalidBundle(
            "manifest evaluation registry differs from the frozen specification".into(),
        ));
    }

    let mut evaluations = BTreeMap::new();
    for entry in &manifest.evaluations {
        verify_hashed_path(&root, &entry.endpoints)?;
        verify_hashed_path(&root, &entry.scores)?;
        verify_hashed_path(&root, &entry.source_provenance)?;
        let provenance_path = checked_bundle_path(&root, &entry.source_provenance.path)?;
        let provenance: serde_json::Value = read_json(&provenance_path)?;
        if !provenance.is_object() {
            return Err(Error::InvalidBundle(format!(
                "{} source provenance must be a JSON object",
                entry.evaluation_id
            )));
        }
        let evaluation = load_evaluation(&root, entry, &registry)?;
        if evaluations
            .insert(entry.evaluation_id.clone(), evaluation)
            .is_some()
        {
            return Err(Error::InvalidBundle(format!(
                "duplicate evaluation {}",
                entry.evaluation_id
            )));
        }
    }
    let portable_evaluations: Vec<_> = evaluations
        .values()
        .map(|evaluation| PortableEvaluationHashes {
            evaluation_id: evaluation.evaluation_id.clone(),
            label_vector_hash: evaluation.label_vector_hash.clone(),
            score_vector_hashes: evaluation.score_vector_hashes.clone(),
        })
        .collect();
    let computed_bundle_hash =
        compute_bundle_content_hash(&spec, &registry, &portable_evaluations)?;
    if enforce_content_hash && computed_bundle_hash != manifest.bundle_content_hash {
        return Err(Error::InvalidBundle(format!(
            "portable bundle content hash mismatch: declared {}, computed {}",
            manifest.bundle_content_hash, computed_bundle_hash
        )));
    }
    let policy_hash = hash_serializable(&spec.identity_view())?;
    let mut manifest = manifest;
    if !enforce_content_hash {
        manifest.bundle_content_hash = computed_bundle_hash;
    }
    Ok(LoadedBundle {
        root,
        manifest,
        spec,
        registry,
        evaluations,
        policy_hash,
    })
}

fn load_evaluation(
    root: &Path,
    entry: &EvaluationManifest,
    registry: &SystemRegistry,
) -> Result<LoadedEvaluation> {
    let endpoints_path = checked_bundle_path(root, &entry.endpoints.path)?;
    let scores_path = checked_bundle_path(root, &entry.scores.path)?;
    let (endpoint_ids, labels) = read_endpoints(&endpoints_path)?;
    let positive_count = labels.iter().filter(|&&label| label).count();
    let negative_count = labels.len() - positive_count;
    if positive_count == 0 || negative_count == 0 {
        return Err(Error::InvalidBundle(format!(
            "evaluation {} must contain both label classes",
            entry.evaluation_id
        )));
    }
    let scores = read_scores(&scores_path, &endpoint_ids, registry)?;
    let score_vector_hashes = scores
        .iter()
        .map(|(system_id, vector)| {
            (
                system_id.clone(),
                canonical_vector_hash(&endpoint_ids, vector),
            )
        })
        .collect();
    Ok(LoadedEvaluation {
        evaluation_id: entry.evaluation_id.clone(),
        label_vector_hash: canonical_label_hash(&endpoint_ids, &labels),
        endpoint_ids,
        labels,
        positive_count,
        negative_count,
        scores,
        score_vector_hashes,
        raw_endpoints_hash: entry.endpoints.sha256.clone(),
        raw_scores_hash: entry.scores.sha256.clone(),
    })
}

fn read_endpoints(path: &Path) -> Result<(Vec<String>, Vec<bool>)> {
    let mut reader = csv::Reader::from_path(path).map_err(|source| Error::Csv {
        path: path.to_owned(),
        source,
    })?;
    let headers = reader
        .headers()
        .map_err(|source| Error::Csv {
            path: path.to_owned(),
            source,
        })?
        .clone();
    let endpoint_index = column_index(path, &headers, "endpoint_id")?;
    let label_index = column_index(path, &headers, "label")?;
    let mut ids = Vec::new();
    let mut labels = Vec::new();
    let mut unique = BTreeSet::new();
    for (index, record) in reader.records().enumerate() {
        let record = record.map_err(|source| Error::Csv {
            path: path.to_owned(),
            source,
        })?;
        let endpoint = record[endpoint_index].trim().to_owned();
        if endpoint.is_empty() || !unique.insert(endpoint.clone()) {
            return Err(Error::InvalidBundle(format!(
                "{} row {} has an empty or duplicate endpoint_id",
                path.display(),
                index + 2
            )));
        }
        let label = match record[label_index].trim() {
            "1" | "true" | "TRUE" => true,
            "0" | "false" | "FALSE" => false,
            other => {
                return Err(Error::InvalidBundle(format!(
                    "{} row {} has invalid binary label {other:?}",
                    path.display(),
                    index + 2
                )));
            }
        };
        ids.push(endpoint);
        labels.push(label);
    }
    if ids.is_empty() {
        return Err(Error::InvalidBundle(format!(
            "{} contains no endpoints",
            path.display()
        )));
    }
    let mut keyed: Vec<_> = ids.into_iter().zip(labels).collect();
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(keyed.into_iter().unzip())
}

fn read_scores(
    path: &Path,
    endpoint_ids: &[String],
    registry: &SystemRegistry,
) -> Result<BTreeMap<String, Vec<f64>>> {
    let mut reader = csv::Reader::from_path(path).map_err(|source| Error::Csv {
        path: path.to_owned(),
        source,
    })?;
    let headers = reader
        .headers()
        .map_err(|source| Error::Csv {
            path: path.to_owned(),
            source,
        })?
        .clone();
    let endpoint_index = column_index(path, &headers, "endpoint_id")?;
    let systems = registry.sorted_systems();
    let score_indices = systems
        .iter()
        .map(|system| column_index(path, &headers, &system.score_column))
        .collect::<Result<Vec<_>>>()?;
    let expected_columns: BTreeSet<_> = systems
        .iter()
        .map(|system| system.score_column.as_str())
        .collect();
    let observed_score_columns: BTreeSet<_> = headers
        .iter()
        .filter(|header| *header != "endpoint_id")
        .collect();
    if observed_score_columns != expected_columns {
        return Err(Error::InvalidBundle(format!(
            "{} score columns do not exactly match the system registry",
            path.display()
        )));
    }
    let mut rows: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for (row_index, record) in reader.records().enumerate() {
        let record = record.map_err(|source| Error::Csv {
            path: path.to_owned(),
            source,
        })?;
        let endpoint = record[endpoint_index].trim().to_owned();
        if endpoint.is_empty() || rows.contains_key(&endpoint) {
            return Err(Error::InvalidBundle(format!(
                "{} row {} has an empty or duplicate endpoint_id",
                path.display(),
                row_index + 2
            )));
        }
        let mut values = Vec::with_capacity(systems.len());
        for (system, &column_index) in systems.iter().zip(&score_indices) {
            let raw = record[column_index].trim();
            let value = raw.parse::<f64>().map_err(|_| {
                Error::InvalidBundle(format!(
                    "{} row {} column {} is not a score: {raw:?}",
                    path.display(),
                    row_index + 2,
                    system.score_column
                ))
            })?;
            if !value.is_finite() {
                return Err(Error::InvalidBundle(format!(
                    "{} row {} column {} is nonfinite",
                    path.display(),
                    row_index + 2,
                    system.score_column
                )));
            }
            values.push(value);
        }
        rows.insert(endpoint, values);
    }
    let expected: BTreeSet<_> = endpoint_ids.iter().cloned().collect();
    let observed: BTreeSet<_> = rows.keys().cloned().collect();
    if expected != observed {
        return Err(Error::InvalidBundle(format!(
            "{} endpoint roster differs from endpoints.csv",
            path.display()
        )));
    }
    let mut result: BTreeMap<String, Vec<f64>> = systems
        .iter()
        .map(|system| {
            (
                system.system_id.clone(),
                Vec::with_capacity(endpoint_ids.len()),
            )
        })
        .collect();
    for endpoint in endpoint_ids {
        let values = rows.get(endpoint).expect("roster equality was established");
        for (system, value) in systems.iter().zip(values) {
            result
                .get_mut(&system.system_id)
                .expect("system initialized")
                .push(*value);
        }
    }
    Ok(result)
}

fn column_index(path: &Path, headers: &csv::StringRecord, column: &str) -> Result<usize> {
    headers
        .iter()
        .position(|header| header == column)
        .ok_or_else(|| {
            Error::InvalidBundle(format!("{} is missing column {column}", path.display()))
        })
}

fn verify_hashed_path(root: &Path, declared: &HashedPath) -> Result<()> {
    if declared.sha256.len() != 64
        || !declared
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(Error::InvalidBundle(format!(
            "invalid lowercase SHA-256 for {}",
            declared.path.display()
        )));
    }
    let path = checked_bundle_path(root, &declared.path)?;
    let actual = sha256_file(&path)?;
    if actual != declared.sha256 {
        return Err(Error::InvalidBundle(format!(
            "SHA-256 mismatch for {}: declared {}, computed {}",
            declared.path.display(),
            declared.sha256,
            actual
        )));
    }
    Ok(())
}

fn checked_bundle_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(Error::InvalidBundle(format!(
            "unsafe bundle-relative path {}",
            relative.display()
        )));
    }
    let joined = root.join(relative);
    let canonical = joined
        .canonicalize()
        .map_err(|error| crate::error::io(&joined, error))?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(Error::InvalidBundle(format!(
            "bundle path {} escapes the root or is not a regular file",
            relative.display()
        )));
    }
    Ok(canonical)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file = File::open(path).map_err(|error| crate::error::io(path, error))?;
    serde_json::from_reader(file).map_err(|source| Error::Json {
        path: path.to_owned(),
        source,
    })
}
