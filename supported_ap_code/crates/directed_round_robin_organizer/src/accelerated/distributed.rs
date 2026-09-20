//! Scheduler-neutral distributed execution. Every ordered pair belongs to its
//! lower-potential target; completion starts only after all target workers finish.
use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::certificate::{audit_survivors, load_evidence};
use super::plan::{read_json, write_json};
use super::scheduler::{assess_targets, coordinator};
use super::*;
use crate::artifact::{create_staging_directory, publish_staging_directory};
use crate::identity::hash_serializable;
use crate::input::LoadedBundle;
use crate::plan::match_for_pair;
use crate::{Error, Result};

fn invalid(message: &str) -> Error {
    Error::InvalidPlan(message.into())
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetReceipt {
    schema_version: u32,
    plan_id: String,
    shard_count: usize,
    shard_id: usize,
    targets: Vec<String>,
    exclusions: BTreeMap<String, ExclusionWitness>,
    matches: Vec<MatchReference>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletionReceipt {
    schema_version: u32,
    plan_id: String,
    survivor_certificate_hash: String,
    shard_count: usize,
    shard_id: usize,
    matches: Vec<MatchReference>,
}

fn assignment(plan: &AcceleratedPlan, count: usize, id: usize) -> Result<Vec<String>> {
    if count == 0 || id >= count {
        return Err(invalid("invalid shard count/id"));
    }
    // General contexts retain the existing full-roster SCC algorithm. Other
    // scheduled tasks publish empty receipts instead of duplicating that work.
    if plan
        .contexts
        .values()
        .any(|c| matches!(c, ContextExecution::Exhaustive { .. }))
    {
        return Ok(if id == 0 {
            plan.systems.clone()
        } else {
            vec![]
        });
    }
    let order = plan
        .contexts
        .values()
        .next()
        .ok_or_else(|| invalid("no contexts"))?
        .ordered_systems(&plan.systems);
    Ok(order
        .into_iter()
        .enumerate()
        .filter(|(i, _)| i % count == id)
        .map(|(_, s)| s)
        .collect())
}

fn publish<T: Serialize + for<'de> Deserialize<'de> + PartialEq>(
    root: &Path,
    name: &str,
    value: &T,
) -> Result<()> {
    if root.exists() {
        let previous: T = read_json(&root.join(name))?;
        return if previous == *value {
            Ok(())
        } else {
            Err(invalid("existing distributed output differs"))
        };
    }
    let stage = create_staging_directory(root)?;
    write_json(&stage.join(name), value)?;
    publish_staging_directory(&stage, root)
}

pub fn run_target_shard(
    bundle: &LoadedBundle,
    plan: &AcceleratedPlan,
    results: &Path,
    receipts: &Path,
    shard_count: usize,
    shard_id: usize,
    threads: usize,
) -> Result<()> {
    let targets = assignment(plan, shard_count, shard_id)?;
    let mut worker = coordinator(bundle, plan, results, threads)?;
    let exclusions = assess_targets(&mut worker, &targets)?;
    let receipt = TargetReceipt {
        schema_version: 1,
        plan_id: plan.plan_id.clone(),
        shard_count,
        shard_id,
        targets,
        exclusions,
        matches: worker
            .evidence
            .values()
            .map(|e| e.reference.clone())
            .collect(),
    };
    publish(
        &receipts.join(shard_id.to_string()),
        "targets.json",
        &receipt,
    )
}

pub fn merge_target_shards(
    bundle: &LoadedBundle,
    plan: &AcceleratedPlan,
    results: &Path,
    receipts: &Path,
    shard_count: usize,
    output: &Path,
) -> Result<()> {
    if shard_count == 0 {
        return Err(invalid("zero target shards"));
    }
    let mut exclusions = BTreeMap::new();
    let mut matches = BTreeMap::new();
    for id in 0..shard_count {
        let receipt: TargetReceipt =
            read_json(&receipts.join(id.to_string()).join("targets.json"))?;
        let targets = assignment(plan, shard_count, id)?;
        if receipt.schema_version != 1
            || receipt.plan_id != plan.plan_id
            || receipt.shard_count != shard_count
            || receipt.shard_id != id
            || receipt.targets != targets
            || receipt.exclusions.keys().any(|s| !targets.contains(s))
        {
            return Err(invalid("target receipt identity/coverage mismatch"));
        }
        for (target, witness) in receipt.exclusions {
            exclusions.insert(target, witness);
        }
        for reference in receipt.matches {
            if matches
                .insert(reference.match_id.clone(), reference)
                .is_some()
            {
                return Err(invalid("comparison assigned to multiple target workers"));
            }
        }
    }
    let certificate = SelectionCertificate {
        schema_version: 1,
        plan_id: plan.plan_id.clone(),
        output_scope: OutputScope::SurvivorSet,
        matches: matches.into_values().collect(),
        survivors: plan
            .systems
            .iter()
            .filter(|s| !exclusions.contains_key(*s))
            .cloned()
            .collect(),
        exclusions,
    };
    audit_survivors(bundle, plan, results, &certificate)?;
    publish(output, "selection_certificate.json", &certificate)
}

pub fn run_completion_shard(
    bundle: &LoadedBundle,
    plan: &AcceleratedPlan,
    results: &Path,
    survivors: &Path,
    receipts: &Path,
    shard_count: usize,
    shard_id: usize,
    threads: usize,
) -> Result<()> {
    if shard_count == 0 || shard_id >= shard_count {
        return Err(invalid("invalid completion shard"));
    }
    let certificate = read_certificate(survivors)?;
    audit_survivors(bundle, plan, results, &certificate)?;
    let mut worker = coordinator(bundle, plan, results, threads)?;
    let mut ordinal = 0;
    let mut batch = Vec::new();
    for evaluation in plan.contexts.keys() {
        for (i, a) in certificate.survivors.iter().enumerate() {
            for b in &certificate.survivors[i + 1..] {
                if ordinal % shard_count == shard_id {
                    batch.push(match_for_pair(bundle, evaluation, a, b)?);
                    if batch.len() == plan.options.batch_size {
                        worker.batch(&batch)?;
                        batch.clear();
                    }
                }
                ordinal += 1;
            }
        }
    }
    worker.batch(&batch)?;
    let receipt = CompletionReceipt {
        schema_version: 1,
        plan_id: plan.plan_id.clone(),
        survivor_certificate_hash: hash_serializable(&certificate)?,
        shard_count,
        shard_id,
        matches: worker
            .evidence
            .values()
            .map(|e| e.reference.clone())
            .collect(),
    };
    publish(
        &receipts.join(shard_id.to_string()),
        "completion.json",
        &receipt,
    )
}

pub fn finish_distributed(
    bundle: &LoadedBundle,
    plan: &AcceleratedPlan,
    results: &Path,
    survivors: &Path,
    receipts: &Path,
    shard_count: usize,
    output: &Path,
) -> Result<()> {
    if shard_count == 0 || plan.options.output_scope != OutputScope::SurvivorSetAndOperationalInputs
    {
        return Err(invalid(
            "completion requires operational output scope and positive shard count",
        ));
    }
    let mut certificate = read_certificate(survivors)?;
    audit_survivors(bundle, plan, results, &certificate)?;
    let hash = hash_serializable(&certificate)?;
    let mut matches: BTreeMap<_, _> = certificate
        .matches
        .iter()
        .map(|r| (r.match_id.clone(), r.clone()))
        .collect();
    for id in 0..shard_count {
        let receipt: CompletionReceipt =
            read_json(&receipts.join(id.to_string()).join("completion.json"))?;
        if receipt.schema_version != 1
            || receipt.plan_id != plan.plan_id
            || receipt.shard_count != shard_count
            || receipt.shard_id != id
            || receipt.survivor_certificate_hash != hash
        {
            return Err(invalid("completion receipt identity mismatch"));
        }
        for r in receipt.matches {
            let evidence = load_evidence(
                bundle,
                plan,
                results,
                &r.evaluation_id,
                &r.system_low,
                &r.system_high,
            )?;
            if evidence.reference != r {
                return Err(invalid("completion reference mismatch"));
            }
            if let Some(previous) = matches.insert(r.match_id.clone(), r.clone()) {
                if previous != r {
                    return Err(invalid("conflicting completion reference"));
                }
            }
        }
    }
    certificate.output_scope = OutputScope::SurvivorSetAndOperationalInputs;
    certificate.matches = matches.into_values().collect();
    let audited = audit_selection(bundle, plan, results, &certificate)?;
    if output.exists() {
        if read_certificate(output)? != certificate {
            return Err(invalid("existing final certificate differs"));
        }
        return Ok(());
    }
    let stage = create_staging_directory(output)?;
    write_json(&stage.join("selection_certificate.json"), &certificate)?;
    write_json(&stage.join("audited_selection.json"), &audited)?;
    publish_staging_directory(&stage, output)
}
