use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::artifact::{read_artifact, result_path, validate_artifact, validate_checksum};
use crate::input::LoadedBundle;
use crate::plan::{TournamentPlan, validate_plan};
use crate::spec::DirectedVerdict;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletenessAudit {
    pub plan_id: String,
    pub planned_matches: usize,
    pub valid_completed_matches: usize,
    pub valid_scientific_unresolved_matches: usize,
    pub missing_matches: usize,
    pub invalid_or_corrupt_artifacts: usize,
    pub duplicated_results: usize,
    pub incompatible_results: usize,
    pub recorded_operational_failures_without_result: usize,
    pub complete: bool,
    pub missing_match_ids: Vec<String>,
    pub invalid_artifacts: Vec<String>,
    pub incompatible_artifacts: Vec<String>,
}

pub fn audit_results(
    bundle: &LoadedBundle,
    plan: &TournamentPlan,
    results_root: &Path,
) -> Result<CompletenessAudit> {
    validate_plan(plan, bundle)?;
    let mut audit = empty_audit(plan);
    let expected_ids: BTreeSet<_> = plan
        .matches
        .iter()
        .map(|item| item.match_id.as_str())
        .collect();
    let matches_root = results_root.join("matches");
    if matches_root.exists() {
        for entry in
            fs::read_dir(&matches_root).map_err(|error| crate::error::io(&matches_root, error))?
        {
            let entry = entry.map_err(|error| crate::error::io(&matches_root, error))?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if !expected_ids.contains(stem) {
                classify_unplanned_result(&path, results_root, &expected_ids, &mut audit);
            }
        }
    }
    for item in &plan.matches {
        let path = result_path(results_root, &item.match_id);
        if !path.exists() {
            audit.missing_matches += 1;
            audit.missing_match_ids.push(item.match_id.clone());
            if results_root
                .join("failures")
                .join(format!("{}.json", item.match_id))
                .exists()
            {
                audit.recorded_operational_failures_without_result += 1;
            }
            continue;
        }
        let validation = (|| {
            let artifact = read_artifact(&path)?;
            validate_artifact(&artifact, bundle, plan, item)?;
            validate_checksum(&path)?;
            Ok::<_, Error>(artifact)
        })();
        match validation {
            Ok(artifact) => {
                audit.valid_completed_matches += 1;
                if artifact.judged.low_over_high.verdict == DirectedVerdict::Unresolved
                    || artifact.judged.high_over_low.verdict == DirectedVerdict::Unresolved
                {
                    audit.valid_scientific_unresolved_matches += 1;
                }
            }
            Err(error) => {
                audit.invalid_or_corrupt_artifacts += 1;
                audit
                    .invalid_artifacts
                    .push(format!("{}: {error}", path.display()));
            }
        }
    }
    audit.complete = audit.valid_completed_matches == audit.planned_matches
        && audit.missing_matches == 0
        && audit.invalid_or_corrupt_artifacts == 0
        && audit.duplicated_results == 0
        && audit.incompatible_results == 0;
    Ok(audit)
}

pub fn status_results(plan: &TournamentPlan, results_root: &Path) -> Result<CompletenessAudit> {
    let mut audit = empty_audit(plan);
    let expected_ids: BTreeSet<_> = plan
        .matches
        .iter()
        .map(|item| item.match_id.as_str())
        .collect();
    let matches_root = results_root.join("matches");
    if matches_root.exists() {
        for entry in
            fs::read_dir(&matches_root).map_err(|error| crate::error::io(&matches_root, error))?
        {
            let entry = entry.map_err(|error| crate::error::io(&matches_root, error))?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if !expected_ids.contains(stem) {
                classify_unplanned_result(&path, results_root, &expected_ids, &mut audit);
            }
        }
    }
    for item in &plan.matches {
        let path = result_path(results_root, &item.match_id);
        if !path.exists() {
            audit.missing_matches += 1;
            audit.missing_match_ids.push(item.match_id.clone());
            if results_root
                .join("failures")
                .join(format!("{}.json", item.match_id))
                .exists()
            {
                audit.recorded_operational_failures_without_result += 1;
            }
            continue;
        }
        match read_artifact(&path) {
            Ok(artifact)
                if artifact.complete
                    && artifact.match_id == item.match_id
                    && artifact.plan_id == plan.plan_id =>
            {
                audit.valid_completed_matches += 1;
                if artifact.judged.low_over_high.verdict == DirectedVerdict::Unresolved
                    || artifact.judged.high_over_low.verdict == DirectedVerdict::Unresolved
                {
                    audit.valid_scientific_unresolved_matches += 1;
                }
            }
            Ok(_) => {
                audit.invalid_or_corrupt_artifacts += 1;
                audit
                    .invalid_artifacts
                    .push(format!("{}: identity mismatch", path.display()));
            }
            Err(error) => {
                audit.invalid_or_corrupt_artifacts += 1;
                audit
                    .invalid_artifacts
                    .push(format!("{}: {error}", path.display()));
            }
        }
    }
    audit.complete = audit.valid_completed_matches == audit.planned_matches
        && audit.invalid_or_corrupt_artifacts == 0
        && audit.incompatible_results == 0;
    Ok(audit)
}

fn empty_audit(plan: &TournamentPlan) -> CompletenessAudit {
    CompletenessAudit {
        plan_id: plan.plan_id.clone(),
        planned_matches: plan.expected_match_count,
        valid_completed_matches: 0,
        valid_scientific_unresolved_matches: 0,
        missing_matches: 0,
        invalid_or_corrupt_artifacts: 0,
        duplicated_results: 0,
        incompatible_results: 0,
        recorded_operational_failures_without_result: 0,
        complete: false,
        missing_match_ids: Vec::new(),
        invalid_artifacts: Vec::new(),
        incompatible_artifacts: Vec::new(),
    }
}

fn classify_unplanned_result(
    path: &Path,
    results_root: &Path,
    expected_ids: &BTreeSet<&str>,
    audit: &mut CompletenessAudit,
) {
    let duplicate = read_artifact(path).ok().is_some_and(|artifact| {
        expected_ids.contains(artifact.match_id.as_str())
            && result_path(results_root, &artifact.match_id).exists()
    });
    if duplicate {
        audit.duplicated_results += 1;
    } else {
        audit.incompatible_results += 1;
        audit
            .incompatible_artifacts
            .push(path.display().to_string());
    }
}
