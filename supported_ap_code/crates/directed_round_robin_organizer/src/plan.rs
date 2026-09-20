use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::identity::{hash_serializable, seed_from_hash_material};
use crate::input::LoadedBundle;
use crate::provenance::{BuildProvenance, capture_build_provenance};
use crate::spec::MetricKind;
use crate::{Error, Result};

pub const PLAN_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShardingPolicy {
    RequestedShardCount { shard_count: usize },
    TargetMatchesPerShard { matches_per_shard: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchPlan {
    pub match_id: String,
    pub evaluation_id: String,
    pub system_low: String,
    pub system_high: String,
    pub score_hash_low: String,
    pub score_hash_high: String,
    pub label_vector_hash: String,
    pub seed: u64,
    pub shard_id: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanEvaluationInput {
    pub evaluation_id: String,
    pub raw_endpoints_hash: String,
    pub raw_scores_hash: String,
    pub canonical_label_vector_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TournamentPlan {
    pub schema_version: u32,
    pub plan_id: String,
    pub tournament_id: String,
    pub bundle_content_hash: String,
    pub metric: MetricKind,
    pub policy_hash: String,
    pub seed_derivation_version: u32,
    pub sharding_policy: ShardingPolicy,
    pub shard_count: usize,
    pub evaluation_count: usize,
    pub system_count: usize,
    pub expected_match_count: usize,
    pub evaluation_inputs: Vec<PlanEvaluationInput>,
    pub organizer_build: BuildProvenance,
    pub matches: Vec<MatchPlan>,
}

#[derive(Serialize)]
struct MatchIdentity<'a> {
    schema_version: u32,
    tournament_content_hash: &'a str,
    metric: MetricKind,
    evaluation_id: &'a str,
    system_low: &'a str,
    system_high: &'a str,
    score_hash_low: &'a str,
    score_hash_high: &'a str,
    label_vector_hash: &'a str,
    assessment_policy_hash: &'a str,
    seed_derivation_version: u32,
}

#[derive(Serialize)]
struct SeedMaterial<'a> {
    master_seed: u64,
    metric: MetricKind,
    evaluation_id: &'a str,
    system_low: &'a str,
    system_high: &'a str,
    seed_derivation_version: u32,
}

#[derive(Serialize)]
struct PlanIdentity<'a> {
    schema_version: u32,
    tournament_id: &'a str,
    sharding_policy: &'a ShardingPolicy,
    assignments: Vec<(&'a str, usize)>,
}

pub fn plan_with_shard_count(bundle: &LoadedBundle, shard_count: usize) -> Result<TournamentPlan> {
    if shard_count == 0 {
        return Err(Error::InvalidPlan("shard count must be positive".into()));
    }
    let mut matches = enumerate_matches(bundle)?;
    let actual_shards = shard_count.min(matches.len());
    let match_count = matches.len();
    for (index, match_plan) in matches.iter_mut().enumerate() {
        match_plan.shard_id = index * actual_shards / match_count;
    }
    finish_plan(
        bundle,
        ShardingPolicy::RequestedShardCount { shard_count },
        actual_shards,
        matches,
    )
}

pub fn plan_with_match_target(
    bundle: &LoadedBundle,
    matches_per_shard: usize,
) -> Result<TournamentPlan> {
    if matches_per_shard == 0 {
        return Err(Error::InvalidPlan(
            "target matches per shard must be positive".into(),
        ));
    }
    let mut matches = enumerate_matches(bundle)?;
    for (index, match_plan) in matches.iter_mut().enumerate() {
        match_plan.shard_id = index / matches_per_shard;
    }
    let shard_count = matches.last().map_or(0, |item| item.shard_id + 1);
    finish_plan(
        bundle,
        ShardingPolicy::TargetMatchesPerShard { matches_per_shard },
        shard_count,
        matches,
    )
}

/// Constructs the canonical pair independently of any scheduling or sharding.
/// Match identities and seeds are identical in exhaustive and accelerated runs.
pub fn match_for_pair(
    bundle: &LoadedBundle,
    evaluation_id: &str,
    a: &str,
    b: &str,
) -> Result<MatchPlan> {
    if a == b {
        return Err(Error::InvalidPlan("a match needs distinct systems".into()));
    }
    let (low, high) = if a < b { (a, b) } else { (b, a) };
    let evaluation = bundle
        .evaluations
        .get(evaluation_id)
        .ok_or_else(|| Error::InvalidPlan(format!("unknown evaluation {evaluation_id}")))?;
    let score_hash_low = evaluation
        .score_vector_hashes
        .get(low)
        .ok_or_else(|| Error::InvalidPlan(format!("unknown system {low}")))?
        .clone();
    let score_hash_high = evaluation
        .score_vector_hashes
        .get(high)
        .ok_or_else(|| Error::InvalidPlan(format!("unknown system {high}")))?
        .clone();
    let identity = MatchIdentity {
        schema_version: PLAN_SCHEMA_VERSION,
        tournament_content_hash: &bundle.manifest.bundle_content_hash,
        metric: bundle.spec.metric,
        evaluation_id,
        system_low: low,
        system_high: high,
        score_hash_low: &score_hash_low,
        score_hash_high: &score_hash_high,
        label_vector_hash: &evaluation.label_vector_hash,
        assessment_policy_hash: &bundle.policy_hash,
        seed_derivation_version: bundle.spec.seed_derivation_version,
    };
    let seed = seed_from_hash_material(&SeedMaterial {
        master_seed: bundle.spec.master_seed,
        metric: bundle.spec.metric,
        evaluation_id,
        system_low: low,
        system_high: high,
        seed_derivation_version: bundle.spec.seed_derivation_version,
    })?;
    Ok(MatchPlan {
        match_id: hash_serializable(&identity)?,
        evaluation_id: evaluation_id.into(),
        system_low: low.into(),
        system_high: high.into(),
        score_hash_low,
        score_hash_high,
        label_vector_hash: evaluation.label_vector_hash.clone(),
        seed,
        shard_id: 0,
    })
}

pub fn tournament_id(bundle: &LoadedBundle) -> Result<String> {
    hash_serializable(&(
        "directed-round-robin-tournament-v1",
        &bundle.manifest.bundle_content_hash,
        bundle.spec.metric,
        &bundle.policy_hash,
    ))
}

fn enumerate_matches(bundle: &LoadedBundle) -> Result<Vec<MatchPlan>> {
    let systems = bundle.registry.sorted_systems();
    let pairs_per_evaluation = systems.len() * (systems.len() - 1) / 2;
    let expected = bundle.evaluations.len() * pairs_per_evaluation;
    let mut matches_by_evaluation = Vec::with_capacity(bundle.evaluations.len());
    for evaluation_id in bundle.evaluations.keys() {
        let mut evaluation_matches = Vec::with_capacity(pairs_per_evaluation);
        for low_index in 0..systems.len() {
            for high_index in low_index + 1..systems.len() {
                evaluation_matches.push(match_for_pair(
                    bundle,
                    evaluation_id,
                    &systems[low_index].system_id,
                    &systems[high_index].system_id,
                )?);
            }
        }
        // Content ordering scatters system pairs without introducing an
        // execution-order seed. Interleaving these lists below guarantees
        // that small shards cover every evaluation when their size permits.
        evaluation_matches.sort_by(|left, right| left.match_id.cmp(&right.match_id));
        matches_by_evaluation.push(evaluation_matches);
    }
    let mut matches = Vec::with_capacity(expected);
    for pair_slot in 0..pairs_per_evaluation {
        for evaluation_matches in &matches_by_evaluation {
            matches.push(evaluation_matches[pair_slot].clone());
        }
    }
    if matches.len() != expected {
        return Err(Error::InvalidPlan(format!(
            "enumerated {} matches, expected {expected}",
            matches.len()
        )));
    }
    Ok(matches)
}

fn finish_plan(
    bundle: &LoadedBundle,
    sharding_policy: ShardingPolicy,
    shard_count: usize,
    matches: Vec<MatchPlan>,
) -> Result<TournamentPlan> {
    let tournament_id = tournament_id(bundle)?;
    let assignments = matches
        .iter()
        .map(|item| (item.match_id.as_str(), item.shard_id))
        .collect();
    let plan_id = hash_serializable(&PlanIdentity {
        schema_version: PLAN_SCHEMA_VERSION,
        tournament_id: &tournament_id,
        sharding_policy: &sharding_policy,
        assignments,
    })?;
    let plan = TournamentPlan {
        schema_version: PLAN_SCHEMA_VERSION,
        plan_id,
        tournament_id,
        bundle_content_hash: bundle.manifest.bundle_content_hash.clone(),
        metric: bundle.spec.metric,
        policy_hash: bundle.policy_hash.clone(),
        seed_derivation_version: bundle.spec.seed_derivation_version,
        sharding_policy,
        shard_count,
        evaluation_count: bundle.evaluations.len(),
        system_count: bundle.registry.systems.len(),
        expected_match_count: matches.len(),
        evaluation_inputs: bundle
            .evaluations
            .values()
            .map(|evaluation| PlanEvaluationInput {
                evaluation_id: evaluation.evaluation_id.clone(),
                raw_endpoints_hash: evaluation.raw_endpoints_hash.clone(),
                raw_scores_hash: evaluation.raw_scores_hash.clone(),
                canonical_label_vector_hash: evaluation.label_vector_hash.clone(),
            })
            .collect(),
        organizer_build: capture_build_provenance()?,
        matches,
    };
    validate_plan(&plan, bundle)?;
    Ok(plan)
}

pub fn validate_plan(plan: &TournamentPlan, bundle: &LoadedBundle) -> Result<()> {
    if plan.schema_version != PLAN_SCHEMA_VERSION
        || plan.bundle_content_hash != bundle.manifest.bundle_content_hash
        || plan.metric != bundle.spec.metric
        || plan.policy_hash != bundle.policy_hash
        || plan.seed_derivation_version != bundle.spec.seed_derivation_version
    {
        return Err(Error::InvalidPlan(
            "plan identity does not match bundle".into(),
        ));
    }
    let expected = bundle.evaluations.len()
        * bundle.registry.systems.len()
        * (bundle.registry.systems.len() - 1)
        / 2;
    if plan.expected_match_count != expected
        || plan.matches.len() != expected
        || plan.evaluation_count != bundle.evaluations.len()
        || plan.system_count != bundle.registry.systems.len()
        || plan.shard_count == 0
    {
        return Err(Error::InvalidPlan(
            "plan counts do not match complete round robin".into(),
        ));
    }
    let plan_evaluations: BTreeMap<_, _> = plan
        .evaluation_inputs
        .iter()
        .map(|evaluation| (evaluation.evaluation_id.as_str(), evaluation))
        .collect();
    if plan_evaluations.len() != bundle.evaluations.len() {
        return Err(Error::InvalidPlan(
            "plan evaluation input registry is incomplete or duplicated".into(),
        ));
    }
    for (evaluation_id, loaded) in &bundle.evaluations {
        let Some(planned) = plan_evaluations.get(evaluation_id.as_str()) else {
            return Err(Error::InvalidPlan(format!(
                "plan is missing evaluation input {evaluation_id}"
            )));
        };
        if planned.raw_endpoints_hash != loaded.raw_endpoints_hash
            || planned.raw_scores_hash != loaded.raw_scores_hash
            || planned.canonical_label_vector_hash != loaded.label_vector_hash
        {
            return Err(Error::InvalidPlan(format!(
                "plan input hashes differ for evaluation {evaluation_id}"
            )));
        }
    }
    let system_ids: BTreeSet<_> = bundle
        .registry
        .systems
        .iter()
        .map(|system| system.system_id.as_str())
        .collect();
    let mut ids = BTreeSet::new();
    let mut pairs = BTreeSet::new();
    for item in &plan.matches {
        if !ids.insert(&item.match_id)
            || item.system_low >= item.system_high
            || !system_ids.contains(item.system_low.as_str())
            || !system_ids.contains(item.system_high.as_str())
            || !bundle.evaluations.contains_key(&item.evaluation_id)
            || !pairs.insert((
                item.evaluation_id.as_str(),
                item.system_low.as_str(),
                item.system_high.as_str(),
            ))
            || item.shard_id >= plan.shard_count
        {
            return Err(Error::InvalidPlan(format!(
                "invalid or duplicate match {}",
                item.match_id
            )));
        }
        let evaluation = &bundle.evaluations[&item.evaluation_id];
        if evaluation.score_vector_hashes[&item.system_low] != item.score_hash_low
            || evaluation.score_vector_hashes[&item.system_high] != item.score_hash_high
            || evaluation.label_vector_hash != item.label_vector_hash
        {
            return Err(Error::InvalidPlan(format!(
                "vector hash mismatch in match {}",
                item.match_id
            )));
        }
    }
    let plan_shards: BTreeSet<_> = plan.matches.iter().map(|item| item.shard_id).collect();
    if plan_shards.len() != plan.shard_count {
        return Err(Error::InvalidPlan("plan contains empty shard IDs".into()));
    }
    Ok(())
}

pub fn write_plan(plan: &TournamentPlan, directory: &Path) -> Result<()> {
    fs::create_dir_all(directory).map_err(|error| crate::error::io(directory, error))?;
    let path = directory.join("plan.json");
    let mut bytes = serde_json::to_vec_pretty(plan).map_err(|source| Error::Json {
        path: path.clone(),
        source,
    })?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| crate::error::io(&path, error))?;
    file.write_all(&bytes)
        .map_err(|error| crate::error::io(&path, error))?;
    file.sync_all()
        .map_err(|error| crate::error::io(&path, error))
}

pub fn read_plan(directory: &Path) -> Result<TournamentPlan> {
    let path = directory.join("plan.json");
    let file = File::open(&path).map_err(|error| crate::error::io(&path, error))?;
    serde_json::from_reader(file).map_err(|source| Error::Json { path, source })
}

pub fn shard_registry(plan: &TournamentPlan) -> BTreeMap<usize, Vec<&MatchPlan>> {
    let mut result: BTreeMap<usize, Vec<&MatchPlan>> = BTreeMap::new();
    for item in &plan.matches {
        result.entry(item.shard_id).or_default().push(item);
    }
    result
}
