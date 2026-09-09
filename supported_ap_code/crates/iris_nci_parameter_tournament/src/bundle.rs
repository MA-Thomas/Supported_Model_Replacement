use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use directed_round_robin_organizer::input::{
    BUNDLE_SCHEMA_NAME, BUNDLE_SCHEMA_VERSION, BundleManifest, EvaluationManifest, HashedPath,
    PortableEvaluationHashes, compute_bundle_content_hash,
};
use directed_round_robin_organizer::system::{SystemRecord, SystemRegistry};
use directed_round_robin_organizer::{MetricKind, load_bundle, load_bundle_for_hashing};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::candidates::{
    Candidate, CandidateSelection, MetricBranch, load_metric_rows, select_candidates, write_scan,
};
use crate::config::{MODELS, ModelConfig, PipelineConfig};
use crate::io::{collect_file_hashes, sha256_file, stage_path, write_json, write_json_new};
use crate::tensor::{Observation, ScoredModel, score_model_candidates};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PreparedBranch {
    pub model_id: String,
    pub metric: MetricBranch,
    pub candidate_count: usize,
    pub bundle_content_hash: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PrepareSummary {
    pub schema_version: u32,
    pub output: PathBuf,
    pub config_sha256: String,
    pub branches: Vec<PreparedBranch>,
}

#[derive(Serialize)]
struct CandidateRoster<'a> {
    schema_version: u32,
    model_id: &'a str,
    metric: MetricBranch,
    candidate_count: usize,
    ranking_rule: &'static str,
    diversity_key: [&'static str; 4],
    candidates: &'a [Candidate],
}

pub fn prepare(config_path: &Path, output: &Path) -> Result<PrepareSummary> {
    let config_path = config_path.canonicalize()?;
    let config = PipelineConfig::load(&config_path)?;
    let config_sha256 = sha256_file(&config_path)?;
    let stage = stage_path(output)?;
    let result = (|| {
        let branches = MODELS
            .par_iter()
            .map(|model_id| {
                prepare_model(
                    &stage,
                    &config,
                    &config_sha256,
                    model_id,
                    &config.models[*model_id],
                )
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let manifest = json!({
            "schema_version": 1,
            "config": config_path,
            "config_sha256": config_sha256,
            "candidate_count_per_branch": config.candidate_count,
            "models": MODELS,
            "metrics": ["pr", "roc"],
            "branches": branches,
            "files": collect_file_hashes(&stage)?,
        });
        write_json_new(&stage.join("manifest.json"), &manifest)?;
        fs::rename(&stage, output)?;
        Ok(PrepareSummary {
            schema_version: 1,
            output: output.to_owned(),
            config_sha256,
            branches,
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

fn prepare_model(
    stage: &Path,
    config: &PipelineConfig,
    config_sha256: &str,
    model_id: &str,
    model: &ModelConfig,
) -> Result<Vec<PreparedBranch>> {
    let rows = load_metric_rows(&model.metrics.path)?;
    let selections = MetricBranch::ALL
        .into_iter()
        .map(|branch| {
            Ok((
                branch,
                select_candidates(model_id, branch, &rows, config.candidate_count)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let all_candidates = selections
        .values()
        .flat_map(|selection| selection.selected.iter().cloned())
        .collect::<Vec<_>>();
    let scored = score_model_candidates(model, &all_candidates, config.log_epsilon)
        .with_context(|| format!("scoring candidates for {model_id}"))?;
    selections
        .into_iter()
        .map(|(branch, selection)| {
            build_branch(
                stage,
                config,
                config_sha256,
                model_id,
                model,
                branch,
                &selection,
                &scored,
            )
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn build_branch(
    stage: &Path,
    config: &PipelineConfig,
    config_sha256: &str,
    model_id: &str,
    model: &ModelConfig,
    branch: MetricBranch,
    selection: &CandidateSelection,
    scored: &ScoredModel,
) -> Result<PreparedBranch> {
    let branch_root = stage.join(model_id).join(branch.id());
    fs::create_dir_all(&branch_root)?;
    write_scan(&branch_root.join("candidate_scan.csv"), &selection.scan)?;
    write_json_new(
        &branch_root.join("candidate_roster.json"),
        &CandidateRoster {
            schema_version: 1,
            model_id,
            metric: branch,
            candidate_count: selection.selected.len(),
            ranking_rule: "primary_metric_desc_regime_idx_asc",
            diversity_key: ["d_pos", "d_neg", "M", "N"],
            candidates: &selection.selected,
        },
    )?;
    let bundle = branch_root.join("bundle");
    fs::create_dir_all(bundle.join("evaluations/nci"))?;
    let registry = registry(model_id, model, branch, &selection.selected)?;
    let mut spec = config.tournament_spec(branch.id())?;
    spec.annotations
        .insert("component_model_id".into(), json!(model_id));
    spec.annotations
        .insert("candidate_count".into(), json!(selection.selected.len()));
    spec.annotations
        .insert("config_sha256".into(), json!(config_sha256));
    registry.validate(&spec).map_err(anyhow::Error::msg)?;
    write_json(&bundle.join("systems.json"), &registry)?;
    write_json(&bundle.join("tournament_spec.json"), &spec)?;
    write_endpoints(
        &bundle.join("evaluations/nci/endpoints.csv"),
        &scored.observations,
    )?;
    write_endpoint_identities(
        &bundle.join("evaluations/nci/endpoint_identities.csv"),
        &scored.observations,
    )?;
    write_scores(
        &bundle.join("evaluations/nci/scores.csv"),
        &registry,
        &selection.selected,
        scored,
    )?;
    write_json(
        &bundle.join("evaluations/nci/source_provenance.json"),
        &json!({
            "schema_version": 1,
            "evaluation_id": "nci",
            "component_model_id": model_id,
            "metric_branch": branch,
            "config_sha256": config_sha256,
            "metrics": model.metrics,
            "primary": model.primary,
            "query_q": model.query_q,
            "candidate_roster_sha256": sha256_file(&branch_root.join("candidate_roster.json"))?,
            "candidate_scan_sha256": sha256_file(&branch_root.join("candidate_scan.csv"))?,
            "endpoint_identities_sha256": sha256_file(&bundle.join("evaluations/nci/endpoint_identities.csv"))?,
            "score_formula": "log(Q * P_pos * P_neg * pi + epsilon)",
            "tau_policy": "per_observation_argmax_with_lowest_index_tie_rule",
            "log_epsilon": config.log_epsilon,
        }),
    )?;
    let evaluation = evaluation_manifest(&bundle, "nci")?;
    let manifest = finalize_bundle(
        &bundle,
        config.provider_identity.clone(),
        spec.metric,
        vec![evaluation],
    )?;
    Ok(PreparedBranch {
        model_id: model_id.to_owned(),
        metric: branch,
        candidate_count: selection.selected.len(),
        bundle_content_hash: manifest.bundle_content_hash,
    })
}

fn registry(
    model_id: &str,
    model: &ModelConfig,
    branch: MetricBranch,
    candidates: &[Candidate],
) -> Result<SystemRegistry> {
    let systems = candidates
        .iter()
        .map(|candidate| {
            let row = &candidate.parameters;
            Ok(SystemRecord {
                system_id: candidate.system_id.clone(),
                display_label: format!(
                    "{} {} rank {}: d+= {}, d-= {}, M={}, N={}, s+= {}, s-= {}",
                    model.display_label,
                    branch.id().to_uppercase(),
                    candidate.selected_rank,
                    row.d_pos,
                    row.d_neg,
                    row.m,
                    row.n,
                    row.steepness_pos,
                    row.steepness_neg,
                ),
                score_column: format!("score_{}", candidate.system_id),
                annotations: BTreeMap::from([
                    ("provider_domain".into(), json!("IRIS_NCI")),
                    ("component_model_id".into(), json!(model_id)),
                    ("metric_branch".into(), json!(branch)),
                    ("selected_rank".into(), json!(candidate.selected_rank)),
                    ("regime_idx".into(), json!(row.regime_idx)),
                    ("geometry_idx".into(), json!(row.geometry_idx)),
                    ("mn_idx".into(), json!(row.mn_idx)),
                    ("d_pos".into(), json!(row.d_pos)),
                    ("d_neg".into(), json!(row.d_neg)),
                    ("steepness_pos".into(), json!(row.steepness_pos)),
                    ("steepness_neg".into(), json!(row.steepness_neg)),
                    ("M".into(), json!(row.m)),
                    ("N".into(), json!(row.n)),
                    ("empirical_pr_auc".into(), json!(row.pr_auc)),
                    ("empirical_roc_auc".into(), json!(row.roc_auc)),
                ]),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(SystemRegistry {
        schema_version: 2,
        systems,
    })
}

fn endpoint_id(observation: &Observation) -> String {
    format!("nci_obs_{:06}", observation.obs_idx)
}

fn write_endpoints(path: &Path, observations: &[Observation]) -> Result<()> {
    let mut writer = csv::Writer::from_path(path)?;
    writer.write_record(["endpoint_id", "label"])?;
    for observation in observations {
        writer.write_record([
            endpoint_id(observation),
            u8::from(observation.label).to_string(),
        ])?;
    }
    writer.flush()?;
    Ok(())
}

fn write_endpoint_identities(path: &Path, observations: &[Observation]) -> Result<()> {
    let mut writer = csv::Writer::from_path(path)?;
    writer.write_record([
        "endpoint_id",
        "obs_idx",
        "patient_id",
        "env_id",
        "peptide",
        "hla",
        "gene",
        "cancer_type",
        "wt_mt_group_id",
        "label",
    ])?;
    for row in observations {
        writer.write_record([
            endpoint_id(row),
            row.obs_idx.to_string(),
            row.patient_id.clone(),
            row.env_id.to_string(),
            row.peptide.clone(),
            row.hla.clone(),
            row.gene.clone(),
            row.cancer_type.clone(),
            row.wt_mt_group_id.clone(),
            u8::from(row.label).to_string(),
        ])?;
    }
    writer.flush()?;
    Ok(())
}

fn write_scores(
    path: &Path,
    registry: &SystemRegistry,
    candidates: &[Candidate],
    scored: &ScoredModel,
) -> Result<()> {
    let by_id: BTreeMap<_, _> = candidates
        .iter()
        .map(|candidate| (candidate.system_id.as_str(), candidate))
        .collect();
    let systems = registry.sorted_systems();
    let mut writer = csv::Writer::from_path(path)?;
    let mut header = vec!["endpoint_id".to_owned()];
    header.extend(systems.iter().map(|system| system.score_column.clone()));
    writer.write_record(header)?;
    for observation in &scored.observations {
        let mut row = vec![endpoint_id(observation)];
        for system in &systems {
            let candidate = by_id[system.system_id.as_str()];
            let value = scored
                .scores_by_regime
                .get(&candidate.parameters.regime_idx)
                .context("selected score vector was not materialized")?[observation.obs_idx];
            if !value.is_finite() {
                bail!("nonfinite tournament score");
            }
            row.push(value.to_string());
        }
        writer.write_record(row)?;
    }
    writer.flush()?;
    Ok(())
}

fn hashed_path(root: &Path, relative: PathBuf) -> Result<HashedPath> {
    Ok(HashedPath {
        sha256: sha256_file(&root.join(&relative))?,
        path: relative,
    })
}

fn evaluation_manifest(root: &Path, evaluation_id: &str) -> Result<EvaluationManifest> {
    let base = PathBuf::from("evaluations").join(evaluation_id);
    Ok(EvaluationManifest {
        evaluation_id: evaluation_id.to_owned(),
        endpoints: hashed_path(root, base.join("endpoints.csv"))?,
        scores: hashed_path(root, base.join("scores.csv"))?,
        source_provenance: hashed_path(root, base.join("source_provenance.json"))?,
    })
}

fn finalize_bundle(
    root: &Path,
    provider_identity: String,
    metric: MetricKind,
    evaluations: Vec<EvaluationManifest>,
) -> Result<BundleManifest> {
    let mut manifest = BundleManifest {
        schema_name: BUNDLE_SCHEMA_NAME.into(),
        schema_version: BUNDLE_SCHEMA_VERSION,
        metric,
        bundle_creation_time: OffsetDateTime::now_utc().format(&Rfc3339)?,
        systems: hashed_path(root, "systems.json".into())?,
        tournament_spec: hashed_path(root, "tournament_spec.json".into())?,
        evaluations,
        score_provider_identity: provider_identity,
        minimum_organizer_schema_version: BUNDLE_SCHEMA_VERSION,
        bundle_content_hash: String::new(),
    };
    write_json(&root.join("bundle_manifest.json"), &manifest)?;
    let loaded = load_bundle_for_hashing(root).map_err(anyhow::Error::msg)?;
    let portable = loaded
        .evaluations
        .values()
        .map(|evaluation| PortableEvaluationHashes {
            evaluation_id: evaluation.evaluation_id.clone(),
            label_vector_hash: evaluation.label_vector_hash.clone(),
            score_vector_hashes: evaluation.score_vector_hashes.clone(),
        })
        .collect::<Vec<_>>();
    manifest.bundle_content_hash =
        compute_bundle_content_hash(&loaded.spec, &loaded.registry, &portable)
            .map_err(anyhow::Error::msg)?;
    write_json(&root.join("bundle_manifest.json"), &manifest)?;
    load_bundle(root).map_err(anyhow::Error::msg)?;
    Ok(manifest)
}

pub fn audit_prepared(root: &Path) -> Result<()> {
    let manifest: serde_json::Value = crate::io::read_json(&root.join("manifest.json"))?;
    let config_path = manifest["config"]
        .as_str()
        .context("prepared manifest config path")?;
    let config_sha256 = manifest["config_sha256"]
        .as_str()
        .context("prepared manifest config_sha256")?;
    if sha256_file(Path::new(config_path))? != config_sha256 {
        bail!("prepared package configuration no longer matches its manifest")
    }
    let expected_files = manifest["files"]
        .as_object()
        .context("prepared manifest files")?;
    let actual = collect_file_hashes(root)?;
    if expected_files.len() != actual.len() {
        bail!("prepared file count differs from manifest");
    }
    for (path, hash) in expected_files {
        if actual.get(path).map(String::as_str) != hash.as_str() {
            bail!("prepared artifact hash mismatch: {path}");
        }
    }
    for model in MODELS {
        for branch in MetricBranch::ALL {
            load_bundle(&root.join(model).join(branch.id()).join("bundle"))
                .map_err(anyhow::Error::msg)?;
        }
    }
    Ok(())
}
