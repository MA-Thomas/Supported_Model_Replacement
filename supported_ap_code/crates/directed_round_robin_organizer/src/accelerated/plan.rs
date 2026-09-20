use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use supported_ap::Prevalence;

use crate::artifact::{RunBinding, create_staging_directory, publish_staging_directory};
use crate::identity::hash_serializable;
use crate::input::LoadedBundle;
use crate::plan::{PlanEvaluationInput, tournament_id};
use crate::provenance::{BuildProvenance, capture_build_provenance};
use crate::{Error, MetricKind, Result, SelectionStrategy};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputScope {
    SurvivorSet,
    /// Complete numerical match reports within the final S0, in every context.
    #[default]
    SurvivorSetAndOperationalInputs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceleratedOptions {
    pub output_scope: OutputScope,
    /// Maximum immutable work batch. Changing this may change extra work,
    /// but cannot change selection or pair seeds. It is bound into the plan.
    pub batch_size: usize,
    /// Explicit opt-out only; callers cannot assert an ordering capability.
    pub exhaustive_contexts: BTreeSet<String>,
}

impl Default for AcceleratedOptions {
    fn default() -> Self {
        Self {
            output_scope: OutputScope::default(),
            batch_size: 16,
            exhaustive_contexts: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContextExecution {
    /// Values are recomputed by the auditor using the empirical gate's kernel.
    OrderedCnap {
        prevalence: Prevalence,
        values: BTreeMap<String, f64>,
    },
    OrderedAuroc {
        doubled_credits: BTreeMap<String, u64>,
    },
    Exhaustive {
        reason: String,
    },
}

impl ContextExecution {
    /// A rejected direction cannot pass the observed magnitude gate. This is
    /// not an assessed NotSupported verdict and must not be serialized as one.
    pub(crate) fn rejects(&self, winner: &str, loser: &str, delta: f64) -> bool {
        match self {
            Self::OrderedCnap { values, .. } => values[winner] - values[loser] <= delta,
            Self::OrderedAuroc { doubled_credits } => {
                doubled_credits[winner] <= doubled_credits[loser]
            }
            Self::Exhaustive { .. } => false,
        }
    }

    pub(crate) fn ordered_systems(&self, systems: &[String]) -> Vec<String> {
        let mut order = systems.to_vec();
        order.sort_by(|a, b| {
            let comparison = match self {
                Self::OrderedCnap { values, .. } => values[b].total_cmp(&values[a]),
                Self::OrderedAuroc { doubled_credits } => {
                    doubled_credits[b].cmp(&doubled_credits[a])
                }
                Self::Exhaustive { .. } => std::cmp::Ordering::Equal,
            };
            comparison.then_with(|| a.cmp(b))
        });
        order
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceleratedPlan {
    pub schema_version: u32,
    pub plan_id: String,
    pub tournament_id: String,
    pub bundle_content_hash: String,
    pub policy_hash: String,
    pub metric: MetricKind,
    pub seed_derivation_version: u32,
    pub supported_ap_version: String,
    pub scientific_schema: String,
    pub strategy: SelectionStrategy,
    pub options: AcceleratedOptions,
    pub systems: Vec<String>,
    pub evaluation_inputs: Vec<PlanEvaluationInput>,
    pub contexts: BTreeMap<String, ContextExecution>,
    pub organizer_build: BuildProvenance,
}

impl AcceleratedPlan {
    pub(crate) fn binding(&self) -> RunBinding<'_> {
        RunBinding {
            tournament_id: &self.tournament_id,
            bundle_content_hash: &self.bundle_content_hash,
            plan_id: &self.plan_id,
            metric: self.metric,
            policy_hash: &self.policy_hash,
            seed_derivation_version: self.seed_derivation_version,
            organizer_build: &self.organizer_build,
        }
    }
}

pub fn plan_accelerated(
    bundle: &LoadedBundle,
    options: AcceleratedOptions,
) -> Result<AcceleratedPlan> {
    prepare(bundle, options, capture_build_provenance()?)
}

fn prepare(
    bundle: &LoadedBundle,
    options: AcceleratedOptions,
    organizer_build: BuildProvenance,
) -> Result<AcceleratedPlan> {
    bundle.spec.validate()?;
    if options.batch_size == 0
        || options
            .exhaustive_contexts
            .iter()
            .any(|id| !bundle.evaluations.contains_key(id))
    {
        return Err(Error::InvalidPlan(
            "invalid batch size or unknown exhaustive context".into(),
        ));
    }
    let contexts = bundle
        .evaluations
        .iter()
        .map(|(id, evaluation)| {
            let context = if options.exhaustive_contexts.contains(id) {
                ContextExecution::Exhaustive {
                    reason: "explicit context opt-out".into(),
                }
            } else {
                crate::judge::prepare_context_execution(bundle, evaluation)
            };
            (id.clone(), context)
        })
        .collect();
    let mut plan = AcceleratedPlan {
        schema_version: 1,
        plan_id: String::new(),
        tournament_id: tournament_id(bundle)?,
        bundle_content_hash: bundle.manifest.bundle_content_hash.clone(),
        policy_hash: bundle.policy_hash.clone(),
        metric: bundle.spec.metric,
        seed_derivation_version: bundle.spec.seed_derivation_version,
        supported_ap_version: supported_ap::PACKAGE_VERSION.into(),
        scientific_schema: supported_ap::MANUSCRIPT_VERSION.into(),
        strategy: SelectionStrategy::CandidateConservative,
        options,
        systems: bundle
            .registry
            .sorted_systems()
            .iter()
            .map(|s| s.system_id.clone())
            .collect(),
        evaluation_inputs: bundle
            .evaluations
            .values()
            .map(|e| PlanEvaluationInput {
                evaluation_id: e.evaluation_id.clone(),
                raw_endpoints_hash: e.raw_endpoints_hash.clone(),
                raw_scores_hash: e.raw_scores_hash.clone(),
                canonical_label_vector_hash: e.label_vector_hash.clone(),
            })
            .collect(),
        contexts,
        organizer_build,
    };
    plan.plan_id = hash_serializable(&("accelerated-plan-v1", &plan))?;
    Ok(plan)
}

/// Reconstructs the entire compact plan, including every numerical order key.
pub fn validate_accelerated_plan(plan: &AcceleratedPlan, bundle: &LoadedBundle) -> Result<()> {
    if *plan != prepare(bundle, plan.options.clone(), plan.organizer_build.clone())? {
        return Err(Error::InvalidPlan(
            "accelerated plan differs from judge-derived recipe".into(),
        ));
    }
    Ok(())
}

pub(crate) fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).map_err(|e| crate::error::io(path, e))?;
    serde_json::from_slice(&bytes).map_err(|source| Error::Json {
        path: path.to_owned(),
        source,
    })
}

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    use std::io::Write;
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|source| Error::Json {
        path: path.to_owned(),
        source,
    })?;
    bytes.push(b'\n');
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| crate::error::io(path, e))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| crate::error::io(path, e))
}

pub fn write_accelerated_plan(plan: &AcceleratedPlan, directory: &Path) -> Result<()> {
    let staging = create_staging_directory(directory)?;
    write_json(&staging.join("accelerated_plan.json"), plan)?;
    publish_staging_directory(&staging, directory)
}

pub fn read_accelerated_plan(directory: &Path) -> Result<AcceleratedPlan> {
    read_json(&directory.join("accelerated_plan.json"))
}
