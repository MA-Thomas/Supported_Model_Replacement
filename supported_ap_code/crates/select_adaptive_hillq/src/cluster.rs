//! Deterministic cluster planning, sharded paired-CNAP execution, auditing,
//! and loading of complete shard outcomes for final selection.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use directed_round_robin_organizer::input::{LoadedBundle, load_bundle};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cnap::{
    CnapContract, CnapOutcome, aligned_maximum_reference, complete_forward_assessment,
    observed_gate, parse_signature_hex, ranking_signature, signature_hex,
};
use crate::config::{MODELS, SELECTION_COHORTS};
use crate::data::{
    AuthEndpoint, Task, authoritative_endpoint_table, load_task, validate_task_labels,
};
use crate::error::{Result, SelectionError};
use crate::grid::{
    AggregationOrder, BaseGrid, GridSpec, aggregation_orders, make_grid, parse_alpha_values,
    parse_q_values,
};
use crate::manifest::sha256_file;
use crate::numeric::{HillOrder, PowerOrder, adaptive_components, self_gated_score};
use crate::output::{fmt_f64, write_text};

pub const CLUSTER_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GridContract {
    pub alpha_values: Vec<String>,
    pub q_values: Vec<String>,
    pub c_min: f64,
    pub c_max: f64,
    pub c_step: f64,
    pub kappa_min: f64,
    pub kappa_max: f64,
    pub kappa_points: usize,
    pub solver_absolute_tolerance: f64,
    pub solver_max_iterations: usize,
}

impl GridContract {
    pub fn new(
        powers: &[PowerOrder],
        hills: &[HillOrder],
        spec: &GridSpec,
        solver_absolute_tolerance: f64,
        solver_max_iterations: usize,
    ) -> Self {
        Self {
            alpha_values: powers.iter().map(|order| fmt_f64(order.value())).collect(),
            q_values: hills.iter().map(|order| fmt_f64(order.value())).collect(),
            c_min: spec.c_min,
            c_max: spec.c_max,
            c_step: spec.c_step,
            kappa_min: spec.kappa_min,
            kappa_max: spec.kappa_max,
            kappa_points: spec.kappa_points,
            solver_absolute_tolerance,
            solver_max_iterations,
        }
    }

    pub fn orders(&self) -> Result<Vec<AggregationOrder>> {
        let powers = parse_alpha_values(&self.alpha_values)?;
        let hills = parse_q_values(&self.q_values)?;
        Ok(aggregation_orders(&powers, &hills))
    }

    pub fn grid_spec(&self) -> GridSpec {
        GridSpec {
            c_min: self.c_min,
            c_max: self.c_max,
            c_step: self.c_step,
            kappa_min: self.kappa_min,
            kappa_max: self.kappa_max,
            kappa_points: self.kappa_points,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlanOptions {
    pub source_root: PathBuf,
    pub bundle_root: PathBuf,
    pub output: PathBuf,
    pub grid: GridContract,
    pub selection_replications: usize,
    pub matches_per_shard: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanIdentity {
    schema_version: u32,
    analysis: String,
    source_manifest_sha256: String,
    pr_bundle_manifest_sha256: String,
    pr_tournament_spec_sha256: String,
    grid: GridContract,
    selection_replications: usize,
    inherited_tournament_replications: usize,
    matches_per_shard: usize,
    cluster_executable_sha256: String,
    search_refinement_multiplier: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardRecord {
    pub shard_id: usize,
    pub match_count: usize,
    pub relative_path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanManifest {
    pub schema_version: u32,
    pub analysis: String,
    pub plan_id: String,
    pub source_root_at_planning: String,
    pub bundle_root_at_planning: String,
    pub source_manifest_sha256: String,
    pub pr_bundle_manifest_sha256: String,
    pub pr_tournament_spec_sha256: String,
    pub grid: GridContract,
    pub selection_replications: usize,
    pub inherited_tournament_replications: usize,
    pub matches_per_shard: usize,
    pub cluster_executable_sha256: String,
    pub search_refinement_multiplier: usize,
    pub unique_match_count: usize,
    pub shard_count: usize,
    pub unique_matches_by_model_cohort: BTreeMap<String, usize>,
    pub selection_seeds: BTreeMap<String, u64>,
    pub shards: Vec<ShardRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannedMatch {
    pub global_index: usize,
    pub model: String,
    pub cohort: String,
    pub aggregation_order: usize,
    pub alpha_order: usize,
    pub q_order: usize,
    pub grid_index: usize,
    pub ranking_signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShardPlan {
    schema_version: u32,
    plan_id: String,
    shard_id: usize,
    matches: Vec<PlannedMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchOutcome {
    pub planned: PlannedMatch,
    pub outcome: CnapOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShardResult {
    schema_version: u32,
    plan_id: String,
    plan_shard_sha256: String,
    shard_id: usize,
    selection_replications: usize,
    cluster_executable_sha256: String,
    search_refinement_multiplier: usize,
    outcomes: Vec<MatchOutcome>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub schema_version: u32,
    pub plan_id: String,
    pub expected_shards: usize,
    pub complete_shards: usize,
    pub expected_matches: usize,
    pub complete_matches: usize,
    pub missing_shards: Vec<usize>,
    pub invalid_shards: Vec<usize>,
    pub invalid_shard_errors: BTreeMap<usize, String>,
    pub complete: bool,
}

pub struct PrecomputedCnap {
    pub plan: PlanManifest,
    outcomes: HashMap<(String, String), HashMap<[u8; 32], CnapOutcome>>,
}

impl PrecomputedCnap {
    pub fn outcome(&self, model: &str, cohort: &str, signature: &[u8; 32]) -> Result<CnapOutcome> {
        self.outcomes
            .get(&(model.to_string(), cohort.to_string()))
            .and_then(|group| group.get(signature))
            .cloned()
            .ok_or_else(|| {
                SelectionError::msg(format!(
                    "audited CNAP cache lacks {model}/{cohort}/{}",
                    signature_hex(signature)
                ))
            })
    }
}

fn io_error(path: &Path, source: std::io::Error) -> SelectionError {
    SelectionError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let text = std::fs::read_to_string(path).map_err(|source| io_error(path, source))?;
    serde_json::from_str(&text).map_err(|source| SelectionError::Json {
        path: path.to_path_buf(),
        source,
    })
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(|source| SelectionError::Json {
        path: path.to_path_buf(),
        source,
    })? + "\n";
    write_text(path, &text)
}

fn sha256_serialized<T: Serialize>(value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(value).map_err(|source| SelectionError::Json {
        path: PathBuf::from("<serialized-plan-identity>"),
        source,
    })?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn canonical_or_original(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn current_executable_sha256() -> Result<String> {
    let executable = std::env::current_exe().map_err(|source| SelectionError::Io {
        path: PathBuf::from("<current-executable>"),
        source,
    })?;
    sha256_file(&executable)
}

fn validate_inputs(source_root: &Path, bundle_root: &Path) -> Result<LoadedBundle> {
    if !source_root.join("manifest.json").is_file() {
        return Err(SelectionError::msg(format!(
            "source root lacks manifest.json: {}",
            source_root.display()
        )));
    }
    iris_fullroster_pipeline::transfer::validate_transfer_package(source_root).map_err(
        |error| {
            SelectionError::msg(format!(
                "Rust transfer package validation failed for {}: {error:#}",
                source_root.display()
            ))
        },
    )?;
    load_bundle(&bundle_root.join("pr")).map_err(|error| {
        SelectionError::msg(format!("failed to load authoritative PR bundle: {error}"))
    })
}

fn input_hashes(source_root: &Path, bundle_root: &Path) -> Result<(String, String, String)> {
    Ok((
        sha256_file(&source_root.join("manifest.json"))?,
        sha256_file(&bundle_root.join("pr").join("bundle_manifest.json"))?,
        sha256_file(&bundle_root.join("pr").join("tournament_spec.json"))?,
    ))
}

fn task_and_reference(
    source_root: &Path,
    bundle_root: &Path,
    bundle: &LoadedBundle,
    authorities: &BTreeMap<String, Vec<AuthEndpoint>>,
    model: &str,
    cohort: &str,
) -> Result<(Task, Vec<bool>, Vec<f64>)> {
    let (task, _) = load_task(source_root, cohort, model, "pr")?;
    let authority = authorities
        .get(cohort)
        .ok_or_else(|| SelectionError::msg(format!("missing authority for {cohort}")))?;
    let endpoint_ids = validate_task_labels(&task, authority)?;
    let labels = task.labels();
    let labels_bool = labels.iter().map(|&label| label == 1).collect();
    let (_, maximum, _) = aligned_maximum_reference(bundle, cohort, model, &endpoint_ids, &labels)?;
    let _ = bundle_root;
    Ok((task, labels_bool, maximum))
}

fn score_vector(
    anchors: &[f64],
    corroboration_offer: &[f64],
    base: &BaseGrid,
    grid_index: usize,
    tolerance: f64,
    iterations: usize,
) -> Result<Vec<f64>> {
    anchors
        .iter()
        .zip(corroboration_offer)
        .map(|(&anchor, &offer)| {
            self_gated_score(
                anchor,
                offer,
                base.c[grid_index],
                base.kappa[grid_index],
                tolerance,
                iterations,
            )
        })
        .collect()
}

fn write_plan_shard(
    staging: &Path,
    plan_id: &str,
    shard_id: usize,
    matches: Vec<PlannedMatch>,
) -> Result<ShardRecord> {
    let relative_path = format!("shards/shard_{shard_id}.json");
    let path = staging.join(&relative_path);
    let shard = ShardPlan {
        schema_version: CLUSTER_SCHEMA_VERSION,
        plan_id: plan_id.to_string(),
        shard_id,
        matches,
    };
    write_json(&path, &shard)?;
    Ok(ShardRecord {
        shard_id,
        match_count: shard.matches.len(),
        relative_path,
        sha256: sha256_file(&path)?,
    })
}

pub fn create_plan(options: &PlanOptions) -> Result<PlanManifest> {
    if options.matches_per_shard == 0 || options.selection_replications == 0 {
        return Err(SelectionError::msg(
            "matches-per-shard and selection-replications must be positive",
        ));
    }
    if options.output.exists() {
        return Err(SelectionError::msg(format!(
            "plan output already exists: {}",
            options.output.display()
        )));
    }
    let source_root = canonical_or_original(&options.source_root);
    let bundle_root = canonical_or_original(&options.bundle_root);
    let bundle = validate_inputs(&source_root, &bundle_root)?;
    let orders = options.grid.orders()?;
    let spec = options.grid.grid_spec();
    let base = make_grid(&spec)?;
    if !options.grid.solver_absolute_tolerance.is_finite()
        || options.grid.solver_absolute_tolerance <= 0.0
        || options.grid.solver_max_iterations == 0
    {
        return Err(SelectionError::msg("invalid solver contract"));
    }
    let contract =
        CnapContract::from_pr_bundle_with_replications(&bundle, options.selection_replications)?;
    let inherited = bundle.spec.computational_design.replications;
    let (source_hash, bundle_hash, spec_hash) = input_hashes(&source_root, &bundle_root)?;
    let executable_hash = current_executable_sha256()?;
    let identity = PlanIdentity {
        schema_version: CLUSTER_SCHEMA_VERSION,
        analysis: "adaptive_power_hillq_complete_f_paired_cnap_selection".to_string(),
        source_manifest_sha256: source_hash.clone(),
        pr_bundle_manifest_sha256: bundle_hash.clone(),
        pr_tournament_spec_sha256: spec_hash.clone(),
        grid: options.grid.clone(),
        selection_replications: options.selection_replications,
        inherited_tournament_replications: inherited,
        matches_per_shard: options.matches_per_shard,
        cluster_executable_sha256: executable_hash.clone(),
        search_refinement_multiplier: crate::cnap::SEARCH_REFINEMENT_MULTIPLIER,
    };
    let plan_id = sha256_serialized(&identity)?;

    let parent = options.output.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
    let staging = parent.join(format!(
        ".{}.tmp-{}",
        options
            .output
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("adaptive-plan"),
        std::process::id()
    ));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|source| io_error(&staging, source))?;
    }
    std::fs::create_dir_all(staging.join("shards")).map_err(|source| io_error(&staging, source))?;

    let built = (|| {
        let mut authorities = BTreeMap::new();
        for cohort in SELECTION_COHORTS {
            authorities.insert(
                cohort.to_string(),
                authoritative_endpoint_table(&bundle_root, cohort)?,
            );
        }
        let mut pending = Vec::with_capacity(options.matches_per_shard);
        let mut records = Vec::new();
        let mut group_counts = BTreeMap::new();
        let mut global_index = 0usize;

        for model in MODELS {
            for cohort in SELECTION_COHORTS {
                let (task, _, _) = task_and_reference(
                    &source_root,
                    &bundle_root,
                    &bundle,
                    &authorities,
                    model,
                    cohort,
                )?;
                let mut representatives: HashMap<[u8; 32], (usize, usize)> = HashMap::new();
                for (aggregation_order, order) in orders.iter().enumerate() {
                    let (anchors, offers) =
                        adaptive_components(&task.raw_candidates, order.power, order.hill)?;
                    let signatures: Result<Vec<[u8; 32]>> = (0..base.len())
                        .into_par_iter()
                        .map(|grid_index| {
                            let scores = score_vector(
                                &anchors,
                                &offers,
                                &base,
                                grid_index,
                                options.grid.solver_absolute_tolerance,
                                options.grid.solver_max_iterations,
                            )?;
                            Ok(ranking_signature(&scores))
                        })
                        .collect();
                    for (grid_index, signature) in signatures?.into_iter().enumerate() {
                        representatives
                            .entry(signature)
                            .or_insert((aggregation_order, grid_index));
                    }
                }
                let mut unique: Vec<([u8; 32], (usize, usize))> =
                    representatives.into_iter().collect();
                unique.sort_by_key(|(_, representative)| *representative);
                group_counts.insert(format!("{model}/{cohort}"), unique.len());
                eprintln!(
                    "planned {model}/{cohort}: {} exact unique rankings",
                    unique.len()
                );
                for (signature, (aggregation_order, grid_index)) in unique {
                    let order = orders[aggregation_order];
                    pending.push(PlannedMatch {
                        global_index,
                        model: model.to_string(),
                        cohort: cohort.to_string(),
                        aggregation_order,
                        alpha_order: order.alpha_order,
                        q_order: order.q_order,
                        grid_index,
                        ranking_signature: signature_hex(&signature),
                    });
                    global_index += 1;
                    if pending.len() == options.matches_per_shard {
                        let shard_id = records.len();
                        records.push(write_plan_shard(
                            &staging,
                            &plan_id,
                            shard_id,
                            std::mem::take(&mut pending),
                        )?);
                        pending = Vec::with_capacity(options.matches_per_shard);
                    }
                }
            }
        }
        if !pending.is_empty() {
            let shard_id = records.len();
            records.push(write_plan_shard(&staging, &plan_id, shard_id, pending)?);
        }
        let selection_seeds = SELECTION_COHORTS
            .iter()
            .map(|cohort| Ok((cohort.to_string(), contract.selection_seed(cohort)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        let manifest = PlanManifest {
            schema_version: CLUSTER_SCHEMA_VERSION,
            analysis: identity.analysis,
            plan_id,
            source_root_at_planning: source_root.display().to_string(),
            bundle_root_at_planning: bundle_root.display().to_string(),
            source_manifest_sha256: source_hash,
            pr_bundle_manifest_sha256: bundle_hash,
            pr_tournament_spec_sha256: spec_hash,
            grid: options.grid.clone(),
            selection_replications: options.selection_replications,
            inherited_tournament_replications: inherited,
            matches_per_shard: options.matches_per_shard,
            cluster_executable_sha256: executable_hash,
            search_refinement_multiplier: crate::cnap::SEARCH_REFINEMENT_MULTIPLIER,
            unique_match_count: global_index,
            shard_count: records.len(),
            unique_matches_by_model_cohort: group_counts,
            selection_seeds,
            shards: records,
        };
        write_json(&staging.join("plan.json"), &manifest)?;
        Ok(manifest)
    })();

    match built {
        Ok(manifest) => {
            std::fs::rename(&staging, &options.output)
                .map_err(|source| io_error(&options.output, source))?;
            Ok(manifest)
        }
        Err(error) => {
            std::fs::remove_dir_all(&staging).ok();
            Err(error)
        }
    }
}

pub fn load_plan(plan_root: &Path) -> Result<PlanManifest> {
    let manifest: PlanManifest = read_json(&plan_root.join("plan.json"))?;
    let identity = PlanIdentity {
        schema_version: manifest.schema_version,
        analysis: manifest.analysis.clone(),
        source_manifest_sha256: manifest.source_manifest_sha256.clone(),
        pr_bundle_manifest_sha256: manifest.pr_bundle_manifest_sha256.clone(),
        pr_tournament_spec_sha256: manifest.pr_tournament_spec_sha256.clone(),
        grid: manifest.grid.clone(),
        selection_replications: manifest.selection_replications,
        inherited_tournament_replications: manifest.inherited_tournament_replications,
        matches_per_shard: manifest.matches_per_shard,
        cluster_executable_sha256: manifest.cluster_executable_sha256.clone(),
        search_refinement_multiplier: manifest.search_refinement_multiplier,
    };
    if manifest.search_refinement_multiplier != crate::cnap::SEARCH_REFINEMENT_MULTIPLIER
        || manifest.schema_version != CLUSTER_SCHEMA_VERSION
        || sha256_serialized(&identity)? != manifest.plan_id
        || manifest.shard_count != manifest.shards.len()
        || manifest.unique_match_count
            != manifest
                .shards
                .iter()
                .map(|shard| shard.match_count)
                .sum::<usize>()
    {
        return Err(SelectionError::msg("invalid adaptive CNAP plan manifest"));
    }
    Ok(manifest)
}

fn load_plan_shard(
    plan_root: &Path,
    manifest: &PlanManifest,
    shard_id: usize,
) -> Result<(ShardRecord, ShardPlan)> {
    let record = manifest
        .shards
        .get(shard_id)
        .filter(|record| record.shard_id == shard_id)
        .cloned()
        .ok_or_else(|| SelectionError::msg(format!("unknown shard id {shard_id}")))?;
    let path = plan_root.join(&record.relative_path);
    if sha256_file(&path)? != record.sha256 {
        return Err(SelectionError::msg(format!(
            "plan shard hash mismatch: {}",
            path.display()
        )));
    }
    let shard: ShardPlan = read_json(&path)?;
    if shard.schema_version != CLUSTER_SCHEMA_VERSION
        || shard.plan_id != manifest.plan_id
        || shard.shard_id != shard_id
        || shard.matches.len() != record.match_count
    {
        return Err(SelectionError::msg(format!(
            "invalid plan shard {}",
            path.display()
        )));
    }
    Ok((record, shard))
}

fn result_path(results_root: &Path, shard_id: usize) -> PathBuf {
    results_root.join(format!("shard_{shard_id}.json"))
}

fn result_hash_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(".sha256");
    PathBuf::from(value)
}

fn validate_shard_result_payload(
    result: &ShardResult,
    path: &Path,
    manifest: &PlanManifest,
    record: &ShardRecord,
    shard: &ShardPlan,
) -> Result<()> {
    let metadata_valid = result.schema_version == CLUSTER_SCHEMA_VERSION
        && result.plan_id == manifest.plan_id
        && result.plan_shard_sha256 == record.sha256
        && result.shard_id == shard.shard_id
        && result.selection_replications == manifest.selection_replications
        && result.cluster_executable_sha256 == manifest.cluster_executable_sha256
        && result.search_refinement_multiplier == manifest.search_refinement_multiplier
        && result.outcomes.len() == shard.matches.len();
    if !metadata_valid {
        return Err(SelectionError::msg(format!(
            "result metadata mismatch: {}",
            path.display()
        )));
    }
    for (actual, expected) in result.outcomes.iter().zip(&shard.matches) {
        if &actual.planned != expected {
            return Err(SelectionError::msg(format!(
                "result entry mismatch: {}",
                path.display()
            )));
        }
        validate_outcome(&actual.outcome, path)?;
    }
    Ok(())
}

fn validate_shard_result(
    path: &Path,
    manifest: &PlanManifest,
    record: &ShardRecord,
    shard: &ShardPlan,
) -> Result<ShardResult> {
    let hash_path = result_hash_path(path);
    let recorded_hash = std::fs::read_to_string(&hash_path)
        .map_err(|source| io_error(&hash_path, source))?
        .trim()
        .to_ascii_lowercase();
    if recorded_hash.len() != 64 || !recorded_hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SelectionError::msg(format!(
            "invalid result checksum sidecar: {}",
            hash_path.display()
        )));
    }
    let actual_hash = sha256_file(path)?;
    if actual_hash != recorded_hash {
        return Err(SelectionError::msg(format!(
            "result file checksum mismatch: {} (recorded {}, recomputed {})",
            path.display(),
            recorded_hash,
            actual_hash
        )));
    }
    let result: ShardResult = read_json(path)?;
    validate_shard_result_payload(&result, path, manifest, record, shard)?;
    Ok(result)
}

fn validate_outcome(outcome: &CnapOutcome, path: &Path) -> Result<()> {
    let required = [
        outcome.observed_retained_difference,
        outcome.limiting_prevalence,
        outcome.adaptive_empirical_ap,
        outcome.maximum_empirical_ap,
    ];
    let optional = [
        outcome.supported_magnitude,
        outcome.survival_subset_fraction,
        outcome.mean_retained_effect,
    ];
    let full_present = optional.iter().all(Option::is_some);
    let full_absent = optional.iter().all(Option::is_none);
    let survival_valid = outcome
        .survival_subset_fraction
        .is_none_or(|value| value.is_finite() && (0.0..=1.0).contains(&value));
    if outcome
        .observed_bounds
        .is_some_and(|b| b.verdict != outcome.observed_status)
        || outcome
            .full_bounds
            .is_some_and(|b| b.verdict != outcome.staged_status)
        || !outcome.search_tolerance.is_finite()
        || outcome.search_tolerance <= 0.0
        || outcome.observed_gate_supported
            != (outcome.observed_status == supported_ap::PolicyVerdict::VerifiedPass)
        || outcome.staged_supported
            != (outcome.staged_status == supported_ap::PolicyVerdict::VerifiedPass)
        || outcome.search_iterations > outcome.maximum_search_iterations
        || required.iter().any(|value| !value.is_finite())
        || optional.iter().flatten().any(|value| !value.is_finite())
        || !survival_valid
        || outcome.staged_supported && !outcome.observed_gate_supported
        || outcome.observed_gate_supported && !full_present
        || !outcome.observed_gate_supported && !full_absent
    {
        return Err(SelectionError::msg(format!(
            "invalid paired-CNAP outcome in {}",
            path.display()
        )));
    }
    Ok(())
}

struct WorkerContext {
    anchors: Vec<f64>,
    corroboration_offer: Vec<f64>,
    labels: Vec<bool>,
    maximum_reference: Vec<f64>,
    selection_seed: u64,
}

pub fn run_shard(
    source_root: &Path,
    bundle_root: &Path,
    plan_root: &Path,
    results_root: &Path,
    shard_id: usize,
) -> Result<usize> {
    let source_root = canonical_or_original(source_root);
    let bundle_root = canonical_or_original(bundle_root);
    let manifest = load_plan(plan_root)?;
    validate_plan_inputs(&manifest, &source_root, &bundle_root)?;
    if current_executable_sha256()? != manifest.cluster_executable_sha256 {
        return Err(SelectionError::msg(
            "cluster worker executable differs from the immutable planning executable",
        ));
    }
    let (record, shard) = load_plan_shard(plan_root, &manifest, shard_id)?;
    std::fs::create_dir_all(results_root).map_err(|source| io_error(results_root, source))?;
    let final_path = result_path(results_root, shard_id);
    if final_path.exists() {
        validate_shard_result(&final_path, &manifest, &record, &shard)?;
        return Ok(shard.matches.len());
    }

    let bundle = load_bundle(&bundle_root.join("pr")).map_err(|error| {
        SelectionError::msg(format!("failed to load authoritative PR bundle: {error}"))
    })?;
    let contract =
        CnapContract::from_pr_bundle_with_replications(&bundle, manifest.selection_replications)?;
    let orders = manifest.grid.orders()?;
    let base = make_grid(&manifest.grid.grid_spec())?;
    let mut authorities = BTreeMap::new();
    for cohort in SELECTION_COHORTS {
        authorities.insert(
            cohort.to_string(),
            authoritative_endpoint_table(&bundle_root, cohort)?,
        );
    }

    let mut contexts = HashMap::new();
    for planned in &shard.matches {
        let key = (
            planned.model.clone(),
            planned.cohort.clone(),
            planned.aggregation_order,
        );
        if contexts.contains_key(&key) {
            continue;
        }
        let order = *orders.get(planned.aggregation_order).ok_or_else(|| {
            SelectionError::msg(format!(
                "invalid aggregation order {}",
                planned.aggregation_order
            ))
        })?;
        let (task, labels, maximum_reference) = task_and_reference(
            &source_root,
            &bundle_root,
            &bundle,
            &authorities,
            &planned.model,
            &planned.cohort,
        )?;
        let (anchors, corroboration_offer) =
            adaptive_components(&task.raw_candidates, order.power, order.hill)?;
        contexts.insert(
            key,
            WorkerContext {
                anchors,
                corroboration_offer,
                labels,
                maximum_reference,
                selection_seed: contract.selection_seed(&planned.cohort)?,
            },
        );
    }

    let outcomes: Result<Vec<MatchOutcome>> = shard
        .matches
        .par_iter()
        .map(|planned| {
            let context = contexts
                .get(&(
                    planned.model.clone(),
                    planned.cohort.clone(),
                    planned.aggregation_order,
                ))
                .ok_or_else(|| SelectionError::msg("missing worker context"))?;
            if planned.grid_index >= base.len() {
                return Err(SelectionError::msg(format!(
                    "invalid grid index {}",
                    planned.grid_index
                )));
            }
            let scores = score_vector(
                &context.anchors,
                &context.corroboration_offer,
                &base,
                planned.grid_index,
                manifest.grid.solver_absolute_tolerance,
                manifest.grid.solver_max_iterations,
            )?;
            let actual_signature = ranking_signature(&scores);
            if signature_hex(&actual_signature) != planned.ranking_signature {
                return Err(SelectionError::msg(format!(
                    "ranking signature reconstruction failed for match {}",
                    planned.global_index
                )));
            }
            let (paired, observed) = observed_gate(
                &scores,
                &context.maximum_reference,
                &context.labels,
                &contract,
            )?;
            let outcome =
                complete_forward_assessment(&paired, observed, &contract, context.selection_seed)?;
            Ok(MatchOutcome {
                planned: planned.clone(),
                outcome,
            })
        })
        .collect();
    let result = ShardResult {
        schema_version: CLUSTER_SCHEMA_VERSION,
        plan_id: manifest.plan_id.clone(),
        plan_shard_sha256: record.sha256.clone(),
        shard_id,
        selection_replications: manifest.selection_replications,
        cluster_executable_sha256: manifest.cluster_executable_sha256.clone(),
        search_refinement_multiplier: manifest.search_refinement_multiplier,
        outcomes: outcomes?,
    };
    validate_shard_result_payload(&result, &final_path, &manifest, &record, &shard)?;
    let temporary = results_root.join(format!(".shard_{shard_id}.tmp-{}.json", std::process::id()));
    write_json(&temporary, &result)?;
    let temporary_hash = result_hash_path(&temporary);
    write_text(&temporary_hash, &(sha256_file(&temporary)? + "\n"))?;
    let final_hash = result_hash_path(&final_path);
    std::fs::rename(&temporary_hash, &final_hash)
        .map_err(|source| io_error(&final_hash, source))?;
    std::fs::rename(&temporary, &final_path).map_err(|source| io_error(&final_path, source))?;
    validate_shard_result(&final_path, &manifest, &record, &shard)?;
    Ok(result.outcomes.len())
}

pub fn status(plan_root: &Path, results_root: &Path) -> Result<StatusReport> {
    let manifest = load_plan(plan_root)?;
    let mut complete_shards = 0usize;
    let mut complete_matches = 0usize;
    let mut missing = Vec::new();
    let mut invalid = Vec::new();
    let mut invalid_errors = BTreeMap::new();
    for record in &manifest.shards {
        let path = result_path(results_root, record.shard_id);
        if !path.is_file() {
            missing.push(record.shard_id);
            continue;
        }
        match load_plan_shard(plan_root, &manifest, record.shard_id)
            .and_then(|(_, shard)| validate_shard_result(&path, &manifest, record, &shard))
        {
            Ok(result) => {
                complete_shards += 1;
                complete_matches += result.outcomes.len();
            }
            Err(error) => {
                invalid.push(record.shard_id);
                invalid_errors.insert(record.shard_id, error.to_string());
            }
        }
    }
    Ok(StatusReport {
        schema_version: CLUSTER_SCHEMA_VERSION,
        plan_id: manifest.plan_id,
        expected_shards: manifest.shard_count,
        complete_shards,
        expected_matches: manifest.unique_match_count,
        complete_matches,
        complete: missing.is_empty()
            && invalid.is_empty()
            && complete_matches == manifest.unique_match_count,
        missing_shards: missing,
        invalid_shards: invalid,
        invalid_shard_errors: invalid_errors,
    })
}

fn validate_plan_inputs(
    manifest: &PlanManifest,
    source_root: &Path,
    bundle_root: &Path,
) -> Result<()> {
    let (source_hash, bundle_hash, spec_hash) = input_hashes(source_root, bundle_root)?;
    if source_hash != manifest.source_manifest_sha256
        || bundle_hash != manifest.pr_bundle_manifest_sha256
        || spec_hash != manifest.pr_tournament_spec_sha256
    {
        return Err(SelectionError::msg(
            "current source or PR bundle does not match the immutable cluster plan",
        ));
    }
    Ok(())
}

pub fn audit(
    source_root: &Path,
    bundle_root: &Path,
    plan_root: &Path,
    results_root: &Path,
) -> Result<StatusReport> {
    let manifest = load_plan(plan_root)?;
    validate_plan_inputs(&manifest, source_root, bundle_root)?;
    let report = status(plan_root, results_root)?;
    if !report.complete {
        return Err(SelectionError::msg(format!(
            "CNAP shard set is incomplete: {}/{} shards complete, {} missing, {} invalid",
            report.complete_shards,
            report.expected_shards,
            report.missing_shards.len(),
            report.invalid_shards.len()
        )));
    }
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
pub fn load_complete_cache(
    source_root: &Path,
    bundle_root: &Path,
    plan_root: &Path,
    results_root: &Path,
    powers: &[PowerOrder],
    hills: &[HillOrder],
    spec: &GridSpec,
    solver_absolute_tolerance: f64,
    solver_max_iterations: usize,
    selection_replications: usize,
) -> Result<PrecomputedCnap> {
    audit(source_root, bundle_root, plan_root, results_root)?;
    let plan = load_plan(plan_root)?;
    let expected_grid = GridContract::new(
        powers,
        hills,
        spec,
        solver_absolute_tolerance,
        solver_max_iterations,
    );
    if plan.grid != expected_grid || plan.selection_replications != selection_replications {
        return Err(SelectionError::msg(
            "final selector grid or selection contract differs from the audited CNAP plan",
        ));
    }
    let mut outcomes: HashMap<(String, String), HashMap<[u8; 32], CnapOutcome>> = HashMap::new();
    for record in &plan.shards {
        let (_, shard) = load_plan_shard(plan_root, &plan, record.shard_id)?;
        let result = validate_shard_result(
            &result_path(results_root, record.shard_id),
            &plan,
            record,
            &shard,
        )?;
        for entry in result.outcomes {
            let signature = parse_signature_hex(&entry.planned.ranking_signature)?;
            let group = outcomes
                .entry((entry.planned.model, entry.planned.cohort))
                .or_default();
            if group.insert(signature, entry.outcome).is_some() {
                return Err(SelectionError::msg(
                    "duplicate ranking outcome across audited shards",
                ));
            }
        }
    }
    if outcomes.values().map(HashMap::len).sum::<usize>() != plan.unique_match_count {
        return Err(SelectionError::msg(
            "audited CNAP cache match count differs from plan",
        ));
    }
    Ok(PrecomputedCnap { plan, outcomes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_contract_round_trips_infinity() {
        let powers = parse_alpha_values(&["0".into(), "inf".into()]).unwrap();
        let hills = parse_q_values(&["1".into(), "inf".into()]).unwrap();
        let spec = GridSpec {
            c_min: -1.0,
            c_max: 1.0,
            c_step: 1.0,
            kappa_min: 0.1,
            kappa_max: 1.0,
            kappa_points: 2,
        };
        let contract = GridContract::new(&powers, &hills, &spec, 1e-10, 64);
        assert_eq!(
            contract.orders().unwrap(),
            aggregation_orders(&powers, &hills)
        );
        assert_eq!(make_grid(&contract.grid_spec()).unwrap().len(), 6);
    }

    #[test]
    fn result_checksum_path_is_adjacent() {
        assert_eq!(
            result_hash_path(Path::new("results/shard_7.json")),
            PathBuf::from("results/shard_7.json.sha256")
        );
    }
}
