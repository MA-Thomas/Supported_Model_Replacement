use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use rayon::prelude::*;
use serde::Serialize;

use super::certificate::{
    EvidenceIndex, ExclusionWitness, SelectionCertificate, audit_selection, complete_graph,
    component_witness, key, load_evidence, lookup,
};
use super::plan::{
    AcceleratedPlan, ContextExecution, OutputScope, validate_accelerated_plan, write_json,
};
use crate::artifact::{
    WorkOutcome, create_staging_directory, execute_one, publish_staging_directory,
};
use crate::input::LoadedBundle;
use crate::plan::{MatchPlan, match_for_pair};
use crate::provenance::{BuildProvenance, capture_build_provenance};
use crate::{DirectedVerdict, Error, Result};

#[derive(Debug, Serialize)]
pub struct AcceleratedRunSummary {
    pub computed: usize,
    pub already_valid: usize,
    pub assessed_matches: usize,
    pub possible_matches: usize,
    pub survivors: Vec<String>,
    pub output_scope: OutputScope,
}

pub(super) struct Coordinator<'a> {
    pub(super) bundle: &'a LoadedBundle,
    pub(super) plan: &'a AcceleratedPlan,
    pub(super) results: &'a Path,
    pool: rayon::ThreadPool,
    build: BuildProvenance,
    execution: String,
    pub(super) evidence: EvidenceIndex,
    pub(super) computed: usize,
    pub(super) already_valid: usize,
}

impl Coordinator<'_> {
    /// Workers see only an immutable batch. Commit in request order, never in
    /// completion order. The complete batch is validated before state changes.
    pub(super) fn batch(&mut self, requests: &[MatchPlan]) -> Result<()> {
        if requests.is_empty() {
            return Ok(());
        }
        let outcomes = self.pool.install(|| {
            requests
                .par_iter()
                .map(|item| {
                    let outcome = execute_one(
                        self.bundle,
                        &self.plan.binding(),
                        item,
                        self.results,
                        &self.build,
                        &self.execution,
                    )?;
                    let evidence = load_evidence(
                        self.bundle,
                        self.plan,
                        self.results,
                        &item.evaluation_id,
                        &item.system_low,
                        &item.system_high,
                    )?;
                    Ok((outcome, evidence))
                })
                .collect::<Vec<Result<_>>>()
        });
        let committed = outcomes.into_iter().collect::<Result<Vec<_>>>()?;
        for (outcome, evidence) in committed {
            match outcome {
                WorkOutcome::Computed => self.computed += 1,
                WorkOutcome::AlreadyValid => self.already_valid += 1,
            }
            let r = &evidence.reference;
            self.evidence.insert(
                key(&r.evaluation_id, &r.system_low, &r.system_high),
                evidence,
            );
        }
        Ok(())
    }

    fn complete_pairs(&mut self, evaluation: &str, systems: &[String]) -> Result<()> {
        let mut batch = Vec::with_capacity(self.plan.options.batch_size);
        for (i, a) in systems.iter().enumerate() {
            for b in &systems[i + 1..] {
                if lookup(&self.evidence, evaluation, a, b).is_some() {
                    continue;
                }
                batch.push(match_for_pair(self.bundle, evaluation, a, b)?);
                if batch.len() == self.plan.options.batch_size {
                    self.batch(&batch)?;
                    batch.clear();
                }
            }
        }
        self.batch(&batch)
    }

    fn witness(
        &self,
        evaluation: &str,
        target: &str,
        challengers: &[String],
    ) -> Option<ExclusionWitness> {
        challengers
            .iter()
            .filter(|c| *c != target)
            .find_map(|challenger| {
                let e = lookup(&self.evidence, evaluation, challenger, target)?;
                (e.verdict(challenger) == DirectedVerdict::Supported).then(|| {
                    ExclusionWitness::SupportedIncoming {
                        evaluation_id: evaluation.into(),
                        challenger: challenger.clone(),
                        match_id: e.reference.match_id.clone(),
                    }
                })
            })
    }
}

/// Resumable local execution. Existing immutable pair artifacts are validated
/// and reused; scheduling state is deterministically reconstructed from them.
/// The output directory is published only after an independent complete audit.
pub fn run_accelerated(
    bundle: &LoadedBundle,
    plan: &AcceleratedPlan,
    results: &Path,
    output: &Path,
    threads: usize,
) -> Result<AcceleratedRunSummary> {
    if output.exists() {
        return Err(Error::ArtifactConflict(format!(
            "output already exists: {}",
            output.display()
        )));
    }
    let mut coordinator = coordinator(bundle, plan, results, threads)?;
    let exclusions = assess_targets(&mut coordinator, &plan.systems)?;
    let survivors: Vec<_> = plan
        .systems
        .iter()
        .filter(|s| !exclusions.contains_key(*s))
        .cloned()
        .collect();
    if plan.options.output_scope == OutputScope::SurvivorSetAndOperationalInputs {
        for evaluation in plan.contexts.keys() {
            coordinator.complete_pairs(evaluation, &survivors)?;
        }
    }
    let certificate = SelectionCertificate {
        schema_version: 1,
        plan_id: plan.plan_id.clone(),
        output_scope: plan.options.output_scope,
        matches: coordinator
            .evidence
            .values()
            .map(|e| e.reference.clone())
            .collect(),
        exclusions,
        survivors: survivors.clone(),
    };
    let audited = audit_selection(bundle, plan, results, &certificate)?;
    let summary = AcceleratedRunSummary {
        computed: coordinator.computed,
        already_valid: coordinator.already_valid,
        assessed_matches: certificate.matches.len(),
        possible_matches: plan.systems.len() * (plan.systems.len() - 1) / 2 * plan.contexts.len(),
        survivors,
        output_scope: plan.options.output_scope,
    };
    let staging = create_staging_directory(output)?;
    write_json(&staging.join("selection_certificate.json"), &certificate)?;
    write_json(&staging.join("audited_selection.json"), &audited)?;
    write_json(&staging.join("run_summary.json"), &summary)?;
    publish_staging_directory(&staging, output)?;
    Ok(summary)
}

pub(super) fn coordinator<'a>(
    bundle: &'a LoadedBundle,
    plan: &'a AcceleratedPlan,
    results: &'a Path,
    threads: usize,
) -> Result<Coordinator<'a>> {
    validate_accelerated_plan(plan, bundle)?;
    if threads == 0 {
        return Err(Error::InvalidPlan(
            "worker thread count must be positive".into(),
        ));
    }
    fs::create_dir_all(results.join("matches")).map_err(|e| crate::error::io(results, e))?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|e| Error::Judge(e.to_string()))?;
    Ok(Coordinator {
        bundle,
        plan,
        results,
        pool,
        build: capture_build_provenance()?,
        execution: format!("rayon_shared_pool:{threads}_threads"),
        evidence: EvidenceIndex::new(),
        computed: 0,
        already_valid: 0,
    })
}

pub(super) fn assess_targets(
    coordinator: &mut Coordinator<'_>,
    targets: &[String],
) -> Result<BTreeMap<String, ExclusionWitness>> {
    let bundle = coordinator.bundle;
    let plan = coordinator.plan;
    let mut exclusions = BTreeMap::new();
    if targets.is_empty() {
        return Ok(exclusions);
    }
    // General contexts always get a full-roster SCC analysis, including cycles.
    for (evaluation, context) in &plan.contexts {
        if matches!(context, ContextExecution::Exhaustive { .. }) {
            coordinator.complete_pairs(evaluation, &plan.systems)?;
            let graph = complete_graph(&plan.systems, evaluation, &coordinator.evidence)?;
            for target in &plan.systems {
                if let Some(witness) = component_witness(evaluation, &graph, target) {
                    exclusions.entry(target.clone()).or_insert(witness);
                }
            }
        }
    }
    let delta = bundle.spec.evidence_policy.magnitude_threshold;
    for (evaluation, context) in &plan.contexts {
        if matches!(context, ContextExecution::Exhaustive { .. }) {
            continue;
        }
        let order = context.ordered_systems(&plan.systems);
        for target in order.iter().filter(|s| targets.contains(s)) {
            if exclusions.contains_key(target) {
                continue;
            }
            if let Some(witness) = coordinator.witness(evaluation, target, &order) {
                exclusions.insert(target.clone(), witness);
                continue;
            }
            let mut batch = Vec::with_capacity(plan.options.batch_size);
            // The challenger roster is NEVER filtered by exclusions. Supported
            // replacement need not be transitive, even in an ordered context.
            for challenger in &order {
                if challenger == target
                    || context.rejects(challenger, target, delta)
                    || lookup(&coordinator.evidence, evaluation, challenger, target).is_some()
                {
                    continue;
                }
                batch.push(match_for_pair(bundle, evaluation, challenger, target)?);
                if batch.len() == plan.options.batch_size {
                    coordinator.batch(&batch)?;
                    let defeated = batch.iter().any(|item| {
                        let challenger = if item.system_low == *target {
                            &item.system_high
                        } else {
                            &item.system_low
                        };
                        lookup(&coordinator.evidence, evaluation, challenger, target)
                            .is_some_and(|e| e.verdict(challenger) == DirectedVerdict::Supported)
                    });
                    batch.clear();
                    if defeated {
                        break;
                    }
                }
            }
            coordinator.batch(&batch)?;
            if let Some(witness) = coordinator.witness(evaluation, target, &order) {
                exclusions.insert(target.clone(), witness);
            }
        }
    }
    Ok(exclusions)
}
