use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use directed_round_robin_organizer::input::{
    BUNDLE_SCHEMA_NAME, BUNDLE_SCHEMA_VERSION, BundleManifest, EvaluationManifest, HashedPath,
};
use directed_round_robin_organizer::spec::{MetricKind, SelectionStrategy, TournamentSpec};
use directed_round_robin_organizer::system::{SystemRecord, SystemRegistry};
use directed_round_robin_organizer::{load_bundle, load_bundle_for_hashing};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::contract::{BRANCHES, COHORTS, L2Spec, MODELS, PipelineConfig};
use crate::io::{read_json, sha256_file, stage_path, write_json};
use crate::transfer::{input_mapping, validate_transfer_package_against_config};

const POLICIES: [&str; 2] = ["pdac_only", "all_contexts_equal_weight"];

#[derive(Clone, Debug)]
struct AuthorityRow {
    endpoint_id: String,
    label: bool,
    identity: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
struct SelectedParameter {
    fields: BTreeMap<String, String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionManifestContract {
    analysis: String,
    schema_version: u32,
    status: String,
    implementation: String,
    source_root: PathBuf,
    base_bundle_root: PathBuf,
    transfer_manifest_sha256: String,
    base_bundle_content_hashes: BTreeMap<String, String>,
    formula: Value,
    selection: Value,
    grid: Value,
    label_contracts: Value,
    selected_parameters: Value,
    source_files: BTreeMap<String, String>,
    software_versions: Value,
    executable: Value,
    base_bundle_files: BTreeMap<String, String>,
    generated_files: BTreeMap<String, String>,
}

type SelectionKey = (String, String, String);
type AdaptiveScoreKey = (String, String, String);
type AdaptiveScores = BTreeMap<AdaptiveScoreKey, BTreeMap<String, (bool, f64)>>;
type SelectionData = (
    BTreeMap<SelectionKey, SelectedParameter>,
    AdaptiveScores,
    BTreeMap<String, String>,
);

pub fn build_fixed_bundles(
    config_path: &Path,
    transfer_package: &Path,
    output: &Path,
) -> Result<Value> {
    if output.exists() {
        bail!("output already exists: {}", output.display());
    }
    let config_path = config_path.canonicalize()?;
    let config = PipelineConfig::load(&config_path)?;
    let transfer_manifest =
        validate_transfer_package_against_config(transfer_package, &config_path)?;
    let transfer_manifest_hash = sha256_file(&transfer_package.join("manifest.json"))?;
    let input_manifest: Value = read_json(&config.input_bundle.join("manifest.json"))?;
    let stage = stage_path(output)?;
    let result = (|| {
        std::fs::create_dir_all(&stage)?;
        let mut reports = Map::new();
        for branch in BRANCHES {
            let bundle = stage.join(branch);
            let report = build_fixed_metric_bundle(
                &config,
                transfer_package,
                &transfer_manifest_hash,
                &input_manifest,
                branch,
                &bundle,
            )?;
            reports.insert(branch.to_owned(), report);
        }
        std::fs::rename(&stage, output)?;
        Ok(json!({
            "status": "built",
            "output": output,
            "transfer_validation": {
                "status": "pass",
                "manifest_sha256": transfer_manifest_hash,
                "config_sha256": transfer_manifest.config_sha256(),
                "input_manifest_sha256": transfer_manifest.input_manifest_sha256(),
            },
            "metrics": reports,
        }))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

fn build_fixed_metric_bundle(
    config: &PipelineConfig,
    transfer_package: &Path,
    transfer_manifest_hash: &str,
    input_manifest: &Value,
    branch: &str,
    output: &Path,
) -> Result<Value> {
    std::fs::create_dir_all(output)?;
    let registry = fixed_registry(config, branch)?;
    let spec = tournament_spec(config, branch, &registry)?;
    write_json(&output.join("systems.json"), &registry)?;
    write_json(&output.join("tournament_spec.json"), &spec)?;

    let mut evaluations = Vec::new();
    for cohort_id in COHORTS {
        let cohort = &config.cohorts[cohort_id];
        let mapping = input_mapping(
            &config.input_bundle,
            input_manifest,
            &cohort.input_prefix,
            "full",
        )?;
        let authority = load_authority(cohort_id, cohort, &mapping)?;
        let directory = output.join("evaluations").join(cohort_id);
        std::fs::create_dir_all(&directory)?;
        write_authority(&directory, cohort, &authority)?;
        let source_records = write_fixed_scores(
            &directory.join("scores.csv"),
            transfer_package,
            cohort_id,
            branch,
            &authority,
            &registry,
        )?;
        let identity_hash = sha256_file(&directory.join("endpoint_identities.csv"))?;
        write_json(
            &directory.join("source_provenance.json"),
            &json!({
                "schema_version": 3,
                "evaluation_id": cohort_id,
                "analysis_unit_fields": cohort.endpoint_fields,
                "label_contract": {
                    "response_field": cohort.response_field,
                    "label_field": cohort.label_field,
                    "threshold": cohort.threshold,
                    "comparison_operator": cohort.comparison_operator.map(crate::contract::ComparisonOperator::symbol),
                },
                "measurement_error_policy": cohort.measurement_error_policy,
                "accepted_measurement_error": config.tournament.accepted_measurement_error,
                "covid_spike_label_specification": if cohort_id == "covid_spike" { Some(&config.tournament.covid_spike_label_specification) } else { None },
                "tournament_label_authority": "Rust full-roster input package mapping",
                "mapping": mapping,
                "mapping_sha256": sha256_file(&mapping)?,
                "input_manifest_sha256": config.input_manifest_sha256,
                "transfer_manifest_sha256": transfer_manifest_hash,
                "endpoint_identity_table": {"path": "endpoint_identities.csv", "sha256": identity_hash},
                "transfer_records": source_records,
            }),
        )?;
        evaluations.push(evaluation_manifest(output, cohort_id)?);
    }
    let manifest = finalize_manifest(
        output,
        config.provider_identity.clone(),
        spec.metric,
        evaluations,
    )?;
    if registry.systems.len() != 60 {
        bail!(
            "fixed bundle has {} systems instead of 60",
            registry.systems.len()
        );
    }
    Ok(json!({
        "metric": manifest.metric,
        "systems": registry.systems.len(),
        "evaluations": 3,
        "bundle_content_hash": manifest.bundle_content_hash,
    }))
}

fn fixed_registry(config: &PipelineConfig, branch: &str) -> Result<SystemRegistry> {
    let mut systems = Vec::with_capacity(60);
    for model_id in MODELS {
        let model = &config.models[model_id];
        for l2 in &config.fixed_l2 {
            let l2_id = l2.id();
            let system_id = format!("{model_id}__{l2_id}");
            systems.push(SystemRecord {
                display_label: format!("{} / {l2_id}", model.display_label),
                score_column: format!("score_{system_id}"),
                system_id,
                annotations: BTreeMap::from([
                    ("provider_domain".into(), json!("IRIS")),
                    ("component_model_id".into(), json!(model_id)),
                    (
                        "metric_branch_id".into(),
                        json!(format!("{model_id}_{branch}_branch")),
                    ),
                    ("l2_aggregation_id".into(), json!(l2_id)),
                    ("l2_aggregation_spec".into(), serde_json::to_value(l2)?),
                    ("nci_summary_sha256".into(), json!(model.nci_summary_sha256)),
                ]),
            });
        }
    }
    Ok(SystemRegistry {
        schema_version: 2,
        systems,
    })
}

fn tournament_spec(
    config: &PipelineConfig,
    branch: &str,
    registry: &SystemRegistry,
) -> Result<TournamentSpec> {
    let metric = metric_kind(branch)?;
    let l2_ids: Vec<_> = config.fixed_l2.iter().map(L2Spec::id).collect();
    let spec = TournamentSpec {
        schema_version: 2,
        metric,
        verdict_field: if branch == "pr" {
            "forward.staged_verdict".into()
        } else {
            "baseline.verdict".into()
        },
        evidence_policy: config.tournament.shared.evidence_policy.clone(),
        computational_design: config.tournament.shared.computational_design.clone(),
        reference_assessment: config.tournament.shared.reference_assessment.clone(),
        master_seed: config.tournament.shared.master_seed,
        seed_derivation_version: 1,
        evaluations: COHORTS.into_iter().map(str::to_owned).collect(),
        conjunction_rule: "all_evaluations".into(),
        graph_maximality_rule: "source_strongly_connected_components".into(),
        selection_rule: "source_scc_maximal_vertices".into(),
        selection_strategy: SelectionStrategy::CandidateConservative,
        pr_cnap: (branch == "pr").then(|| config.tournament.pr.clone()),
        auroc: (branch == "roc").then(|| config.tournament.roc.clone()),
        annotations: BTreeMap::from([
            ("provider_domain".into(), json!("IRIS")),
            (
                "scientific_contract".into(),
                json!({
                    "l2_aggregation_ids": l2_ids,
                    "full_roster_floor": (1e-12_f64).ln(),
                    "candidate_identity": "endpoint+nmer+normalized-HLA",
                    "covid_spike_label_specification": config.tournament.covid_spike_label_specification,
                    "evaluation_label_contracts": config.evaluation_label_contracts(),
                }),
            ),
        ]),
        operational_tie_break: None,
    };
    spec.validate().map_err(anyhow::Error::msg)?;
    registry.validate(&spec).map_err(anyhow::Error::msg)?;
    Ok(spec)
}

fn metric_kind(branch: &str) -> Result<MetricKind> {
    match branch {
        "pr" => Ok(MetricKind::PrCnap),
        "roc" => Ok(MetricKind::Auroc),
        _ => bail!("unknown metric branch {branch}"),
    }
}

fn load_authority(
    cohort_id: &str,
    cohort: &crate::contract::CohortConfig,
    mapping: &Path,
) -> Result<Vec<AuthorityRow>> {
    let mut reader = csv::Reader::from_path(mapping)?;
    let headers = reader.headers()?.clone();
    let index = |name: &str| {
        headers
            .iter()
            .position(|header| header == name)
            .with_context(|| format!("{} lacks column {name}", mapping.display()))
    };
    let field_indices: Vec<_> = cohort
        .endpoint_fields
        .iter()
        .map(|field| index(field))
        .collect::<Result<_>>()?;
    let label_index = cohort.label_field.as_deref().map(index).transpose()?;
    let response_index = cohort.response_field.as_deref().map(index).transpose()?;
    let mut rows = BTreeMap::<String, AuthorityRow>::new();
    for record in reader.records() {
        let record = record?;
        let mut identity = BTreeMap::new();
        let mut ordered = Vec::new();
        for (field, &column) in cohort.endpoint_fields.iter().zip(&field_indices) {
            let value = record.get(column).context("short mapping row")?.trim();
            if value.is_empty() {
                bail!("blank {field} in {}", mapping.display());
            }
            identity.insert(field.clone(), value.to_owned());
            ordered.push([field.clone(), value.to_owned()]);
        }
        let label = if let Some(column) = label_index {
            match record.get(column).context("short mapping row")?.trim() {
                "0" => false,
                "1" => true,
                other => bail!("invalid binary label {other:?} in {}", mapping.display()),
            }
        } else {
            let value: f64 = record
                .get(response_index.context("missing response index")?)
                .context("short mapping row")?
                .trim()
                .parse()?;
            let threshold = cohort.threshold.context("missing threshold")?;
            match cohort
                .comparison_operator
                .context("missing comparison operator")?
            {
                crate::contract::ComparisonOperator::GreaterThan => value > threshold,
                crate::contract::ComparisonOperator::GreaterThanOrEqual => value >= threshold,
            }
        };
        let digest = hex_digest(&serde_json::to_vec(&ordered)?);
        let endpoint_id = format!("{cohort_id}.{digest}");
        let candidate = AuthorityRow {
            endpoint_id: endpoint_id.clone(),
            label,
            identity,
        };
        if let Some(previous) = rows.get(&endpoint_id)
            && (previous.label != candidate.label || previous.identity != candidate.identity)
        {
            bail!("conflicting mapping rows for endpoint {endpoint_id}");
        }
        rows.insert(endpoint_id, candidate);
    }
    let rows: Vec<_> = rows.into_values().collect();
    let positives = rows.iter().filter(|row| row.label).count();
    if positives == 0 || positives == rows.len() {
        bail!("{cohort_id} authority has only one label class");
    }
    Ok(rows)
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn write_authority(
    directory: &Path,
    cohort: &crate::contract::CohortConfig,
    rows: &[AuthorityRow],
) -> Result<()> {
    let mut endpoint_writer = csv::Writer::from_path(directory.join("endpoints.csv"))?;
    endpoint_writer.write_record(["endpoint_id", "label"])?;
    let mut identity_writer = csv::Writer::from_path(directory.join("endpoint_identities.csv"))?;
    let mut identity_header = vec!["endpoint_id".to_owned()];
    identity_header.extend(cohort.endpoint_fields.iter().cloned());
    identity_writer.write_record(&identity_header)?;
    for row in rows {
        endpoint_writer.serialize((&row.endpoint_id, u8::from(row.label)))?;
        let mut record = vec![row.endpoint_id.clone()];
        for field in &cohort.endpoint_fields {
            record.push(row.identity[field].clone());
        }
        identity_writer.write_record(record)?;
    }
    endpoint_writer.flush()?;
    identity_writer.flush()?;
    Ok(())
}

fn write_fixed_scores(
    path: &Path,
    transfer_package: &Path,
    cohort_id: &str,
    branch: &str,
    authority: &[AuthorityRow],
    registry: &SystemRegistry,
) -> Result<Vec<Value>> {
    let authority_by_identity: BTreeMap<Vec<String>, &AuthorityRow> = authority
        .iter()
        .map(|row| (row.identity.values().cloned().collect(), row))
        .collect();
    let mut scores = BTreeMap::<String, BTreeMap<String, f64>>::new();
    let mut sources = Vec::new();
    for model_id in MODELS {
        let directory = transfer_package.join(cohort_id).join(model_id).join(branch);
        let predictions = directory.join("long_peptide_predictions.csv");
        let summary = directory.join("summary.json");
        let mut reader = csv::Reader::from_path(&predictions)?;
        let headers = reader.headers()?.clone();
        let column = |name: &str| {
            headers
                .iter()
                .position(|header| header == name)
                .with_context(|| format!("{} lacks {name}", predictions.display()))
        };
        let l2 = column("l2_variant")?;
        let score = column("score")?;
        let label = column("label")?;
        let identity_columns: Vec<_> = authority[0]
            .identity
            .keys()
            .map(|field| column(field))
            .collect::<Result<_>>()?;
        let mut observed = BTreeSet::new();
        for record in reader.records() {
            let record = record?;
            let l2_id = record.get(l2).context("short prediction row")?;
            let system_id = format!("{model_id}__{l2_id}");
            if !registry
                .systems
                .iter()
                .any(|system| system.system_id == system_id)
            {
                bail!("prediction declares unknown L2 system {system_id}");
            }
            let identity: Vec<_> = identity_columns
                .iter()
                .map(|&index| record.get(index).unwrap_or("").trim().to_owned())
                .collect();
            let authority_row = authority_by_identity
                .get(&identity)
                .with_context(|| format!("prediction endpoint is outside {cohort_id} authority"))?;
            let embedded = record.get(label).context("short prediction row")?;
            if embedded != if authority_row.label { "1" } else { "0" } {
                bail!(
                    "prediction label differs from authority for {}",
                    authority_row.endpoint_id
                );
            }
            let value: f64 = record.get(score).context("short prediction row")?.parse()?;
            if !value.is_finite() {
                bail!("nonfinite fixed-L2 score");
            }
            if !observed.insert((system_id.clone(), authority_row.endpoint_id.clone())) {
                bail!("duplicate fixed-L2 score");
            }
            scores
                .entry(system_id)
                .or_default()
                .insert(authority_row.endpoint_id.clone(), value);
        }
        sources.push(json!({
            "model_id": model_id,
            "summary": summary,
            "summary_sha256": sha256_file(&summary)?,
            "predictions": predictions,
            "predictions_sha256": sha256_file(&predictions)?,
        }));
    }
    let mut sorted_systems: Vec<_> = registry.systems.iter().collect();
    sorted_systems.sort_by(|left, right| left.system_id.cmp(&right.system_id));
    let mut writer = csv::Writer::from_path(path)?;
    let mut header = vec!["endpoint_id".to_owned()];
    header.extend(
        sorted_systems
            .iter()
            .map(|system| system.score_column.clone()),
    );
    writer.write_record(header)?;
    for row in authority {
        let mut output = vec![row.endpoint_id.clone()];
        for system in &sorted_systems {
            let column = scores
                .get(&system.system_id)
                .and_then(|values| values.get(&row.endpoint_id))
                .with_context(|| {
                    format!("missing score for {}/{}", system.system_id, row.endpoint_id)
                })?;
            output.push(column.to_string());
        }
        writer.write_record(output)?;
    }
    writer.flush()?;
    Ok(sources)
}

fn evaluation_manifest(root: &Path, cohort_id: &str) -> Result<EvaluationManifest> {
    let base = PathBuf::from("evaluations").join(cohort_id);
    Ok(EvaluationManifest {
        evaluation_id: cohort_id.to_owned(),
        endpoints: hashed_path(root, base.join("endpoints.csv"))?,
        scores: hashed_path(root, base.join("scores.csv"))?,
        source_provenance: hashed_path(root, base.join("source_provenance.json"))?,
    })
}

fn hashed_path(root: &Path, relative: PathBuf) -> Result<HashedPath> {
    Ok(HashedPath {
        sha256: sha256_file(&root.join(&relative))?,
        path: relative,
    })
}

fn finalize_manifest(
    root: &Path,
    provider: String,
    metric: MetricKind,
    evaluations: Vec<EvaluationManifest>,
) -> Result<BundleManifest> {
    let mut manifest = BundleManifest {
        schema_name: BUNDLE_SCHEMA_NAME.into(),
        schema_version: BUNDLE_SCHEMA_VERSION,
        metric,
        bundle_creation_time: OffsetDateTime::now_utc().format(&Rfc3339)?,
        systems: hashed_path(root, PathBuf::from("systems.json"))?,
        tournament_spec: hashed_path(root, PathBuf::from("tournament_spec.json"))?,
        evaluations,
        score_provider_identity: provider,
        minimum_organizer_schema_version: BUNDLE_SCHEMA_VERSION,
        bundle_content_hash: String::new(),
    };
    write_json(&root.join("bundle_manifest.json"), &manifest)?;
    let loaded = load_bundle_for_hashing(root).map_err(anyhow::Error::msg)?;
    manifest.bundle_content_hash = loaded.manifest.bundle_content_hash;
    // load_bundle_for_hashing returns the computed identity through the loaded
    // manifest only when declared; compute it from its validated structures.
    manifest.bundle_content_hash =
        directed_round_robin_organizer::input::compute_bundle_content_hash(
            &loaded.spec,
            &loaded.registry,
            &loaded
                .evaluations
                .values()
                .map(
                    |evaluation| directed_round_robin_organizer::input::PortableEvaluationHashes {
                        evaluation_id: evaluation.evaluation_id.clone(),
                        label_vector_hash: evaluation.label_vector_hash.clone(),
                        score_vector_hashes: evaluation.score_vector_hashes.clone(),
                    },
                )
                .collect::<Vec<_>>(),
        )
        .map_err(anyhow::Error::msg)?;
    write_json(&root.join("bundle_manifest.json"), &manifest)?;
    load_bundle(root).map_err(anyhow::Error::msg)?;
    Ok(manifest)
}

pub fn build_combined_bundles(
    base_bundles: &Path,
    selection_root: &Path,
    output: &Path,
) -> Result<Value> {
    if output.exists() {
        bail!("output already exists: {}", output.display());
    }
    let (parameters, scores, hashes) = load_selection(selection_root)?;
    validate_combined_inputs(base_bundles, &hashes)?;
    let stage = stage_path(output)?;
    let result = (|| {
        std::fs::create_dir_all(&stage)?;
        let mut reports = Map::new();
        for branch in BRANCHES {
            let report = build_combined_metric(
                &base_bundles.join(branch),
                selection_root,
                branch,
                &parameters,
                &scores,
                &hashes,
                &stage.join(branch),
            )?;
            reports.insert(branch.to_owned(), report);
        }
        std::fs::rename(&stage, output)?;
        Ok(json!({"status": "built", "output": output, "metrics": reports}))
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

fn validate_combined_inputs(
    base_bundles: &Path,
    selection_hashes: &BTreeMap<String, String>,
) -> Result<()> {
    let expected_transfer = &selection_hashes["transfer_manifest"];
    for branch in BRANCHES {
        let root = base_bundles.join(branch);
        let bundle = load_bundle(&root).map_err(anyhow::Error::msg)?;
        if bundle.manifest.bundle_content_hash != selection_hashes[&format!("base_bundle_{branch}")]
        {
            bail!("supplied {branch} base bundle differs from the bundle used for selection");
        }
        for evaluation in &bundle.manifest.evaluations {
            let provenance: Value = read_json(&root.join(&evaluation.source_provenance.path))?;
            if provenance["transfer_manifest_sha256"].as_str() != Some(expected_transfer) {
                bail!(
                    "base bundle transfer provenance differs from adaptive selection for {branch}/{}",
                    evaluation.evaluation_id
                );
            }
        }
    }
    Ok(())
}

fn load_selection(root: &Path) -> Result<SelectionData> {
    let manifest_path = root.join("selection_manifest.json");
    let manifest: SelectionManifestContract = read_json(&manifest_path)?;
    if manifest.schema_version != 4
        || manifest.analysis != "component-metric-specific self-gated Hill-q L2 parameter selection"
        || manifest.status != "post_hoc_model_development"
        || manifest.implementation != "rust_native"
        || manifest.formula["score"].as_str() != Some("S solves S = m + C_q * sigmoid((c-S)/kappa)")
        || manifest.formula["floor_definition"].as_str() != Some("ln(1e-12)")
        || manifest.formula["empty_roster_score"]
            .as_f64()
            .map(f64::to_bits)
            != Some((1e-12_f64).ln().to_bits())
        || !manifest.selection.is_object()
        || !manifest.grid.is_object()
        || !manifest.label_contracts.is_object()
        || !manifest.selected_parameters.is_array()
        || !manifest.software_versions.is_object()
        || !manifest.executable.is_object()
        || manifest.selection["pr_selection_computational_design"]["replications"].as_u64()
            != Some(200)
        || manifest.selection["pr_selection_computational_design"]["computational_order"].as_u64()
            != Some(2)
    {
        bail!("selection manifest has the wrong scientific contract");
    }
    if manifest
        .base_bundle_content_hashes
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != BRANCHES.into_iter().collect()
        || manifest.transfer_manifest_sha256.is_empty()
    {
        bail!("selection manifest has incomplete transfer or base-bundle provenance");
    }
    verify_recorded_hashes(&manifest.source_files)?;
    verify_recorded_hashes(&manifest.base_bundle_files)?;
    let recorded_transfer = manifest.source_root.join("manifest.json");
    if sha256_file(&recorded_transfer)? != manifest.transfer_manifest_sha256 {
        bail!("selection transfer-manifest hash does not resolve");
    }
    for branch in BRANCHES {
        let recorded_bundle = manifest.base_bundle_root.join(branch);
        let loaded = load_bundle(&recorded_bundle).map_err(anyhow::Error::msg)?;
        if loaded.manifest.bundle_content_hash != manifest.base_bundle_content_hashes[branch] {
            bail!("selection base-bundle content hash does not resolve for {branch}");
        }
    }
    verify_generated_hashes(root, &manifest.generated_files)?;
    let mut hashes = BTreeMap::from([
        ("manifest".into(), sha256_file(&manifest_path)?),
        (
            "transfer_manifest".into(),
            manifest.transfer_manifest_sha256.clone(),
        ),
        (
            "base_bundle_pr".into(),
            manifest.base_bundle_content_hashes["pr"].clone(),
        ),
        (
            "base_bundle_roc".into(),
            manifest.base_bundle_content_hashes["roc"].clone(),
        ),
    ]);
    for name in ["selected_parameters.csv", "selected_endpoint_scores.csv"] {
        let path = root.join(name);
        let digest = sha256_file(&path)?;
        if manifest.generated_files.get(name) != Some(&digest) {
            bail!("selection artifact hash mismatch for {name}");
        }
        hashes.insert(name.trim_end_matches(".csv").into(), digest);
    }
    let mut parameters = BTreeMap::new();
    let parameter_path = root.join("selected_parameters.csv");
    let mut reader = csv::Reader::from_path(&parameter_path)?;
    let headers = reader.headers()?.clone();
    for record in reader.records() {
        let record = record?;
        let fields: BTreeMap<_, _> = headers
            .iter()
            .zip(record.iter())
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect();
        for required in [
            "selection_policy",
            "model",
            "branch",
            "selection_metric",
            "q_id",
            "q",
            "c",
            "kappa",
            "c_at_boundary",
            "kappa_at_boundary",
            "system_id",
        ] {
            if !fields.contains_key(required) {
                bail!("selected parameter table lacks {required}");
            }
        }
        let branch = fields["branch"].as_str();
        let expected_metric = if branch == "pr" {
            "paired_supported_cnap_vs_model_matched_max"
        } else {
            "auroc"
        };
        if fields["selection_metric"] != expected_metric {
            bail!("selected parameter has the wrong branch metric");
        }
        if branch == "pr" {
            for required in [
                "reference_system_id",
                "pdac_cnap_staged_supported",
                "covid_spike_cnap_staged_supported",
                "covid_nonspike_cnap_staged_supported",
            ] {
                if !fields.contains_key(required) {
                    bail!("PR selected parameter table lacks {required}");
                }
            }
            if fields["reference_system_id"] != format!("{}__max", fields["model"])
                || fields["pdac_cnap_staged_supported"] != "True"
                || fields["selection_policy"] == "all_contexts_equal_weight"
                    && (fields["covid_spike_cnap_staged_supported"] != "True"
                        || fields["covid_nonspike_cnap_staged_supported"] != "True")
            {
                bail!("PR selected parameter lacks its required staged-CNAP support");
            }
        }
        let key = (
            fields["selection_policy"].clone(),
            fields["branch"].clone(),
            fields["model"].clone(),
        );
        if fields["c_at_boundary"] != "False" || fields["kappa_at_boundary"] != "False" {
            bail!("selected parameter is on a grid boundary: {key:?}");
        }
        let kappa: f64 = fields["kappa"].parse()?;
        if !kappa.is_finite() || kappa <= 0.0 || !fields["c"].parse::<f64>()?.is_finite() {
            bail!("invalid selected c/kappa");
        }
        let expected = adaptive_system_id(&key.0, &key.2)?;
        if fields["system_id"] != expected
            || parameters
                .insert(key, SelectedParameter { fields })
                .is_some()
        {
            bail!("invalid or duplicate selected parameter row");
        }
    }
    let expected: BTreeSet<_> = POLICIES
        .into_iter()
        .flat_map(|policy| {
            BRANCHES.into_iter().flat_map(move |branch| {
                MODELS
                    .into_iter()
                    .map(move |model| (policy.to_owned(), branch.to_owned(), model.to_owned()))
            })
        })
        .collect();
    if parameters.keys().cloned().collect::<BTreeSet<_>>() != expected {
        bail!("selected parameter table must contain exactly the twenty required rows");
    }

    let score_path = root.join("selected_endpoint_scores.csv");
    let mut reader = csv::Reader::from_path(&score_path)?;
    let headers = reader.headers()?.clone();
    let index = |name: &str| {
        headers
            .iter()
            .position(|header| header == name)
            .with_context(|| format!("selected scores lack {name}"))
    };
    let policy = index("selection_policy")?;
    let model = index("model")?;
    let branch = index("branch")?;
    let cohort = index("cohort")?;
    let endpoint = index("endpoint_id")?;
    let label = index("label")?;
    let score = index("self_gated_hillq_score")?;
    let mut scores = AdaptiveScores::new();
    for record in reader.records() {
        let record = record?;
        let key = (
            record[policy].to_owned(),
            record[model].to_owned(),
            format!("{}:{}", &record[branch], &record[cohort]),
        );
        let observed_label = match &record[label] {
            "0" => false,
            "1" => true,
            value => bail!("invalid adaptive label {value:?}"),
        };
        let observed_score: f64 = record[score].parse()?;
        if !observed_score.is_finite() {
            bail!("nonfinite adaptive endpoint score");
        }
        if scores
            .entry(key)
            .or_default()
            .insert(
                record[endpoint].to_owned(),
                (observed_label, observed_score),
            )
            .is_some()
        {
            bail!("duplicate adaptive endpoint score");
        }
    }
    let expected_score_scopes: BTreeSet<_> = POLICIES
        .into_iter()
        .flat_map(|policy| {
            MODELS.into_iter().flat_map(move |model| {
                BRANCHES.into_iter().flat_map(move |branch| {
                    COHORTS.into_iter().map(move |cohort| {
                        (
                            policy.to_owned(),
                            model.to_owned(),
                            format!("{branch}:{cohort}"),
                        )
                    })
                })
            })
        })
        .collect();
    if scores.keys().cloned().collect::<BTreeSet<_>>() != expected_score_scopes {
        bail!("adaptive endpoint scores must contain exactly the sixty required scopes");
    }
    Ok((parameters, scores, hashes))
}

fn verify_recorded_hashes(records: &BTreeMap<String, String>) -> Result<()> {
    if records.is_empty() {
        bail!("selection manifest contains an empty provenance hash registry");
    }
    for (path, expected) in records {
        if sha256_file(Path::new(path))? != *expected {
            bail!("selection provenance hash mismatch for {path}");
        }
    }
    Ok(())
}

fn verify_generated_hashes(root: &Path, records: &BTreeMap<String, String>) -> Result<()> {
    if records.is_empty() {
        bail!("selection manifest contains an empty generated-file registry");
    }
    for (relative, expected) in records {
        let path = root.join(relative);
        if sha256_file(&path)? != *expected {
            bail!("selection generated-file hash mismatch for {relative}");
        }
    }
    Ok(())
}

fn adaptive_system_id(policy: &str, model: &str) -> Result<String> {
    let suffix = match policy {
        "pdac_only" => "self_gated_hillq_pdac_selected",
        "all_contexts_equal_weight" => "self_gated_hillq_all_contexts_selected",
        _ => bail!("unknown adaptive selection policy {policy}"),
    };
    Ok(format!("{model}__{suffix}"))
}

fn build_combined_metric(
    base_root: &Path,
    selection_root: &Path,
    branch: &str,
    parameters: &BTreeMap<SelectionKey, SelectedParameter>,
    scores: &AdaptiveScores,
    hashes: &BTreeMap<String, String>,
    output: &Path,
) -> Result<Value> {
    let base = load_bundle(base_root).map_err(anyhow::Error::msg)?;
    if base.registry.systems.len() != 60 || base.spec.metric != metric_kind(branch)? {
        bail!("base bundle is not the expected 60-system {branch} bundle");
    }
    std::fs::create_dir_all(output)?;
    let mut registry = base.registry.clone();
    let by_id: BTreeMap<_, _> = registry
        .systems
        .iter()
        .map(|system| (system.system_id.clone(), system.clone()))
        .collect();
    let mut additions = Vec::new();
    for policy in POLICIES {
        for model in MODELS {
            let selected = &parameters[&(policy.to_owned(), branch.to_owned(), model.to_owned())];
            let mut template = by_id
                .get(&format!("{model}__max"))
                .with_context(|| format!("base bundle lacks {model}__max"))?
                .clone();
            let system_id = adaptive_system_id(policy, model)?;
            template.system_id = system_id.clone();
            template.score_column = format!("score_{system_id}");
            template.display_label = format!(
                "{} / Self-gated Hill-q ({})",
                template
                    .display_label
                    .rsplit_once(" / ")
                    .map_or(template.display_label.as_str(), |pair| pair.0),
                if policy == "pdac_only" {
                    "PDAC-selected"
                } else {
                    "all-context-selected"
                },
            );
            template.annotations.insert(
                "l2_aggregation_id".into(),
                json!(system_id.split_once("__").unwrap().1),
            );
            template.annotations.insert(
                "l2_aggregation_family".into(),
                json!("self_gated_adaptive_hillq"),
            );
            template
                .annotations
                .insert("selection_policy".into(), json!(policy));
            template.annotations.insert(
                "self_gated_hillq_parameters".into(),
                json!({
                    "q_id": selected.fields["q_id"],
                    "q": selected.fields["q"],
                    "c": selected.fields["c"].parse::<f64>()?,
                    "kappa": selected.fields["kappa"].parse::<f64>()?,
                    "formula": "S solves S = m + C_q * sigmoid((c-S)/kappa)",
                    "floor": (1e-12_f64).ln(),
                    "selection_manifest_sha256": hashes["manifest"],
                    "selected_parameters_sha256": hashes["selected_parameters"],
                    "post_selection_status": "post_hoc_model_development",
                }),
            );
            additions.push(template);
        }
    }
    registry.systems.extend(additions.clone());
    let mut spec = base.spec.clone();
    spec.annotations.insert(
        "adaptive_l2_selection".into(),
        json!({
            "aggregation_family": "self_gated_adaptive_hillq",
            "formula": "S solves S = m + C_q * sigmoid((c-S)/kappa)",
            "floor": (1e-12_f64).ln(),
            "selection_policies": POLICIES,
            "selection_manifest_sha256": hashes["manifest"],
            "selected_parameters_sha256": hashes["selected_parameters"],
        }),
    );
    registry.validate(&spec).map_err(anyhow::Error::msg)?;
    write_json(&output.join("systems.json"), &registry)?;
    write_json(&output.join("tournament_spec.json"), &spec)?;
    let mut manifests = Vec::new();
    for cohort in COHORTS {
        let source = base_root.join("evaluations").join(cohort);
        let destination = output.join("evaluations").join(cohort);
        std::fs::create_dir_all(&destination)?;
        std::fs::copy(
            source.join("endpoints.csv"),
            destination.join("endpoints.csv"),
        )?;
        std::fs::copy(
            source.join("endpoint_identities.csv"),
            destination.join("endpoint_identities.csv"),
        )?;
        extend_scores(
            &source.join("scores.csv"),
            &destination.join("scores.csv"),
            branch,
            cohort,
            &additions,
            scores,
        )?;
        let mut provenance: Value = read_json(&source.join("source_provenance.json"))?;
        provenance["adaptive_l2_extension"] = json!({
            "aggregation_family": "self_gated_adaptive_hillq",
            "metric_branch": branch,
            "selection_root": selection_root,
            "selection_artifact_hashes": hashes,
            "base_bundle": base_root,
            "base_bundle_content_hash": base.manifest.bundle_content_hash,
        });
        write_json(&destination.join("source_provenance.json"), &provenance)?;
        manifests.push(evaluation_manifest(output, cohort)?);
    }
    let manifest = finalize_manifest(
        output,
        "iris-rust-self-gated-hillq-provider-v3".into(),
        spec.metric,
        manifests,
    )?;
    let expected_matches = 3 * registry.systems.len() * (registry.systems.len() - 1) / 2;
    if registry.systems.len() != 70 || expected_matches != 7245 {
        bail!("combined tournament dimensions are not 70 systems / 7245 matches");
    }
    Ok(
        json!({"metric": manifest.metric, "systems": 70, "expected_matches": expected_matches, "bundle_content_hash": manifest.bundle_content_hash}),
    )
}

fn extend_scores(
    source: &Path,
    destination: &Path,
    branch: &str,
    cohort: &str,
    additions: &[SystemRecord],
    scores: &AdaptiveScores,
) -> Result<()> {
    let labels = base_labels(
        source
            .parent()
            .context("base score path has no evaluation directory")?
            .join("endpoints.csv")
            .as_path(),
    )?;
    let mut reader = csv::Reader::from_path(source)?;
    let mut header = reader.headers()?.clone();
    for system in additions {
        header.push_field(&system.score_column);
    }
    let mut writer = csv::Writer::from_path(destination)?;
    writer.write_record(&header)?;
    let endpoint_index = header
        .iter()
        .position(|field| field == "endpoint_id")
        .context("base scores lack endpoint_id")?;
    let expected = base_endpoint_ids(source)?;
    for policy in POLICIES {
        for model in MODELS {
            let key = (
                policy.to_owned(),
                model.to_owned(),
                format!("{branch}:{cohort}"),
            );
            let roster = scores
                .get(&key)
                .with_context(|| format!("adaptive scores lack scope {key:?}"))?
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>();
            if roster != expected {
                bail!("adaptive and base endpoint rosters differ for {key:?}");
            }
        }
    }
    let mut seen = BTreeMap::<(String, String), BTreeSet<String>>::new();
    for result in reader.records() {
        let mut record = result?;
        let endpoint = record
            .get(endpoint_index)
            .context("short base score row")?
            .to_owned();
        for policy in POLICIES {
            for model in MODELS {
                let key = (
                    policy.to_owned(),
                    model.to_owned(),
                    format!("{branch}:{cohort}"),
                );
                let (label, score) = scores
                    .get(&key)
                    .and_then(|rows| rows.get(&endpoint))
                    .with_context(|| format!("adaptive roster lacks {key:?}/{endpoint}"))?;
                if labels.get(&endpoint) != Some(label) {
                    bail!("adaptive label differs from base authority for {endpoint}");
                }
                record.push_field(&score.to_string());
                seen.entry((policy.into(), model.into()))
                    .or_default()
                    .insert(endpoint.clone());
            }
        }
        writer.write_record(&record)?;
    }
    writer.flush()?;
    for roster in seen.values() {
        if roster != &expected {
            bail!("adaptive and base endpoint rosters differ");
        }
    }
    Ok(())
}

fn base_labels(path: &Path) -> Result<BTreeMap<String, bool>> {
    let mut reader = csv::Reader::from_path(path)?;
    let headers = reader.headers()?.clone();
    let endpoint = headers
        .iter()
        .position(|field| field == "endpoint_id")
        .context("base endpoints lack endpoint_id")?;
    let label = headers
        .iter()
        .position(|field| field == "label")
        .context("base endpoints lack label")?;
    let mut labels = BTreeMap::new();
    for record in reader.records() {
        let record = record?;
        let value = match record.get(label).context("short endpoint row")? {
            "0" => false,
            "1" => true,
            other => bail!("invalid base endpoint label {other:?}"),
        };
        if labels
            .insert(
                record
                    .get(endpoint)
                    .context("short endpoint row")?
                    .to_owned(),
                value,
            )
            .is_some()
        {
            bail!("duplicate base endpoint");
        }
    }
    Ok(labels)
}

fn base_endpoint_ids(scores: &Path) -> Result<BTreeSet<String>> {
    let mut reader = csv::Reader::from_path(scores)?;
    let index = reader
        .headers()?
        .iter()
        .position(|field| field == "endpoint_id")
        .context("base scores lack endpoint_id")?;
    reader
        .records()
        .map(|record| Ok(record?.get(index).context("short score row")?.to_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template_config() -> PipelineConfig {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../IRIS_scripts/iris_fullroster_pipeline.example.json");
        serde_json::from_reader(std::fs::File::open(path).unwrap()).unwrap()
    }

    #[test]
    fn fixed_spec_exposes_all_evaluation_label_contracts() {
        let config = template_config();
        let registry = fixed_registry(&config, "pr").unwrap();
        let spec = tournament_spec(&config, "pr", &registry).unwrap();
        let contract = &spec.annotations["scientific_contract"];
        assert_eq!(
            contract["evaluation_label_contracts"]["covid_nonspike"]["comparison_operator"],
            serde_json::json!(">")
        );
        assert_eq!(
            contract["evaluation_label_contracts"]["pdac"]["label_field"],
            serde_json::json!("long_peptide_label")
        );
    }
}
