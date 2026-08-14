use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::identity::{hash_serializable, sha256_bytes, sha256_file};
use crate::input::LoadedBundle;
use crate::judge::{JudgedMatch, judge_match};
use crate::plan::{MatchPlan, TournamentPlan, validate_plan};
use crate::provenance::{BuildProvenance, capture_build_provenance};
use crate::spec::MetricKind;
use crate::{Error, Result};

pub const MATCH_ARTIFACT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageIdentity {
    pub package: String,
    pub version: String,
    pub scientific_schema: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchArtifact {
    pub schema_version: u32,
    pub complete: bool,
    pub tournament_id: String,
    pub bundle_content_hash: String,
    pub plan_id: String,
    pub match_id: String,
    pub metric: MetricKind,
    pub evaluation_id: String,
    pub system_low: String,
    pub system_high: String,
    pub endpoint_count: usize,
    pub positive_count: usize,
    pub negative_count: usize,
    pub label_vector_hash: String,
    pub score_hash_low: String,
    pub score_hash_high: String,
    pub assessment_policy_hash: String,
    pub seed: u64,
    pub seed_derivation_version: u32,
    pub supported_ap: PackageIdentity,
    pub organizer: PackageIdentity,
    pub plan_build: BuildProvenance,
    pub execution_build: BuildProvenance,
    pub judge_execution: String,
    pub started_unix_milliseconds: u128,
    pub completed_unix_milliseconds: u128,
    pub judged: JudgedMatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardRunSummary {
    pub shard_id: usize,
    pub assigned: usize,
    pub computed: usize,
    pub already_valid: usize,
    pub failed: usize,
    pub worker_threads: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct OperationalFailure {
    schema_version: u32,
    match_id: String,
    plan_id: String,
    error: String,
}

#[derive(Debug)]
enum WorkOutcome {
    Computed,
    AlreadyValid,
}

pub fn result_path(results_root: &Path, match_id: &str) -> PathBuf {
    results_root
        .join("matches")
        .join(format!("{match_id}.json"))
}

pub fn checksum_path(result_path: &Path) -> PathBuf {
    result_path.with_extension("json.sha256")
}

pub fn run_shard(
    bundle: &LoadedBundle,
    plan: &TournamentPlan,
    shard_id: usize,
    results_root: &Path,
    threads: usize,
) -> Result<ShardRunSummary> {
    validate_plan(plan, bundle)?;
    if shard_id >= plan.shard_count {
        return Err(Error::InvalidPlan(format!(
            "shard {shard_id} is outside 0..{}",
            plan.shard_count
        )));
    }
    if threads == 0 {
        return Err(Error::InvalidPlan(
            "worker thread count must be positive".into(),
        ));
    }
    fs::create_dir_all(results_root.join("matches"))
        .map_err(|error| crate::error::io(results_root.join("matches"), error))?;
    fs::create_dir_all(results_root.join("failures"))
        .map_err(|error| crate::error::io(results_root.join("failures"), error))?;
    let assigned: Vec<_> = plan
        .matches
        .iter()
        .filter(|item| item.shard_id == shard_id)
        .cloned()
        .collect();
    let assigned_count = assigned.len();
    let queue = Arc::new(Mutex::new(VecDeque::from(assigned)));
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let execution_build = capture_build_provenance()?;
    let worker_count = threads.min(assigned_count.max(1));
    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            let outcomes = Arc::clone(&outcomes);
            let execution_build = &execution_build;
            scope.spawn(move || {
                loop {
                    let item = queue.lock().expect("work queue poisoned").pop_front();
                    let Some(item) = item else { break };
                    let outcome = execute_one(bundle, plan, &item, results_root, execution_build);
                    if let Err(error) = &outcome {
                        let _ = record_failure(results_root, plan, &item, error);
                    }
                    outcomes
                        .lock()
                        .expect("outcome queue poisoned")
                        .push((item.match_id, outcome));
                }
            });
        }
    });
    let outcomes = Arc::try_unwrap(outcomes)
        .expect("all scoped workers have exited")
        .into_inner()
        .expect("outcome queue poisoned");
    let mut summary = ShardRunSummary {
        shard_id,
        assigned: assigned_count,
        computed: 0,
        already_valid: 0,
        failed: 0,
        worker_threads: worker_count,
    };
    let mut errors = Vec::new();
    for (match_id, outcome) in outcomes {
        match outcome {
            Ok(WorkOutcome::Computed) => summary.computed += 1,
            Ok(WorkOutcome::AlreadyValid) => summary.already_valid += 1,
            Err(error) => {
                summary.failed += 1;
                errors.push(format!("{match_id}: {error}"));
            }
        }
    }
    if errors.is_empty() {
        Ok(summary)
    } else {
        Err(Error::Judge(format!(
            "{} match(es) failed in shard {shard_id}: {}",
            errors.len(),
            errors.join("; ")
        )))
    }
}

fn execute_one(
    bundle: &LoadedBundle,
    plan: &TournamentPlan,
    item: &MatchPlan,
    results_root: &Path,
    execution_build: &BuildProvenance,
) -> Result<WorkOutcome> {
    let final_path = result_path(results_root, &item.match_id);
    if final_path.exists() {
        let artifact = read_artifact(&final_path)?;
        validate_artifact(&artifact, bundle, plan, item)?;
        validate_or_repair_checksum(&final_path)?;
        return Ok(WorkOutcome::AlreadyValid);
    }
    let evaluation = &bundle.evaluations[&item.evaluation_id];
    let started_unix_milliseconds = unix_milliseconds()?;
    let judged = judge_match(bundle, item)?;
    let completed_unix_milliseconds = unix_milliseconds()?;
    let artifact = MatchArtifact {
        schema_version: MATCH_ARTIFACT_SCHEMA_VERSION,
        complete: true,
        tournament_id: plan.tournament_id.clone(),
        bundle_content_hash: plan.bundle_content_hash.clone(),
        plan_id: plan.plan_id.clone(),
        match_id: item.match_id.clone(),
        metric: plan.metric,
        evaluation_id: item.evaluation_id.clone(),
        system_low: item.system_low.clone(),
        system_high: item.system_high.clone(),
        endpoint_count: evaluation.endpoint_ids.len(),
        positive_count: evaluation.positive_count,
        negative_count: evaluation.negative_count,
        label_vector_hash: item.label_vector_hash.clone(),
        score_hash_low: item.score_hash_low.clone(),
        score_hash_high: item.score_hash_high.clone(),
        assessment_policy_hash: plan.policy_hash.clone(),
        seed: item.seed,
        seed_derivation_version: plan.seed_derivation_version,
        supported_ap: PackageIdentity {
            package: "supported-ap".into(),
            version: supported_ap::PACKAGE_VERSION.into(),
            scientific_schema: supported_ap::MANUSCRIPT_VERSION.into(),
        },
        organizer: PackageIdentity {
            package: env!("CARGO_PKG_NAME").into(),
            version: env!("CARGO_PKG_VERSION").into(),
            scientific_schema: format!("match-v{MATCH_ARTIFACT_SCHEMA_VERSION}"),
        },
        plan_build: plan.organizer_build.clone(),
        execution_build: execution_build.clone(),
        judge_execution: "sequential_with_match_level_parallelism".into(),
        started_unix_milliseconds,
        completed_unix_milliseconds,
        judged,
    };
    validate_artifact(&artifact, bundle, plan, item)?;
    publish_artifact(&final_path, &artifact)?;
    Ok(WorkOutcome::Computed)
}

pub fn validate_artifact(
    artifact: &MatchArtifact,
    bundle: &LoadedBundle,
    plan: &TournamentPlan,
    item: &MatchPlan,
) -> Result<()> {
    let evaluation = bundle.evaluations.get(&item.evaluation_id).ok_or_else(|| {
        Error::ArtifactConflict(format!("unknown evaluation in match {}", item.match_id))
    })?;
    if artifact.schema_version != MATCH_ARTIFACT_SCHEMA_VERSION
        || !artifact.complete
        || artifact.tournament_id != plan.tournament_id
        || artifact.bundle_content_hash != plan.bundle_content_hash
        || artifact.plan_id != plan.plan_id
        || artifact.match_id != item.match_id
        || artifact.metric != plan.metric
        || artifact.evaluation_id != item.evaluation_id
        || artifact.system_low != item.system_low
        || artifact.system_high != item.system_high
        || artifact.endpoint_count != evaluation.endpoint_ids.len()
        || artifact.positive_count != evaluation.positive_count
        || artifact.negative_count != evaluation.negative_count
        || artifact.label_vector_hash != item.label_vector_hash
        || artifact.score_hash_low != item.score_hash_low
        || artifact.score_hash_high != item.score_hash_high
        || artifact.assessment_policy_hash != plan.policy_hash
        || artifact.seed != item.seed
        || artifact.seed_derivation_version != plan.seed_derivation_version
        || artifact.supported_ap.package != "supported-ap"
        || artifact.supported_ap.version != supported_ap::PACKAGE_VERSION
        || artifact.supported_ap.scientific_schema != supported_ap::MANUSCRIPT_VERSION
        || artifact.organizer.package != env!("CARGO_PKG_NAME")
        || artifact.plan_build != plan.organizer_build
    {
        return Err(Error::ArtifactConflict(format!(
            "result identity or provenance mismatch for {}",
            item.match_id
        )));
    }
    let directions = [
        &artifact.judged.low_over_high,
        &artifact.judged.high_over_low,
    ];
    if directions[0].winner != item.system_low
        || directions[0].loser != item.system_high
        || directions[1].winner != item.system_high
        || directions[1].loser != item.system_low
    {
        return Err(Error::ArtifactConflict(format!(
            "directed outcomes have wrong orientation for {}",
            item.match_id
        )));
    }
    Ok(())
}

pub fn read_artifact(path: &Path) -> Result<MatchArtifact> {
    let file = File::open(path).map_err(|error| crate::error::io(path, error))?;
    serde_json::from_reader(file).map_err(|source| Error::Json {
        path: path.to_owned(),
        source,
    })
}

fn publish_artifact(final_path: &Path, artifact: &MatchArtifact) -> Result<()> {
    let bytes = {
        let mut bytes = serde_json::to_vec_pretty(artifact).map_err(|source| Error::Json {
            path: final_path.to_owned(),
            source,
        })?;
        bytes.push(b'\n');
        bytes
    };
    let _: MatchArtifact = serde_json::from_slice(&bytes).map_err(|source| Error::Json {
        path: final_path.to_owned(),
        source,
    })?;
    let temporary = unique_temporary_path(final_path)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| crate::error::io(&temporary, error))?;
    file.write_all(&bytes)
        .map_err(|error| crate::error::io(&temporary, error))?;
    file.sync_all()
        .map_err(|error| crate::error::io(&temporary, error))?;
    drop(file);
    let reopened = read_artifact(&temporary)?;
    if !reopened.complete
        || reopened.schema_version != artifact.schema_version
        || reopened.match_id != artifact.match_id
        || reopened.plan_id != artifact.plan_id
        || reopened.judged.low_over_high.verdict != artifact.judged.low_over_high.verdict
        || reopened.judged.high_over_low.verdict != artifact.judged.high_over_low.verdict
    {
        return Err(Error::ArtifactConflict(format!(
            "temporary serialization failed validation for {}",
            artifact.match_id
        )));
    }
    match fs::hard_link(&temporary, final_path) {
        Ok(()) => {
            fs::remove_file(&temporary).map_err(|error| crate::error::io(&temporary, error))?;
        }
        Err(error) if final_path.exists() => {
            fs::remove_file(&temporary).map_err(|remove| crate::error::io(&temporary, remove))?;
            return Err(Error::ArtifactConflict(format!(
                "result appeared concurrently for {}: {error}",
                artifact.match_id
            )));
        }
        Err(error) => return Err(crate::error::io(final_path, error)),
    }
    publish_checksum(final_path, &sha256_bytes(&bytes))
}

fn unique_temporary_path(final_path: &Path) -> Result<PathBuf> {
    let parent = final_path
        .parent()
        .ok_or_else(|| Error::ArtifactConflict("result has no parent directory".into()))?;
    let name = final_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::ArtifactConflict("result filename is not UTF-8".into()))?;
    for attempt in 0..1024_u32 {
        let candidate = parent.join(format!(".{name}.{}.{}.tmp", std::process::id(), attempt));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(Error::ArtifactConflict(
        "could not allocate a unique temporary result path".into(),
    ))
}

fn publish_checksum(result_path: &Path, checksum: &str) -> Result<()> {
    let path = checksum_path(result_path);
    if path.exists() {
        let existing = fs::read_to_string(&path).map_err(|error| crate::error::io(&path, error))?;
        if existing.trim() == checksum {
            return Ok(());
        }
        return Err(Error::ArtifactConflict(format!(
            "conflicting checksum sidecar {}",
            path.display()
        )));
    }
    let temporary = unique_temporary_path(&path)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| crate::error::io(&temporary, error))?;
    writeln!(file, "{checksum}").map_err(|error| crate::error::io(&temporary, error))?;
    file.sync_all()
        .map_err(|error| crate::error::io(&temporary, error))?;
    match fs::hard_link(&temporary, &path) {
        Ok(()) => {
            fs::remove_file(&temporary).map_err(|error| crate::error::io(&temporary, error))?;
            Ok(())
        }
        Err(_) if path.exists() => {
            fs::remove_file(&temporary).map_err(|error| crate::error::io(&temporary, error))?;
            let existing =
                fs::read_to_string(&path).map_err(|error| crate::error::io(&path, error))?;
            if existing.trim() == checksum {
                Ok(())
            } else {
                Err(Error::ArtifactConflict(format!(
                    "conflicting checksum sidecar {}",
                    path.display()
                )))
            }
        }
        Err(error) => Err(crate::error::io(&path, error)),
    }
}

pub fn validate_or_repair_checksum(result_path: &Path) -> Result<()> {
    let actual = sha256_file(result_path)?;
    let sidecar = checksum_path(result_path);
    if sidecar.exists() {
        let declared =
            fs::read_to_string(&sidecar).map_err(|error| crate::error::io(&sidecar, error))?;
        if declared.trim() != actual {
            return Err(Error::ArtifactConflict(format!(
                "checksum mismatch for {}",
                result_path.display()
            )));
        }
        Ok(())
    } else {
        publish_checksum(result_path, &actual)
    }
}

pub fn validate_checksum(result_path: &Path) -> Result<()> {
    let actual = sha256_file(result_path)?;
    let sidecar = checksum_path(result_path);
    if !sidecar.is_file() {
        return Err(Error::ArtifactConflict(format!(
            "missing checksum sidecar for {}",
            result_path.display()
        )));
    }
    let declared =
        fs::read_to_string(&sidecar).map_err(|error| crate::error::io(&sidecar, error))?;
    if declared.trim() != actual {
        return Err(Error::ArtifactConflict(format!(
            "checksum mismatch for {}",
            result_path.display()
        )));
    }
    Ok(())
}

fn record_failure(
    results_root: &Path,
    plan: &TournamentPlan,
    item: &MatchPlan,
    error: &Error,
) -> Result<()> {
    let path = results_root
        .join("failures")
        .join(format!("{}.json", item.match_id));
    let failure = OperationalFailure {
        schema_version: 1,
        match_id: item.match_id.clone(),
        plan_id: plan.plan_id.clone(),
        error: error.to_string(),
    };
    let bytes = serde_json::to_vec_pretty(&failure).map_err(|source| Error::Json {
        path: path.clone(),
        source,
    })?;
    let temporary = unique_temporary_path(&path)?;
    fs::write(&temporary, bytes)
        .map_err(|write_error| crate::error::io(&temporary, write_error))?;
    if path.exists() {
        return Ok(());
    }
    fs::rename(&temporary, &path).map_err(|write_error| crate::error::io(&path, write_error))
}

pub fn artifact_content_hash(artifact: &MatchArtifact) -> Result<String> {
    hash_serializable(artifact)
}

fn unix_milliseconds() -> Result<u128> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|error| Error::Judge(format!("system clock precedes Unix epoch: {error}")))
}
