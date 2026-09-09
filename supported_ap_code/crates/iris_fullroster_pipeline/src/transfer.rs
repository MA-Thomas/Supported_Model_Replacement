use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use arrow::array::{Array, Float32Array, StringArray, UInt8Array, UInt32Array};
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde::Serialize;
use serde_json::{Value, json};

use crate::contract::{
    AdaptiveL2Spec, BRANCHES, COHORTS, Dataset, L2Spec, LOCKED_LOG_EPSILON, MODELS, PipelineConfig,
    Representation, TRANSFER_MANIFEST_SCHEMA_VERSION,
};
use crate::hybrid::aggregate_frozen_adaptive_l2;
use crate::io::{FileRecord, output_file_records, read_json, sha256_file, stage_path, write_json};
use crate::numeric::{aggregate, average_precision, log_score, roc_auc};

#[derive(Debug, Clone)]
struct Observation {
    obs_idx: usize,
    peptide: String,
    hla: String,
    env_id: u32,
    patient_id: String,
    label: u8,
    gene: String,
    cancer_type: String,
    wt_mt_group_id: String,
}

#[derive(Debug, Clone)]
struct Metadata {
    n_params: usize,
    n_mn: usize,
    n_tau: usize,
    geometry_params: Vec<[f64; 4]>,
    tau_values: Vec<f64>,
    mn_tuples: Vec<[u32; 2]>,
}

#[derive(Debug, Clone, Serialize)]
struct DecodedRegime {
    regime_idx: usize,
    param_idx: usize,
    mn_idx: usize,
    geometry_idx: usize,
    tau_idx: usize,
    d_pos: f64,
    d_neg: f64,
    steepness_pos: f64,
    steepness_neg: f64,
    tau_thymus: Option<f64>,
    #[serde(rename = "M")]
    m: u32,
    #[serde(rename = "N")]
    n: u32,
}

#[derive(Debug, Clone)]
struct SelectedRegime {
    selector: String,
    nci_regime_idx: usize,
    decoded: DecodedRegime,
}

#[derive(Debug, Clone)]
struct MappingRow {
    patient_id: String,
    env_id: u32,
    mutation: String,
    long_peptide: String,
    nmer: String,
    hla: String,
    status: String,
    label: u8,
}

#[derive(Debug, Clone)]
struct Endpoint {
    patient_id: String,
    mutation: String,
    long_peptide: String,
    label: u8,
}

#[derive(Debug, Clone)]
struct CandidateRef {
    obs_idx: usize,
    peptide: String,
    hla: String,
}

#[derive(Debug)]
struct QOverride {
    values: Vec<f32>,
    source_obs_indices: Vec<usize>,
    source_env_ids: Vec<u32>,
}

#[derive(Debug, Default, Serialize)]
struct Coverage {
    n_mapping_rows_read: usize,
    n_candidate_rows_eligible: usize,
    n_scoreable_candidates: usize,
    n_completed_zero_candidates: usize,
    n_missing_computations: usize,
    n_duplicate_candidates_removed: usize,
    n_endpoints_total: usize,
    n_positive_endpoints: usize,
    n_negative_endpoints: usize,
    n_endpoints_with_tensor_supported_candidate: usize,
}

#[derive(Clone, Debug, serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TransferManifest {
    schema_version: u32,
    package_kind: String,
    config_path: String,
    config_sha256: String,
    input_bundle: String,
    input_manifest_sha256: String,
    log_epsilon: f64,
    jobs: Vec<JobRecord>,
    output_files: BTreeMap<String, FileRecord>,
}

impl TransferManifest {
    pub fn config_sha256(&self) -> &str {
        &self.config_sha256
    }

    pub fn input_manifest_sha256(&self) -> &str {
        &self.input_manifest_sha256
    }
}

#[derive(Clone, Debug, serde::Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct JobRecord {
    cohort: String,
    parent_cohort: String,
    input_view_id: String,
    view_role: TransferViewRole,
    selection_eligible: bool,
    bundle_eligible: bool,
    model: String,
    branch: String,
    relative_directory: String,
    primary_representation: Representation,
    query_q_representation: Option<Representation>,
    mapping_kind: String,
    mapping_sha256: String,
    nci_summary_sha256: String,
    endpoint_count: usize,
    positive_count: usize,
}

#[derive(Clone, Copy, Debug, serde::Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum TransferViewRole {
    Primary,
    Secondary,
}

pub fn build_transfer_package(config_path: &Path, output: &Path) -> Result<Value> {
    let config_path = config_path.canonicalize()?;
    let config = PipelineConfig::load(&config_path)?;
    external_validation_inputs::validate_bundle(&config.input_bundle)?;
    let config_sha256 = sha256_file(&config_path)?;
    let input_manifest: Value = read_json(&config.input_bundle.join("manifest.json"))?;
    let stage = stage_path(output)?;
    let result = build_transfer_inner(
        &config_path,
        &config_sha256,
        &config,
        &input_manifest,
        &stage,
    );
    match result {
        Ok(mut report) => {
            std::fs::rename(&stage, output)?;
            report["output"] = json!(output);
            Ok(report)
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(&stage);
            Err(error)
        }
    }
}

fn build_transfer_inner(
    config_path: &Path,
    config_sha256: &str,
    config: &PipelineConfig,
    input_manifest: &Value,
    stage: &Path,
) -> Result<Value> {
    let expected_jobs = expected_job_scopes(config).len();
    let mut jobs = Vec::with_capacity(expected_jobs);
    for cohort_id in COHORTS {
        let cohort = &config.cohorts[cohort_id];
        build_view_jobs(
            config,
            input_manifest,
            config_sha256,
            stage,
            cohort_id,
            cohort_id,
            TransferViewRole::Primary,
            true,
            true,
            None,
            &mut jobs,
        )?;
        for (view_id, view) in &cohort.views {
            build_view_jobs(
                config,
                input_manifest,
                config_sha256,
                stage,
                cohort_id,
                view_id,
                TransferViewRole::Secondary,
                view.selection_eligible,
                view.bundle_eligible,
                Some(&view.input_view_id),
                &mut jobs,
            )?;
        }
    }
    if jobs.len() != expected_jobs {
        bail!(
            "transfer batch produced {} jobs instead of {expected_jobs}",
            jobs.len()
        );
    }
    write_secondary_transfer_reports(stage, config, &jobs)?;
    let output_files = output_file_records(stage)?;
    let manifest = TransferManifest {
        schema_version: TRANSFER_MANIFEST_SCHEMA_VERSION,
        package_kind: "iris_fullroster_transfer_package".into(),
        config_path: config_path.to_string_lossy().into_owned(),
        config_sha256: config_sha256.into(),
        input_bundle: config.input_bundle.to_string_lossy().into_owned(),
        input_manifest_sha256: config.input_manifest_sha256.clone(),
        log_epsilon: config.log_epsilon,
        jobs,
        output_files,
    };
    write_json(&stage.join("manifest.json"), &manifest)?;
    validate_transfer_package(stage)?;
    Ok(json!({
        "output": stage,
        "jobs": expected_jobs,
        "manifest_sha256": sha256_file(&stage.join("manifest.json"))?,
    }))
}

#[allow(clippy::too_many_arguments)]
fn build_view_jobs(
    config: &PipelineConfig,
    input_manifest: &Value,
    config_sha256: &str,
    stage: &Path,
    parent_cohort_id: &str,
    view_id: &str,
    view_role: TransferViewRole,
    selection_eligible: bool,
    bundle_eligible: bool,
    input_view_id: Option<&str>,
    jobs: &mut Vec<JobRecord>,
) -> Result<()> {
    let cohort = &config.cohorts[parent_cohort_id];
    let input_view_identity = input_view_id.unwrap_or(&cohort.primary_view_id);
    for model_id in MODELS {
        let model = &config.models[model_id];
        let primary = cohort
            .tensors
            .get(&model.primary)
            .context("validated tensor representation disappeared")?;
        let q_run = model.query_q.map(|representation| {
            cohort
                .tensors
                .get(&representation)
                .expect("validated tensor representation disappeared")
        });
        let mapping_kind = model.primary.mapping_kind();
        let mapping = if let Some(input_view_id) = input_view_id {
            input_view_mapping(
                &config.input_bundle,
                input_manifest,
                &cohort.input_prefix,
                input_view_id,
                mapping_kind,
            )?
        } else {
            input_mapping(
                &config.input_bundle,
                input_manifest,
                &cohort.input_prefix,
                &cohort.primary_view_id,
                mapping_kind,
            )?
        };
        for branch in BRANCHES {
            let relative = match view_role {
                TransferViewRole::Primary => PathBuf::from(view_id).join(model_id).join(branch),
                TransferViewRole::Secondary => PathBuf::from("secondary")
                    .join(view_id)
                    .join(model_id)
                    .join(branch),
            };
            let directory = stage.join(&relative);
            std::fs::create_dir_all(&directory)?;
            let report = run_transfer_job(TransferJob {
                cohort_id: view_id,
                parent_cohort_id,
                input_view_id: input_view_identity,
                view_role,
                selection_eligible,
                bundle_eligible,
                cohort,
                model_id,
                model,
                branch,
                primary,
                q_run,
                mapping: &mapping,
                l2_specs: &config.fixed_l2,
                adaptive_l2: &config.adaptive_l2,
                input_manifest_sha256: &config.input_manifest_sha256,
                config_sha256,
                output: &directory,
            })?;
            jobs.push(JobRecord {
                cohort: view_id.to_owned(),
                parent_cohort: parent_cohort_id.to_owned(),
                input_view_id: input_view_identity.to_owned(),
                view_role,
                selection_eligible,
                bundle_eligible,
                model: model_id.to_owned(),
                branch: branch.to_owned(),
                relative_directory: relative.to_string_lossy().into_owned(),
                primary_representation: model.primary,
                query_q_representation: model.query_q,
                mapping_kind: mapping_kind.to_owned(),
                mapping_sha256: sha256_file(&mapping)?,
                nci_summary_sha256: model.nci_summary_sha256.clone(),
                endpoint_count: report.endpoint_count,
                positive_count: report.positive_count,
            });
        }
    }
    Ok(())
}

fn transfer_metrics(path: &Path) -> Result<BTreeMap<String, (f64, f64)>> {
    let mut reader = csv::Reader::from_path(path)?;
    let headers = reader.headers()?.clone();
    let variant = required_column(&headers, "l2_variant", path)?;
    let roc = required_column(&headers, "roc_auc", path)?;
    let ap = required_column(&headers, "average_precision", path)?;
    let mut metrics = BTreeMap::new();
    for record in reader.records() {
        let record = record?;
        let id = csv_field(&record, variant, path)?.to_owned();
        let values = (
            csv_field(&record, roc, path)?.parse::<f64>()?,
            csv_field(&record, ap, path)?.parse::<f64>()?,
        );
        if metrics.insert(id.clone(), values).is_some() {
            bail!("duplicate L2 metric {id} in {}", path.display());
        }
    }
    Ok(metrics)
}

fn write_secondary_transfer_reports(
    stage: &Path,
    config: &PipelineConfig,
    jobs: &[JobRecord],
) -> Result<()> {
    let by_scope: BTreeMap<_, _> = jobs
        .iter()
        .map(|job| {
            (
                (job.cohort.as_str(), job.model.as_str(), job.branch.as_str()),
                job,
            )
        })
        .collect();
    for parent_cohort in COHORTS {
        let cohort = &config.cohorts[parent_cohort];
        for view_id in cohort.views.keys() {
            let directory = stage.join("secondary").join(view_id);
            let comparison_parent = cohort.primary_view_id.as_str();
            let csv_path = directory.join(format!("comparison_to_{comparison_parent}.csv"));
            let mut writer = csv::Writer::from_path(&csv_path)?;
            writer.write_record([
                "view_id",
                "parent_cohort",
                "model",
                "branch",
                "l2_variant",
                "metric",
                "view_value",
                "parent_value",
                "delta_from_parent",
                "n_endpoints",
                "n_positive",
                "n_completed_zero_candidates",
                "n_missing_computations",
                "mapping_sha256",
            ])?;
            let mut job_audits = Vec::new();
            for model in MODELS {
                for branch in BRANCHES {
                    let secondary = by_scope[&(view_id.as_str(), model, branch)];
                    let primary = by_scope[&(parent_cohort, model, branch)];
                    let secondary_metrics = transfer_metrics(
                        &stage
                            .join(&secondary.relative_directory)
                            .join("transfer_metrics.csv"),
                    )?;
                    let primary_metrics = transfer_metrics(
                        &stage
                            .join(&primary.relative_directory)
                            .join("transfer_metrics.csv"),
                    )?;
                    if secondary_metrics.keys().collect::<Vec<_>>()
                        != primary_metrics.keys().collect::<Vec<_>>()
                    {
                        bail!("secondary and primary fixed-L2 metric rosters differ");
                    }
                    let summary: Value = read_json(
                        &stage
                            .join(&secondary.relative_directory)
                            .join("summary.json"),
                    )?;
                    let completed = summary["coverage"]["n_completed_zero_candidates"]
                        .as_u64()
                        .context("secondary summary lacks completed-zero count")?;
                    let missing = summary["coverage"]["n_missing_computations"]
                        .as_u64()
                        .context("secondary summary lacks missing-computation count")?;
                    if missing != 0 {
                        bail!("secondary view {view_id} contains missing computations");
                    }
                    for (variant, &(secondary_roc, secondary_ap)) in &secondary_metrics {
                        let &(primary_roc, primary_ap) = &primary_metrics[variant];
                        let (metric, view_value, parent_value) = if branch == "pr" {
                            ("average_precision", secondary_ap, primary_ap)
                        } else {
                            ("roc_auc", secondary_roc, primary_roc)
                        };
                        writer.serialize((
                            view_id,
                            comparison_parent,
                            model,
                            branch,
                            variant,
                            metric,
                            view_value,
                            parent_value,
                            view_value - parent_value,
                            secondary.endpoint_count,
                            secondary.positive_count,
                            completed,
                            missing,
                            &secondary.mapping_sha256,
                        ))?;
                    }
                    job_audits.push(json!({
                        "model": model,
                        "branch": branch,
                        "endpoint_count": secondary.endpoint_count,
                        "positive_count": secondary.positive_count,
                        "mapping_kind": secondary.mapping_kind,
                        "mapping_sha256": secondary.mapping_sha256,
                        "n_completed_zero_candidates": completed,
                        "n_missing_computations": missing,
                    }));
                }
            }
            writer.flush()?;
            let json_path = directory.join(format!("comparison_to_{comparison_parent}.json"));
            write_json(
                &json_path,
                &json!({
                    "schema_version": 1,
                    "view_id": view_id,
                    "parent_cohort": comparison_parent,
                    "role": "secondary",
                    "selection_eligible": false,
                    "bundle_eligible": false,
                    "comparison_csv": csv_path.file_name().and_then(|name| name.to_str()),
                    "comparison_csv_sha256": sha256_file(&csv_path)?,
                    "tensor_hashes": cohort.tensors,
                    "jobs": job_audits,
                    "coverage_requirement": "100%; every candidate resolves to a completed tensor observation",
                }),
            )?;
            write_json(
                &directory.join("view_manifest.json"),
                &json!({
                    "schema_version": 1,
                    "view_id": view_id,
                    "parent_cohort": comparison_parent,
                    "role": "secondary",
                    "selection_eligible": false,
                    "bundle_eligible": false,
                    "mapping_hashes": jobs.iter()
                        .filter(|job| job.cohort == *view_id)
                        .map(|job| job.mapping_sha256.clone())
                        .collect::<BTreeSet<_>>(),
                    "tensor_hashes": cohort.tensors,
                    "comparison_json": json_path.file_name().and_then(|name| name.to_str()),
                    "comparison_json_sha256": sha256_file(&json_path)?,
                    "n_missing_computations": 0,
                }),
            )?;
        }
    }
    Ok(())
}

pub fn validate_transfer_package(package: &Path) -> Result<Value> {
    let manifest = load_validated_transfer_manifest(package)?;
    Ok(json!({
        "status": "pass",
        "package": package,
        "jobs": manifest.jobs.len(),
        "files": manifest.output_files.len(),
        "manifest_sha256": sha256_file(&package.join("manifest.json"))?,
    }))
}

pub fn validate_transfer_package_against_config(
    package: &Path,
    config_path: &Path,
) -> Result<TransferManifest> {
    let manifest = load_validated_transfer_manifest(package)?;
    let config_path = config_path.canonicalize()?;
    let config_sha256 = sha256_file(&config_path)?;
    if manifest.config_sha256 != config_sha256 {
        bail!(
            "transfer package configuration hash differs from the supplied pipeline configuration"
        );
    }
    let config = PipelineConfig::load(&config_path)?;
    let input_bundle = resolve_relocated_bundle(
        package,
        Path::new(&manifest.input_bundle),
        &manifest.input_manifest_sha256,
    )?;
    validate_manifest_against_config(package, &manifest, &config, &input_bundle)?;
    Ok(manifest)
}

fn relocation_candidates(package: &Path, recorded: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![recorded.to_path_buf()];
    if let (Some(parent), Some(name)) = (package.parent(), recorded.file_name()) {
        let sibling = parent.join(name);
        if sibling != recorded {
            candidates.push(sibling);
        }
    }
    candidates
}

fn resolve_relocated_config(
    package: &Path,
    recorded: &Path,
    expected_hash: &str,
) -> Result<PathBuf> {
    for candidate in relocation_candidates(package, recorded) {
        if candidate.is_file() && sha256_file(&candidate)? == expected_hash {
            return Ok(candidate.canonicalize()?);
        }
    }
    bail!("transfer package configuration hash no longer resolves")
}

fn resolve_relocated_bundle(
    package: &Path,
    recorded: &Path,
    expected_manifest_hash: &str,
) -> Result<PathBuf> {
    for candidate in relocation_candidates(package, recorded) {
        let manifest = candidate.join("manifest.json");
        if candidate.is_dir()
            && manifest.is_file()
            && sha256_file(&manifest)? == expected_manifest_hash
        {
            return Ok(candidate.canonicalize()?);
        }
    }
    bail!("transfer package input-manifest hash no longer resolves")
}

fn expected_job_scopes(config: &PipelineConfig) -> BTreeSet<(String, String, String)> {
    let mut scopes = BTreeSet::new();
    for cohort_id in COHORTS {
        for model_id in MODELS {
            for branch in BRANCHES {
                scopes.insert((cohort_id.to_owned(), model_id.to_owned(), branch.to_owned()));
            }
        }
        for view_id in config.cohorts[cohort_id].views.keys() {
            for model_id in MODELS {
                for branch in BRANCHES {
                    scopes.insert((view_id.clone(), model_id.to_owned(), branch.to_owned()));
                }
            }
        }
    }
    scopes
}

pub fn load_validated_transfer_manifest(package: &Path) -> Result<TransferManifest> {
    let manifest_path = package.join("manifest.json");
    let manifest: TransferManifest = read_json(&manifest_path)?;
    if manifest.schema_version != TRANSFER_MANIFEST_SCHEMA_VERSION
        || manifest.package_kind != "iris_fullroster_transfer_package"
        || manifest.log_epsilon.to_bits() != LOCKED_LOG_EPSILON.to_bits()
    {
        bail!("invalid transfer package identity");
    }
    let config_path = resolve_relocated_config(
        package,
        Path::new(&manifest.config_path),
        &manifest.config_sha256,
    )?;
    let config = PipelineConfig::load_provenance(&config_path)?;
    let input_bundle = resolve_relocated_bundle(
        package,
        Path::new(&manifest.input_bundle),
        &manifest.input_manifest_sha256,
    )?;
    external_validation_inputs::validate_bundle(&input_bundle)?;
    let expected_scopes = expected_job_scopes(&config);
    let observed_scopes: BTreeSet<(String, String, String)> = manifest
        .jobs
        .iter()
        .map(|job| (job.cohort.clone(), job.model.clone(), job.branch.clone()))
        .collect();
    if manifest.jobs.len() != expected_scopes.len() || observed_scopes != expected_scopes {
        bail!("transfer package job registry is incomplete or duplicated");
    }
    if output_file_records(package)? != manifest.output_files {
        bail!("transfer package output registry is incomplete or contains stale entries");
    }
    for (relative, record) in &manifest.output_files {
        let path = package.join(relative);
        if !path.is_file()
            || path.metadata()?.len() != record.bytes
            || sha256_file(&path)? != record.sha256
        {
            bail!("transfer output hash or size mismatch: {}", path.display());
        }
    }
    for job in &manifest.jobs {
        let root = package.join(&job.relative_directory);
        for name in [
            "long_peptide_predictions.csv",
            "target_tau_selection_by_observation.csv",
            "summary.json",
        ] {
            if !root.join(name).is_file() {
                bail!("transfer job lacks {name}: {}", root.display());
            }
        }
        let summary: Value = read_json(&root.join("summary.json"))?;
        if summary["log_epsilon"].as_f64().map(f64::to_bits) != Some(LOCKED_LOG_EPSILON.to_bits())
            || summary["input_manifest_sha256"].as_str()
                != Some(manifest.input_manifest_sha256.as_str())
        {
            bail!("transfer summary contract mismatch: {}", root.display());
        }
    }
    validate_manifest_against_config(package, &manifest, &config, &input_bundle)?;
    Ok(manifest)
}

fn validate_manifest_against_config(
    package: &Path,
    manifest: &TransferManifest,
    config: &PipelineConfig,
    input_bundle: &Path,
) -> Result<()> {
    if manifest.input_manifest_sha256 != config.input_manifest_sha256
        || manifest.log_epsilon.to_bits() != config.log_epsilon.to_bits()
        || Path::new(&manifest.input_bundle) != config.input_bundle
    {
        bail!("transfer package and pipeline configuration disagree on their input contract");
    }
    let input_manifest: Value = read_json(&input_bundle.join("manifest.json"))?;
    let jobs: BTreeMap<_, _> = manifest
        .jobs
        .iter()
        .map(|job| {
            (
                (job.cohort.as_str(), job.model.as_str(), job.branch.as_str()),
                job,
            )
        })
        .collect();
    if jobs.len() != manifest.jobs.len() {
        bail!("transfer package contains duplicate job scopes");
    }
    for cohort_id in COHORTS {
        let cohort = &config.cohorts[cohort_id];
        for model_id in MODELS {
            let model = &config.models[model_id];
            let mapping_kind = model.primary.mapping_kind();
            let mapping = input_mapping(
                input_bundle,
                &input_manifest,
                &cohort.input_prefix,
                &cohort.primary_view_id,
                mapping_kind,
            )?;
            let mapping_sha256 = sha256_file(&mapping)?;
            for branch in BRANCHES {
                let job = jobs[&(cohort_id, model_id, branch)];
                let expected_relative = PathBuf::from(cohort_id).join(model_id).join(branch);
                if Path::new(&job.relative_directory) != expected_relative
                    || job.parent_cohort != cohort_id
                    || job.input_view_id != cohort.primary_view_id
                    || job.view_role != TransferViewRole::Primary
                    || !job.selection_eligible
                    || !job.bundle_eligible
                    || job.primary_representation != model.primary
                    || job.query_q_representation != model.query_q
                    || job.mapping_kind != mapping_kind
                    || job.mapping_sha256 != mapping_sha256
                    || job.nci_summary_sha256 != model.nci_summary_sha256
                {
                    bail!("transfer job contract mismatch for {cohort_id}/{model_id}/{branch}");
                }
                let summary: Value =
                    read_json(&package.join(&job.relative_directory).join("summary.json"))?;
                let summary_mapping = summary["mapping"]
                    .as_str()
                    .map(PathBuf::from)
                    .context("transfer summary lacks mapping")?;
                let summary_nci = summary["nci_summary"]
                    .as_str()
                    .map(PathBuf::from)
                    .context("transfer summary lacks NCI summary")?;
                let declared_mapping = Path::new(&manifest.input_bundle).join(
                    mapping
                        .file_name()
                        .context("resolved input mapping has no filename")?,
                );
                let mapping_identity_matches = summary_mapping == declared_mapping
                    || (summary_mapping.is_file()
                        && summary_mapping.canonicalize()? == mapping.canonicalize()?);
                if summary["cohort_id"].as_str() != Some(cohort_id)
                    || summary["parent_cohort_id"].as_str() != Some(cohort_id)
                    || summary["input_view_id"].as_str() != Some(cohort.primary_view_id.as_str())
                    || summary["view_role"].as_str() != Some("primary")
                    || summary["selection_eligible"].as_bool() != Some(true)
                    || summary["bundle_eligible"].as_bool() != Some(true)
                    || summary["model_id"].as_str() != Some(model_id)
                    || summary["branch"].as_str() != Some(branch)
                    || !mapping_identity_matches
                    || summary_nci != model.nci_summary
                    || summary["coverage"]["n_endpoints_total"].as_u64()
                        != Some(job.endpoint_count as u64)
                    || summary["coverage"]["n_positive_endpoints"].as_u64()
                        != Some(job.positive_count as u64)
                {
                    bail!(
                        "transfer summary disagrees with job record for {cohort_id}/{model_id}/{branch}"
                    );
                }
            }
        }
        for (view_id, view) in &cohort.views {
            for model_id in MODELS {
                let model = &config.models[model_id];
                let mapping_kind = model.primary.mapping_kind();
                let mapping = input_view_mapping(
                    input_bundle,
                    &input_manifest,
                    &cohort.input_prefix,
                    &view.input_view_id,
                    mapping_kind,
                )?;
                let mapping_sha256 = sha256_file(&mapping)?;
                for branch in BRANCHES {
                    let job = jobs[&(view_id.as_str(), model_id, branch)];
                    let expected_relative = PathBuf::from("secondary")
                        .join(view_id)
                        .join(model_id)
                        .join(branch);
                    if Path::new(&job.relative_directory) != expected_relative
                        || job.parent_cohort != cohort_id
                        || job.input_view_id != view.input_view_id
                        || job.view_role != TransferViewRole::Secondary
                        || job.selection_eligible != view.selection_eligible
                        || job.bundle_eligible != view.bundle_eligible
                        || job.primary_representation != model.primary
                        || job.query_q_representation != model.query_q
                        || job.mapping_kind != mapping_kind
                        || job.mapping_sha256 != mapping_sha256
                        || job.nci_summary_sha256 != model.nci_summary_sha256
                    {
                        bail!("transfer job contract mismatch for {view_id}/{model_id}/{branch}");
                    }
                    let summary: Value =
                        read_json(&package.join(&job.relative_directory).join("summary.json"))?;
                    if summary["cohort_id"].as_str() != Some(view_id)
                        || summary["parent_cohort_id"].as_str() != Some(cohort_id)
                        || summary["input_view_id"].as_str() != Some(view.input_view_id.as_str())
                        || summary["view_role"].as_str() != Some("secondary")
                        || summary["selection_eligible"].as_bool() != Some(view.selection_eligible)
                        || summary["bundle_eligible"].as_bool() != Some(view.bundle_eligible)
                        || summary["model_id"].as_str() != Some(model_id)
                        || summary["branch"].as_str() != Some(branch)
                        || summary["coverage"]["n_endpoints_total"].as_u64()
                            != Some(job.endpoint_count as u64)
                        || summary["coverage"]["n_positive_endpoints"].as_u64()
                            != Some(job.positive_count as u64)
                    {
                        bail!(
                            "transfer summary disagrees with job record for {view_id}/{model_id}/{branch}"
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn input_mapping(
    input_bundle: &Path,
    manifest: &Value,
    prefix: &str,
    primary_view_id: &str,
    kind: &str,
) -> Result<PathBuf> {
    let datasets = manifest["datasets"]
        .as_object()
        .context("input manifest datasets is not an object")?;
    let expected_name = format!("{prefix}_{kind}_longpep_mapping.csv");
    let mut matches = Vec::new();
    for audit in datasets.values() {
        let key = format!("{kind}_mapping");
        if audit["primary_view_id"].as_str() == Some(primary_view_id)
            && audit["output_files"][&key].as_str() == Some(expected_name.as_str())
        {
            matches.push(expected_name.clone());
        }
    }
    if matches.len() != 1 {
        bail!(
            "input manifest did not resolve exactly one {prefix}/{primary_view_id}/{kind} mapping"
        );
    }
    let path = input_bundle.join(&matches[0]);
    if !path.is_file() {
        bail!("input mapping is missing: {}", path.display());
    }
    Ok(path)
}

fn input_view_mapping(
    input_bundle: &Path,
    manifest: &Value,
    prefix: &str,
    view_id: &str,
    kind: &str,
) -> Result<PathBuf> {
    let datasets = manifest["datasets"]
        .as_object()
        .context("input manifest datasets is not an object")?;
    let primary_name = format!("{prefix}_{kind}_longpep_mapping.csv");
    let audits: Vec<_> = datasets
        .values()
        .filter(|audit| {
            audit["output_files"][format!("{kind}_mapping")].as_str() == Some(primary_name.as_str())
        })
        .collect();
    if audits.len() != 1 {
        bail!("input manifest did not resolve exactly one primary dataset for {prefix}/{kind}");
    }
    let view = &audits[0]["views"][view_id];
    if view["role"].as_str() != Some("secondary")
        || view["parent_view_id"].as_str() != audits[0]["primary_view_id"].as_str()
        || view["selection_eligible"].as_bool() != Some(false)
        || view["bundle_eligible"].as_bool() != Some(false)
        || view["subset_status"].as_str() != Some("pass")
    {
        bail!("input view {view_id} is absent or violates the secondary-view contract");
    }
    let key = format!("{kind}_mapping");
    let filename = view["output_files"][&key]
        .as_str()
        .with_context(|| format!("input view {view_id} lacks {key}"))?;
    let path = input_bundle.join(filename);
    if !path.is_file() {
        bail!("input view mapping is missing: {}", path.display());
    }
    Ok(path)
}

struct TransferJob<'a> {
    cohort_id: &'a str,
    parent_cohort_id: &'a str,
    input_view_id: &'a str,
    view_role: TransferViewRole,
    selection_eligible: bool,
    bundle_eligible: bool,
    cohort: &'a crate::contract::CohortConfig,
    model_id: &'a str,
    model: &'a crate::contract::ModelConfig,
    branch: &'a str,
    primary: &'a crate::contract::TensorRun,
    q_run: Option<&'a crate::contract::TensorRun>,
    mapping: &'a Path,
    l2_specs: &'a [L2Spec],
    adaptive_l2: &'a AdaptiveL2Spec,
    input_manifest_sha256: &'a str,
    config_sha256: &'a str,
    output: &'a Path,
}

struct TransferReport {
    endpoint_count: usize,
    positive_count: usize,
}

fn run_transfer_job(job: TransferJob<'_>) -> Result<TransferReport> {
    let metadata = load_metadata(&job.primary.metadata())?;
    let nci_summary: Value = read_json(&job.model.nci_summary)?;
    if nci_summary["tau_mode"].as_str() != Some("per_observation") {
        bail!(
            "NCI summary for {} is not per-observation tau",
            job.model_id
        );
    }
    let selected = select_regime(&nci_summary, job.branch, &metadata)?;
    validate_tau_axis(&nci_summary, job.branch, &metadata)?;
    let observations = load_observations(&job.primary.observations())?;
    let q_override = if let Some(source) = job.q_run {
        let source_observations = load_observations(&source.observations())?;
        let source_q = load_constant_q_values(&source.tensor(), source_observations.len())?;
        Some(build_q_override(
            &observations,
            &source_observations,
            &source_q,
        )?)
    } else {
        None
    };
    let (scores, row_counts, chosen_tau) = stream_profiled_scores(
        &job.primary.tensor(),
        &metadata,
        observations.len(),
        &selected,
        LOCKED_LOG_EPSILON,
        q_override.as_ref().map(|source| source.values.as_slice()),
    )?;
    let mappings = load_mapping(job.cohort, job.mapping)?;
    let (endpoints, candidates, mut coverage) =
        build_endpoint_candidates(&mappings, &observations, &row_counts)?;
    if endpoints.is_empty() {
        bail!("{} mapping produced no endpoints", job.cohort_id);
    }
    let labels: Vec<u8> = endpoints.iter().map(|endpoint| endpoint.label).collect();
    let positive_count = labels.iter().filter(|&&label| label == 1).count();
    if positive_count == 0 || positive_count == labels.len() {
        bail!("{} mapping contains only one class", job.cohort_id);
    }
    let floor = LOCKED_LOG_EPSILON.ln() as f32;
    coverage.n_completed_zero_candidates = candidates
        .iter()
        .flatten()
        .filter(|candidate| scores[candidate.obs_idx * selected.len()] <= floor)
        .count();
    if let Some(source) = &q_override {
        write_q_source_join(
            &job.output.join("target_presentation_q_source_join.csv"),
            &observations,
            source,
        )?;
    }
    write_tau_selection(TauSelectionOutput {
        path: &job.output.join("target_tau_selection_by_observation.csv"),
        dataset: job.cohort.dataset,
        observations: &observations,
        selected: &selected,
        scores: &scores,
        row_counts: &row_counts,
        chosen_tau: &chosen_tau,
        metadata: &metadata,
    })?;
    write_predictions(&job, &endpoints, &candidates, &scores, &labels, &selected)?;
    let endpoint_key = format!("({})", job.cohort.endpoint_fields.join(", "));
    let summary = json!({
        "analysis": "NCI-frozen full-roster transfer with per-observation tau and L2 aggregation",
        "schema_version": 2,
        "dataset": job.cohort.dataset.transfer_name(),
        "cohort_id": job.cohort_id,
        "parent_cohort_id": job.parent_cohort_id,
        "input_view_id": job.input_view_id,
        "view_role": job.view_role,
        "selection_eligible": job.selection_eligible,
        "bundle_eligible": job.bundle_eligible,
        "model_id": job.model_id,
        "branch": job.branch,
        "tensor": job.primary.tensor(),
        "observations": job.primary.observations(),
        "metadata": job.primary.metadata(),
        "mapping": job.mapping,
        "nci_summary": job.model.nci_summary,
        "primary_representation": job.model.primary,
        "query_q_representation": job.model.query_q,
        "component_model": if job.q_run.is_some() { "external query Q with primary-tensor P/N" } else { "Q and P/N from the same tensor" },
        "component_sources": {
            "q_tensor": job.q_run.map_or_else(|| job.primary.tensor(), |run| run.tensor()),
            "q_observations": job.q_run.map_or_else(|| job.primary.observations(), |run| run.observations()),
            "pn_tensor": job.primary.tensor(),
            "pi_tensor": job.primary.tensor(),
        },
        "selected_regimes": selected.iter().map(|row| json!({
            "selector": row.selector,
            "nci_regime_idx": row.nci_regime_idx,
            "target_parameters": row.decoded,
        })).collect::<Vec<_>>(),
        "tau_mode": "per_observation",
        "tau_values": metadata.tau_values,
        "log_epsilon": LOCKED_LOG_EPSILON,
        "complete_f_contract": "every eligible mapping row must resolve to a scored tensor observation; omission floors are forbidden",
        "l2_specs": job.l2_specs,
        "adaptive_l2": job.adaptive_l2,
        "endpoint_key": endpoint_key,
        "response_column": job.cohort.response_field,
        "label_column": job.cohort.label_field,
        "label_threshold": job.cohort.threshold,
        "comparison_operator": job.cohort.comparison_operator,
        "coverage": coverage,
        "input_manifest_sha256": job.input_manifest_sha256,
        "pipeline_config_sha256": job.config_sha256,
    });
    write_json(&job.output.join("summary.json"), &summary)?;
    Ok(TransferReport {
        endpoint_count: endpoints.len(),
        positive_count,
    })
}

fn write_predictions(
    job: &TransferJob<'_>,
    endpoints: &[Endpoint],
    candidates: &[Vec<CandidateRef>],
    scores: &[f32],
    labels: &[u8],
    selected: &[SelectedRegime],
) -> Result<()> {
    if selected.len() != 1 {
        bail!("one metric branch must select exactly one target regime");
    }
    let floor = LOCKED_LOG_EPSILON.ln() as f32;
    let mut writer = csv::Writer::from_path(job.output.join("long_peptide_predictions.csv"))?;
    writer.write_record([
        "dataset",
        "selector",
        "regime_idx",
        "l2_variant",
        "patient_id",
        "mutation",
        "long_peptide",
        "label",
        "component_model_id",
        "score",
        "n_candidates",
    ])?;
    let mut metric_writer = csv::Writer::from_path(job.output.join("transfer_metrics.csv"))?;
    metric_writer.write_record([
        "dataset",
        "selector",
        "regime_idx",
        "l2_variant",
        "roc_auc",
        "average_precision",
        "n_endpoints",
        "n_positive",
    ])?;
    for spec in job.l2_specs {
        let mut endpoint_scores = Vec::with_capacity(endpoints.len());
        for (endpoint_index, endpoint) in endpoints.iter().enumerate() {
            let values: Vec<f32> = candidates[endpoint_index]
                .iter()
                .map(|candidate| scores[candidate.obs_idx * selected.len()])
                .collect();
            let score = aggregate(&values, spec, floor)?;
            endpoint_scores.push(score);
            writer.serialize((
                job.cohort.dataset.transfer_name(),
                &selected[0].selector,
                selected[0].decoded.regime_idx,
                spec.id(),
                &endpoint.patient_id,
                &endpoint.mutation,
                &endpoint.long_peptide,
                endpoint.label,
                job.model_id,
                score,
                values.len(),
            ))?;
        }
        metric_writer.serialize((
            job.cohort.dataset.transfer_name(),
            &selected[0].selector,
            selected[0].decoded.regime_idx,
            spec.id(),
            roc_auc(&endpoint_scores, labels),
            average_precision(&endpoint_scores, labels),
            labels.len(),
            labels.iter().filter(|&&label| label == 1).count(),
        ))?;
    }
    let mut endpoint_scores = Vec::with_capacity(endpoints.len());
    for (endpoint_index, endpoint) in endpoints.iter().enumerate() {
        let roster = &candidates[endpoint_index];
        let values: Vec<f64> = roster
            .iter()
            .map(|candidate| scores[candidate.obs_idx * selected.len()] as f64)
            .collect();
        let peptides: Vec<String> = roster
            .iter()
            .map(|candidate| candidate.peptide.clone())
            .collect();
        let hlas: Vec<String> = roster
            .iter()
            .map(|candidate| candidate.hla.clone())
            .collect();
        let score = aggregate_frozen_adaptive_l2(
            &values,
            &peptides,
            &hlas,
            job.adaptive_l2,
            LOCKED_LOG_EPSILON.ln(),
        )?;
        endpoint_scores.push(score as f32);
        writer.serialize((
            job.cohort.dataset.transfer_name(),
            &selected[0].selector,
            selected[0].decoded.regime_idx,
            job.adaptive_l2.id(),
            &endpoint.patient_id,
            &endpoint.mutation,
            &endpoint.long_peptide,
            endpoint.label,
            job.model_id,
            score,
            values.len(),
        ))?;
    }
    metric_writer.serialize((
        job.cohort.dataset.transfer_name(),
        &selected[0].selector,
        selected[0].decoded.regime_idx,
        job.adaptive_l2.id(),
        roc_auc(&endpoint_scores, labels),
        average_precision(&endpoint_scores, labels),
        labels.len(),
        labels.iter().filter(|&&label| label == 1).count(),
    ))?;
    writer.flush()?;
    metric_writer.flush()?;
    Ok(())
}

fn load_metadata(path: &Path) -> Result<Metadata> {
    let value: Value = read_json(path)?;
    let n_params = value["n_params"].as_u64().context("metadata n_params")? as usize;
    let n_mn = value["n_mn"].as_u64().context("metadata n_mn")? as usize;
    let n_tau = value["n_tau"].as_u64().context("metadata n_tau")? as usize;
    let geometry_params = value["geometry_params"]
        .as_array()
        .context("metadata geometry_params")?
        .iter()
        .map(|row| {
            let row = row.as_array().context("metadata geometry row")?;
            if row.len() != 4 {
                bail!("metadata geometry row must have four values");
            }
            Ok([
                json_number(&row[0])?,
                json_number(&row[1])?,
                json_number(&row[2])?,
                json_number(&row[3])?,
            ])
        })
        .collect::<Result<Vec<_>>>()?;
    let tau_values = value["tau_values"]
        .as_array()
        .context("metadata tau_values")?
        .iter()
        .map(json_number)
        .collect::<Result<Vec<_>>>()?;
    let mn_tuples = value["mn_tuples"]
        .as_array()
        .context("metadata mn_tuples")?
        .iter()
        .map(|row| {
            let row = row.as_array().context("metadata M/N row")?;
            if row.len() != 2 {
                bail!("metadata M/N row must have two values");
            }
            Ok([
                u32::try_from(row[0].as_u64().context("metadata M")?)?,
                u32::try_from(row[1].as_u64().context("metadata N")?)?,
            ])
        })
        .collect::<Result<Vec<_>>>()?;
    if geometry_params.len() * n_tau != n_params
        || tau_values.len() != n_tau
        || mn_tuples.len() != n_mn
        || n_tau == 0
    {
        bail!("metadata grid dimensions are inconsistent");
    }
    Ok(Metadata {
        n_params,
        n_mn,
        n_tau,
        geometry_params,
        tau_values,
        mn_tuples,
    })
}

fn json_number(value: &Value) -> Result<f64> {
    if let Some(number) = value.as_f64() {
        Ok(number)
    } else if let Some(text) = value.as_str() {
        Ok(text.parse()?)
    } else {
        bail!("expected JSON number or number string")
    }
}

fn column_index(batch: &RecordBatch, name: &str) -> Result<usize> {
    batch
        .schema()
        .index_of(name)
        .with_context(|| format!("missing Parquet column {name}"))
}

fn u32_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a UInt32Array> {
    batch
        .column(column_index(batch, name)?)
        .as_any()
        .downcast_ref::<UInt32Array>()
        .with_context(|| format!("Parquet column {name} is not uint32"))
}

fn u8_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a UInt8Array> {
    batch
        .column(column_index(batch, name)?)
        .as_any()
        .downcast_ref::<UInt8Array>()
        .with_context(|| format!("Parquet column {name} is not uint8"))
}

fn f32_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a Float32Array> {
    batch
        .column(column_index(batch, name)?)
        .as_any()
        .downcast_ref::<Float32Array>()
        .with_context(|| format!("Parquet column {name} is not float32"))
}

fn string_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a StringArray> {
    batch
        .column(column_index(batch, name)?)
        .as_any()
        .downcast_ref::<StringArray>()
        .with_context(|| format!("Parquet column {name} is not string"))
}

fn load_observations(path: &Path) -> Result<Vec<Observation>> {
    let file = File::open(path)?;
    let mut observations = Vec::new();
    for batch in ParquetRecordBatchReaderBuilder::try_new(file)?.build()? {
        let batch = batch?;
        let obs_idx = u32_column(&batch, "obs_idx")?;
        let peptide = string_column(&batch, "peptide")?;
        let hla = string_column(&batch, "hla")?;
        let env_id = u32_column(&batch, "env_id")?;
        let patient_id = string_column(&batch, "patient_id")?;
        let label = u8_column(&batch, "label")?;
        let gene = string_column(&batch, "gene")?;
        let cancer_type = string_column(&batch, "cancer_type")?;
        let wt_mt_group_id = string_column(&batch, "wt_mt_group_id")?;
        for row in 0..batch.num_rows() {
            observations.push(Observation {
                obs_idx: obs_idx.value(row) as usize,
                peptide: peptide.value(row).to_owned(),
                hla: hla.value(row).to_owned(),
                env_id: env_id.value(row),
                patient_id: patient_id.value(row).to_owned(),
                label: label.value(row),
                gene: gene.value(row).to_owned(),
                cancer_type: cancer_type.value(row).to_owned(),
                wt_mt_group_id: wt_mt_group_id.value(row).to_owned(),
            });
        }
    }
    observations.sort_by_key(|observation| observation.obs_idx);
    for (expected, observation) in observations.iter().enumerate() {
        if observation.obs_idx != expected {
            bail!("observations are not contiguous and zero-based");
        }
    }
    Ok(observations)
}

type ObservationJoinKey = (String, String, String, u8, String, String, String);

fn observation_join_key(observation: &Observation) -> ObservationJoinKey {
    (
        observation.peptide.clone(),
        normalize_hla(&observation.hla),
        observation.patient_id.clone(),
        observation.label,
        observation.gene.clone(),
        observation.cancer_type.clone(),
        observation.wt_mt_group_id.clone(),
    )
}

fn load_constant_q_values(path: &Path, n_observations: usize) -> Result<Vec<f32>> {
    let file = File::open(path)?;
    let mut values = vec![f32::NAN; n_observations];
    for batch in ParquetRecordBatchReaderBuilder::try_new(file)?.build()? {
        let batch = batch?;
        let obs_idx = u32_column(&batch, "obs_idx")?;
        let q = f32_column(&batch, "q_value")?;
        for row in 0..batch.num_rows() {
            let observation = obs_idx.value(row) as usize;
            if observation >= values.len() {
                bail!("Q tensor observation is outside its observation roster");
            }
            let value = q.value(row);
            if !value.is_finite() || value < 0.0 {
                bail!("Q tensor contains an invalid q_value");
            }
            if values[observation].is_nan() {
                values[observation] = value;
            } else if values[observation].to_bits() != value.to_bits() {
                bail!("Q is parameter-dependent at observation {observation}");
            }
        }
    }
    if values.iter().any(|value| value.is_nan()) {
        bail!("Q tensor lacks at least one source observation");
    }
    Ok(values)
}

fn build_q_override(
    target: &[Observation],
    source: &[Observation],
    source_q: &[f32],
) -> Result<QOverride> {
    if source.len() != source_q.len() {
        bail!("Q source observation and value counts differ");
    }
    let mut lookup = HashMap::new();
    for (index, observation) in source.iter().enumerate() {
        if lookup
            .insert(observation_join_key(observation), index)
            .is_some()
        {
            bail!("Q source observation identity is duplicated");
        }
    }
    let mut used = BTreeSet::new();
    let mut values = Vec::with_capacity(target.len());
    let mut source_obs_indices = Vec::with_capacity(target.len());
    let mut source_env_ids = Vec::with_capacity(target.len());
    for observation in target {
        let source_index = *lookup
            .get(&observation_join_key(observation))
            .with_context(|| {
                format!(
                    "no Q source match for patient={} peptide={} hla={}",
                    observation.patient_id, observation.peptide, observation.hla
                )
            })?;
        used.insert(source_index);
        values.push(source_q[source_index]);
        source_obs_indices.push(source_index);
        source_env_ids.push(source[source_index].env_id);
    }
    if used.len() != source.len() || target.len() != source.len() {
        bail!("Q source join is not exhaustive one-to-one");
    }
    Ok(QOverride {
        values,
        source_obs_indices,
        source_env_ids,
    })
}

fn select_regime(
    summary: &Value,
    branch: &str,
    metadata: &Metadata,
) -> Result<Vec<SelectedRegime>> {
    let (summary_key, selector) = match branch {
        "pr" => ("best_by_pr_auc", "nci_best_pr"),
        "roc" => ("best_by_roc_auc", "nci_best_roc"),
        _ => bail!("unknown metric branch {branch}"),
    };
    let source = &summary[summary_key];
    let geometry = [
        json_number(&source["d_pos"]).context("selected d_pos")?,
        json_number(&source["d_neg"]).context("selected d_neg")?,
        json_number(&source["steepness_pos"]).context("selected steepness_pos")?,
        json_number(&source["steepness_neg"]).context("selected steepness_neg")?,
    ];
    let m = u32::try_from(source["M"].as_u64().context("selected M")?)?;
    let n = u32::try_from(source["N"].as_u64().context("selected N")?)?;
    let geometry_matches: Vec<_> = metadata
        .geometry_params
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            candidate
                .iter()
                .zip(geometry)
                .all(|(&actual, expected)| values_close(actual, expected))
                .then_some(index)
        })
        .collect();
    let mn_matches: Vec<_> = metadata
        .mn_tuples
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| (*candidate == [m, n]).then_some(index))
        .collect();
    if geometry_matches.len() != 1 || mn_matches.len() != 1 {
        bail!("selected NCI regime does not map uniquely into target metadata");
    }
    let geometry_idx = geometry_matches[0];
    let mn_idx = mn_matches[0];
    let regime_idx = geometry_idx * metadata.n_mn + mn_idx;
    Ok(vec![SelectedRegime {
        selector: selector.into(),
        nci_regime_idx: source["regime_idx"]
            .as_u64()
            .context("selected regime_idx")? as usize,
        decoded: DecodedRegime {
            regime_idx,
            param_idx: geometry_idx,
            mn_idx,
            geometry_idx,
            tau_idx: metadata.n_tau,
            d_pos: geometry[0],
            d_neg: geometry[1],
            steepness_pos: geometry[2],
            steepness_neg: geometry[3],
            tau_thymus: None,
            m,
            n,
        },
    }])
}

fn values_close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0)
}

fn validate_tau_axis(summary: &Value, branch: &str, metadata: &Metadata) -> Result<()> {
    let rows = summary["tau_per_observation"][branch]["histogram"]
        .as_array()
        .context("NCI summary lacks the selected tau histogram")?;
    if rows.len() != metadata.n_tau {
        bail!("NCI and target tau axes have different lengths");
    }
    let mut seen = vec![false; metadata.n_tau];
    for row in rows {
        let index = row["tau_idx"].as_u64().context("NCI tau_idx")? as usize;
        if index >= metadata.n_tau || seen[index] {
            bail!("NCI tau histogram contains an invalid or duplicate index");
        }
        seen[index] = true;
        if !values_close(
            metadata.tau_values[index],
            json_number(&row["tau_thymus"]).context("NCI tau_thymus")?,
        ) {
            bail!("NCI and target tau values differ at index {index}");
        }
    }
    Ok(())
}

fn stream_profiled_scores(
    path: &Path,
    metadata: &Metadata,
    n_observations: usize,
    selected: &[SelectedRegime],
    epsilon: f64,
    q_override: Option<&[f32]>,
) -> Result<(Vec<f32>, Vec<usize>, Vec<usize>)> {
    let floor = epsilon.ln() as f32;
    let mut scores = vec![floor; n_observations * selected.len()];
    let mut chosen_tau = vec![usize::MAX; scores.len()];
    let mut seen = vec![false; scores.len() * metadata.n_tau];
    let mut raw_rows = vec![0usize; n_observations];
    let mut row_counts = vec![0usize; n_observations];
    let mut selected_lookup: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (position, regime) in selected.iter().enumerate() {
        selected_lookup
            .entry((regime.decoded.geometry_idx, regime.decoded.mn_idx))
            .or_default()
            .push(position);
    }
    let file = File::open(path)?;
    for batch in ParquetRecordBatchReaderBuilder::try_new(file)?.build()? {
        let batch = batch?;
        let obs_idx = u32_column(&batch, "obs_idx")?;
        let param_idx = u32_column(&batch, "param_idx")?;
        let mn_idx = u32_column(&batch, "mn_idx")?;
        let q = f32_column(&batch, "q_value")?;
        let pos = f32_column(&batch, "pos_prob")?;
        let neg = f32_column(&batch, "neg_prob")?;
        let pi = f32_column(&batch, "pi_eh")?;
        for row in 0..batch.num_rows() {
            let observation = obs_idx.value(row) as usize;
            let parameter = param_idx.value(row) as usize;
            let mn = mn_idx.value(row) as usize;
            if observation >= n_observations
                || parameter >= metadata.n_params
                || mn >= metadata.n_mn
            {
                bail!("tensor index is outside the metadata grid");
            }
            raw_rows[observation] += 1;
            let geometry = parameter / metadata.n_tau;
            let tau = parameter % metadata.n_tau;
            if let Some(positions) = selected_lookup.get(&(geometry, mn)) {
                let score = log_score(
                    q_override.map_or(q.value(row), |values| values[observation]) as f64,
                    pos.value(row) as f64,
                    neg.value(row) as f64,
                    pi.value(row) as f64,
                    epsilon,
                );
                if !score.is_finite() {
                    bail!("profiled tensor score is nonfinite");
                }
                for &position in positions {
                    let offset = observation * selected.len() + position;
                    let coverage = offset * metadata.n_tau + tau;
                    if seen[coverage] {
                        bail!("duplicate selected tensor cell");
                    }
                    seen[coverage] = true;
                    if chosen_tau[offset] == usize::MAX
                        || score > scores[offset]
                        || (score == scores[offset] && tau < chosen_tau[offset])
                    {
                        scores[offset] = score;
                        chosen_tau[offset] = tau;
                    }
                }
            }
        }
    }
    for observation in 0..n_observations {
        if raw_rows[observation] == 0 {
            continue;
        }
        for position in 0..selected.len() {
            let offset = observation * selected.len() + position;
            let count = (0..metadata.n_tau)
                .filter(|&tau| seen[offset * metadata.n_tau + tau])
                .count();
            if count != metadata.n_tau {
                bail!(
                    "covered observation {observation} has {count}/{} selected tau rows",
                    metadata.n_tau
                );
            }
        }
        row_counts[observation] = selected.len() * metadata.n_tau;
    }
    Ok((scores, row_counts, chosen_tau))
}

fn normalize_hla(value: &str) -> String {
    external_validation_inputs::builder::normalize_hla(value)
}

fn required_column(headers: &csv::StringRecord, name: &str, path: &Path) -> Result<usize> {
    headers
        .iter()
        .position(|header| header == name)
        .with_context(|| format!("missing CSV column {name} in {}", path.display()))
}

fn optional_column(headers: &csv::StringRecord, name: &str) -> Option<usize> {
    headers.iter().position(|header| header == name)
}

fn csv_field<'a>(record: &'a csv::StringRecord, index: usize, path: &Path) -> Result<&'a str> {
    record
        .get(index)
        .with_context(|| format!("short CSV row in {}", path.display()))
}

fn load_mapping(cohort: &crate::contract::CohortConfig, path: &Path) -> Result<Vec<MappingRow>> {
    let mut reader = csv::Reader::from_path(path)?;
    let headers = reader.headers()?.clone();
    let patient = required_column(&headers, "patient_id", path)?;
    let env = required_column(&headers, "env_id", path)?;
    let mutation = optional_column(&headers, "mutation");
    let long_peptide = required_column(&headers, "long_peptide", path)?;
    let nmer = required_column(&headers, "nmer", path)?;
    let hla = required_column(&headers, "HLA-RE", path)?;
    let status = required_column(&headers, "mapping_status", path)?;
    let response = cohort
        .response_field
        .as_deref()
        .map(|name| required_column(&headers, name, path))
        .transpose()?;
    let label = cohort
        .label_field
        .as_deref()
        .map(|name| required_column(&headers, name, path))
        .transpose()?;
    let mut rows = Vec::new();
    for result in reader.records() {
        let record = result?;
        let observed_label = if let Some(index) = label {
            match csv_field(&record, index, path)?.trim() {
                "0" => 0,
                "1" => 1,
                value => bail!("non-binary label {value:?} in {}", path.display()),
            }
        } else {
            let value: f64 = csv_field(
                &record,
                response.context("validated response column disappeared")?,
                path,
            )?
            .trim()
            .parse()
            .with_context(|| format!("invalid response in {}", path.display()))?;
            let threshold = cohort
                .threshold
                .context("validated threshold disappeared")?;
            match cohort
                .comparison_operator
                .context("validated comparison operator disappeared")?
            {
                crate::contract::ComparisonOperator::GreaterThan => u8::from(value > threshold),
                crate::contract::ComparisonOperator::GreaterThanOrEqual => {
                    u8::from(value >= threshold)
                }
            }
        };
        rows.push(MappingRow {
            patient_id: csv_field(&record, patient, path)?.trim().to_owned(),
            env_id: csv_field(&record, env, path)?
                .trim()
                .parse()
                .with_context(|| format!("invalid env_id in {}", path.display()))?,
            mutation: if cohort
                .endpoint_fields
                .iter()
                .any(|field| field == "mutation")
            {
                mutation
                    .map(|index| csv_field(&record, index, path))
                    .transpose()?
                    .unwrap_or("")
                    .trim()
                    .to_owned()
            } else {
                String::new()
            },
            long_peptide: csv_field(&record, long_peptide, path)?.trim().to_owned(),
            nmer: csv_field(&record, nmer, path)?.trim().to_uppercase(),
            hla: normalize_hla(csv_field(&record, hla, path)?),
            status: csv_field(&record, status, path)?
                .trim()
                .to_ascii_lowercase(),
            label: observed_label,
        });
    }
    Ok(rows)
}

type TensorLookupKey = (String, u32, String, String);
type EndpointKey = (String, String, String);

fn build_endpoint_candidates(
    mappings: &[MappingRow],
    observations: &[Observation],
    row_counts: &[usize],
) -> Result<(Vec<Endpoint>, Vec<Vec<CandidateRef>>, Coverage)> {
    if observations.len() != row_counts.len() {
        bail!("observation and tensor-coverage lengths differ");
    }
    let mut tensor_lookup = HashMap::<TensorLookupKey, usize>::new();
    for observation in observations {
        let key = (
            observation.patient_id.clone(),
            observation.env_id,
            observation.peptide.to_uppercase(),
            normalize_hla(&observation.hla),
        );
        if tensor_lookup
            .insert(key.clone(), observation.obs_idx)
            .is_some()
        {
            bail!("duplicate tensor observation identity: {key:?}");
        }
    }

    let mut endpoint_labels = BTreeMap::<EndpointKey, u8>::new();
    let mut grouped = BTreeMap::<EndpointKey, Vec<CandidateRef>>::new();
    let mut seen = BTreeMap::<(String, String, String, String, String), usize>::new();
    let mut coverage = Coverage {
        n_mapping_rows_read: mappings.len(),
        ..Coverage::default()
    };
    for row in mappings {
        let length = row.nmer.chars().count();
        if !(9..=12).contains(&length) {
            continue;
        }
        coverage.n_candidate_rows_eligible += 1;
        if row.patient_id.is_empty() || row.long_peptide.is_empty() || row.hla.is_empty() {
            bail!("mapping contains a blank endpoint or HLA identity");
        }
        if row.status != "scoreable" {
            bail!(
                "complete-F transfer requires mapping_status=scoreable; observed {:?}",
                row.status
            );
        }
        let tensor_key = (
            row.patient_id.clone(),
            row.env_id,
            row.nmer.clone(),
            row.hla.clone(),
        );
        let obs_idx = tensor_lookup
            .get(&tensor_key)
            .copied()
            .context("complete-F mapping row lacks a tensor observation")?;
        if row_counts[obs_idx] == 0 {
            bail!("complete-F mapping row resolves to an uncovered tensor observation");
        }
        let endpoint_key = (
            row.patient_id.clone(),
            row.mutation.clone(),
            row.long_peptide.clone(),
        );
        if let Some(previous) = endpoint_labels.insert(endpoint_key.clone(), row.label)
            && previous != row.label
        {
            bail!("conflicting labels for endpoint {endpoint_key:?}");
        }
        let candidate_key = (
            row.patient_id.clone(),
            row.mutation.clone(),
            row.long_peptide.clone(),
            row.nmer.clone(),
            row.hla.clone(),
        );
        if let Some(previous) = seen.get(&candidate_key) {
            if *previous != obs_idx {
                bail!("conflicting duplicate biological candidate {candidate_key:?}");
            }
            coverage.n_duplicate_candidates_removed += 1;
            continue;
        }
        seen.insert(candidate_key, obs_idx);
        coverage.n_scoreable_candidates += 1;
        grouped.entry(endpoint_key).or_default().push(CandidateRef {
            obs_idx,
            peptide: row.nmer.clone(),
            hla: row.hla.clone(),
        });
    }

    let mut endpoints = Vec::with_capacity(endpoint_labels.len());
    let mut candidates = Vec::with_capacity(endpoint_labels.len());
    for (key, label) in endpoint_labels {
        let roster = grouped.remove(&key).unwrap_or_default();
        if roster.is_empty() {
            bail!("endpoint {key:?} has no eligible 9--12mer candidate");
        }
        endpoints.push(Endpoint {
            patient_id: key.0,
            mutation: key.1,
            long_peptide: key.2,
            label,
        });
        candidates.push(roster);
    }
    coverage.n_endpoints_total = endpoints.len();
    coverage.n_positive_endpoints = endpoints
        .iter()
        .filter(|endpoint| endpoint.label == 1)
        .count();
    coverage.n_negative_endpoints = endpoints.len() - coverage.n_positive_endpoints;
    coverage.n_endpoints_with_tensor_supported_candidate = candidates
        .iter()
        .filter(|roster| !roster.is_empty())
        .count();
    Ok((endpoints, candidates, coverage))
}

fn write_q_source_join(path: &Path, target: &[Observation], source: &QOverride) -> Result<()> {
    if target.len() != source.values.len() {
        bail!("Q source join has the wrong length");
    }
    let mut writer = csv::Writer::from_path(path)?;
    writer.write_record([
        "target_obs_idx",
        "source_obs_idx",
        "target_env_id",
        "source_env_id",
        "patient_id",
        "peptide",
        "hla",
        "q_value",
    ])?;
    for (index, observation) in target.iter().enumerate() {
        writer.serialize((
            observation.obs_idx,
            source.source_obs_indices[index],
            observation.env_id,
            source.source_env_ids[index],
            &observation.patient_id,
            &observation.peptide,
            normalize_hla(&observation.hla),
            source.values[index],
        ))?;
    }
    writer.flush()?;
    Ok(())
}

struct TauSelectionOutput<'a> {
    path: &'a Path,
    dataset: Dataset,
    observations: &'a [Observation],
    selected: &'a [SelectedRegime],
    scores: &'a [f32],
    row_counts: &'a [usize],
    chosen_tau: &'a [usize],
    metadata: &'a Metadata,
}

fn write_tau_selection(output: TauSelectionOutput<'_>) -> Result<()> {
    let TauSelectionOutput {
        path,
        dataset,
        observations,
        selected,
        scores,
        row_counts,
        chosen_tau,
        metadata,
    } = output;
    if selected.len() != 1 || scores.len() != observations.len() || chosen_tau.len() != scores.len()
    {
        bail!("tau-selection dimensions are inconsistent");
    }
    let mut writer = csv::Writer::from_path(path)?;
    writer.write_record([
        "dataset",
        "selector",
        "regime_idx",
        "obs_idx",
        "patient_id",
        "env_id",
        "peptide",
        "hla",
        "score",
        "tau_idx",
        "tau_thymus",
        "tensor_rows",
    ])?;
    for (index, observation) in observations.iter().enumerate() {
        if row_counts[index] == 0 || chosen_tau[index] == usize::MAX {
            continue;
        }
        let tau_idx = chosen_tau[index];
        writer.serialize((
            dataset.transfer_name(),
            &selected[0].selector,
            selected[0].decoded.regime_idx,
            observation.obs_idx,
            &observation.patient_id,
            observation.env_id,
            &observation.peptide,
            normalize_hla(&observation.hla),
            scores[index],
            tau_idx,
            metadata.tau_values[tau_idx],
            row_counts[index],
        ))?;
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(index: usize, peptide: &str, hla: &str, env_id: u32) -> Observation {
        Observation {
            obs_idx: index,
            peptide: peptide.into(),
            hla: hla.into(),
            env_id,
            patient_id: "P1".into(),
            label: 1,
            gene: String::new(),
            cancer_type: String::new(),
            wt_mt_group_id: String::new(),
        }
    }

    fn mapping(nmer: &str, hla: &str, env_id: u32, status: &str) -> MappingRow {
        MappingRow {
            patient_id: "P1".into(),
            env_id,
            mutation: String::new(),
            long_peptide: "LONGPEPTIDE".into(),
            nmer: nmer.into(),
            hla: normalize_hla(hla),
            status: status.into(),
            label: 1,
        }
    }

    #[test]
    fn complete_f_roster_removes_exact_duplicate() {
        let observations = vec![observation(0, "AAAAAAAAA", "HLA-A*01:01", 7)];
        let mappings = vec![
            mapping("AAAAAAAAA", "A0101", 7, "scoreable"),
            mapping("AAAAAAAAA", "HLA-A*01:01", 7, "scoreable"),
        ];
        let (endpoints, candidates, audit) =
            build_endpoint_candidates(&mappings, &observations, &[1]).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(candidates[0].len(), 1);
        assert_eq!(audit.n_scoreable_candidates, 1);
        assert_eq!(audit.n_duplicate_candidates_removed, 1);
    }

    #[test]
    fn scoreability_disagreement_fails_closed() {
        let observations = vec![observation(0, "AAAAAAAAA", "A0101", 7)];
        let error = build_endpoint_candidates(
            &[mapping("AAAAAAAAA", "A0101", 7, "floor")],
            &observations,
            &[1],
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("complete-F transfer requires mapping_status=scoreable")
        );
    }

    #[test]
    fn provenance_dependencies_relocate_as_verified_siblings() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("full_roster_transfers");
        std::fs::create_dir(&package).unwrap();

        let config = root.path().join("pipeline.json");
        std::fs::write(&config, b"frozen configuration\n").unwrap();
        let resolved_config = resolve_relocated_config(
            &package,
            Path::new("/unavailable/original/pipeline.json"),
            &sha256_file(&config).unwrap(),
        )
        .unwrap();
        assert_eq!(resolved_config, config.canonicalize().unwrap());

        let input_bundle = root.path().join("full_roster_inputs_v1");
        std::fs::create_dir(&input_bundle).unwrap();
        let input_manifest = input_bundle.join("manifest.json");
        std::fs::write(&input_manifest, b"frozen input manifest\n").unwrap();
        let resolved_bundle = resolve_relocated_bundle(
            &package,
            Path::new("/unavailable/original/full_roster_inputs_v1"),
            &sha256_file(&input_manifest).unwrap(),
        )
        .unwrap();
        assert_eq!(resolved_bundle, input_bundle.canonicalize().unwrap());
    }
}
