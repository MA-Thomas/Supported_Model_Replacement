use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use directed_round_robin_organizer::artifact::{
    read_artifact_with_checksum, result_path, validate_artifact,
};
use directed_round_robin_organizer::judge::JudgeReport;
use directed_round_robin_organizer::reduce::SelectionOutcome;
use directed_round_robin_organizer::{MetricKind, audit_results, load_bundle, load_reduction_for};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use supported_ap::{
    PairedEvaluation, SearchOptions, StagedDirectionalProjectedAp, TargetPrevalences,
};

use crate::config::PipelineConfig;
use crate::io::{collect_file_hashes, read_json, sha256_file, stage_path, write_json_new};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActivationRow {
    pub evaluation_id: String,
    pub winner: String,
    pub loser: String,
    pub observed_supported_magnitude: f64,
    pub full_supported_magnitude: Option<f64>,
    pub observed_survival_fraction: f64,
    pub full_survival_fraction: Option<f64>,
    pub magnitude_requirements_pass: bool,
    pub activation_strength: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IncomingCoordinate {
    pub activation_strength: f64,
    pub challenger: String,
    pub evaluation_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IncomingProfile {
    pub candidate: String,
    pub descending_vector: Vec<f64>,
    pub binding_incoming: Option<IncomingCoordinate>,
    pub coordinates: Vec<IncomingCoordinate>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RangeRegretRow {
    pub candidate: String,
    pub evaluation_id: String,
    pub regret: f64,
    pub comparator: String,
    pub prevalence: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RangeRegretProfile {
    pub candidate: String,
    pub descending_vector: Vec<f64>,
    pub binding_range_regret: Option<RangeRegretRow>,
    pub rows: Vec<RangeRegretRow>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version2Selection {
    pub schema_version: u32,
    pub tournament_id: String,
    pub metric: MetricKind,
    pub s0: Vec<String>,
    pub incoming_profiles: Vec<IncomingProfile>,
    pub t: Vec<String>,
    pub tertiary_rule_invoked: bool,
    pub operational_prevalences: TargetPrevalences,
    pub operational_search: SearchOptions,
    pub operational_scope_statement: String,
    pub range_regret_profiles: Vec<RangeRegretProfile>,
    pub s_op: Vec<String>,
    pub identical_complete_vectors_remain_tied: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version2Summary {
    pub output: PathBuf,
    pub s0: Vec<String>,
    pub t: Vec<String>,
    pub s_op: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SourceFile {
    path: PathBuf,
    sha256: String,
}

fn source_file(path: PathBuf) -> Result<SourceFile> {
    let path = path.canonicalize()?;
    Ok(SourceFile {
        sha256: sha256_file(&path)?,
        path,
    })
}

fn lex_cmp(left: &[f64], right: &[f64]) -> Ordering {
    left.iter()
        .zip(right)
        .find_map(|(left, right)| {
            let order = left.total_cmp(right);
            (order != Ordering::Equal).then_some(order)
        })
        .unwrap_or_else(|| left.len().cmp(&right.len()))
}

fn lex_minimizers<'a, F>(candidates: &'a [String], vector: F) -> Vec<String>
where
    F: Fn(&str) -> &'a [f64],
{
    let Some(best) = candidates
        .iter()
        .min_by(|left, right| lex_cmp(vector(left), vector(right)))
    else {
        return Vec::new();
    };
    candidates
        .iter()
        .filter(|candidate| lex_cmp(vector(candidate), vector(best)) == Ordering::Equal)
        .cloned()
        .collect()
}

fn pr_activation(
    evaluation_id: &str,
    winner: &str,
    loser: &str,
    direction: &StagedDirectionalProjectedAp,
    magnitude_threshold: f64,
) -> Result<ActivationRow> {
    let observed = &direction.observed_gate;
    let full = direction.full_assessment.as_ref();
    let observed_magnitude = observed.supported_magnitude;
    let full_magnitude = full.map(|value| value.supported_magnitude);
    let observed_survival = observed.literal_survival.subset_fraction;
    let full_survival = full.map(|value| value.literal_survival.subset_fraction);
    let passes = observed_magnitude > magnitude_threshold
        && full_magnitude.is_some_and(|value| value > magnitude_threshold);
    Ok(ActivationRow {
        evaluation_id: evaluation_id.to_owned(),
        winner: winner.to_owned(),
        loser: loser.to_owned(),
        observed_supported_magnitude: observed_magnitude,
        full_supported_magnitude: full_magnitude,
        observed_survival_fraction: observed_survival,
        full_survival_fraction: full_survival,
        magnitude_requirements_pass: passes,
        activation_strength: if passes {
            observed_survival.min(full_survival.expect("passing full assessment exists"))
        } else {
            0.0
        },
    })
}

fn auroc_activation(
    evaluation_id: &str,
    winner: &str,
    loser: &str,
    result: &supported_ap::AuRocBreakdownEstimate,
    magnitude_threshold: f64,
) -> ActivationRow {
    // A direction can activate only on certified evidence. The optimizer's
    // lower bounds are therefore the relevant AUROC magnitudes and survival
    // fractions; midpoint estimates cannot create an operational edge.
    let passes = result.observed_gate.supported_magnitude_lower > magnitude_threshold
        && result.baseline.supported_magnitude_lower > magnitude_threshold;
    ActivationRow {
        evaluation_id: evaluation_id.to_owned(),
        winner: winner.to_owned(),
        loser: loser.to_owned(),
        observed_supported_magnitude: result.observed_gate.supported_magnitude_lower,
        full_supported_magnitude: Some(result.baseline.supported_magnitude_lower),
        observed_survival_fraction: result.observed_gate.literal_survival_lower,
        full_survival_fraction: Some(result.baseline.literal_survival_lower),
        magnitude_requirements_pass: passes,
        activation_strength: if passes {
            result
                .observed_gate
                .literal_survival_lower
                .min(result.baseline.literal_survival_lower)
        } else {
            0.0
        },
    }
}

fn source_set(outcome: &SelectionOutcome) -> Vec<String> {
    match outcome {
        SelectionOutcome::UniqueConjecturedSystem { system_id } => vec![system_id.clone()],
        SelectionOutcome::UnresolvedCandidateSet { system_ids } => system_ids.clone(),
        SelectionOutcome::NoSurvivingCandidate => Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn finalize_version2(
    config_path: &Path,
    bundle_root: &Path,
    plan_root: &Path,
    results_root: &Path,
    reduction_root: &Path,
    output: &Path,
) -> Result<Version2Summary> {
    let config = PipelineConfig::load(config_path)?;
    let bundle = load_bundle(bundle_root).map_err(anyhow::Error::msg)?;
    let plan =
        directed_round_robin_organizer::plan::read_plan(plan_root).map_err(anyhow::Error::msg)?;
    let audit = audit_results(&bundle, &plan, results_root).map_err(anyhow::Error::msg)?;
    if !audit.complete {
        bail!("cannot apply Version 2 to incomplete match results")
    }
    let reduction =
        load_reduction_for(reduction_root, &bundle, &plan).map_err(anyhow::Error::msg)?;
    let mut s0 = source_set(&reduction.selection.outcome);
    s0.sort();
    let admissible: BTreeSet<_> = s0.iter().map(String::as_str).collect();
    let magnitude_threshold = bundle.spec.evidence_policy.magnitude_threshold;

    let match_rows = plan
        .matches
        .par_iter()
        .map(|item| {
            let path = result_path(results_root, &item.match_id);
            let artifact = read_artifact_with_checksum(&path).map_err(anyhow::Error::msg)?;
            validate_artifact(&artifact, &bundle, &plan, item).map_err(anyhow::Error::msg)?;
            let rows = match &artifact.judged.report {
                JudgeReport::PrCnap { result, .. } => vec![
                    pr_activation(
                        &artifact.evaluation_id,
                        &artifact.system_low,
                        &artifact.system_high,
                        &result.forward,
                        magnitude_threshold,
                    )?,
                    pr_activation(
                        &artifact.evaluation_id,
                        &artifact.system_high,
                        &artifact.system_low,
                        &result.reverse,
                        magnitude_threshold,
                    )?,
                ],
                JudgeReport::Auroc {
                    low_over_high_result,
                    high_over_low_result,
                    ..
                } => vec![
                    auroc_activation(
                        &artifact.evaluation_id,
                        &artifact.system_low,
                        &artifact.system_high,
                        low_over_high_result,
                        magnitude_threshold,
                    ),
                    auroc_activation(
                        &artifact.evaluation_id,
                        &artifact.system_high,
                        &artifact.system_low,
                        high_over_low_result,
                        magnitude_threshold,
                    ),
                ],
            };
            Ok::<_, anyhow::Error>(rows)
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    let mut incoming_profiles = s0
        .iter()
        .map(|candidate| {
            let mut coordinates = match_rows
                .iter()
                .filter(|row| {
                    row.loser == *candidate
                        && admissible.contains(row.winner.as_str())
                        && row.winner != *candidate
                })
                .map(|row| IncomingCoordinate {
                    activation_strength: row.activation_strength,
                    challenger: row.winner.clone(),
                    evaluation_id: row.evaluation_id.clone(),
                })
                .collect::<Vec<_>>();
            coordinates.sort_by(|left, right| {
                right
                    .activation_strength
                    .total_cmp(&left.activation_strength)
                    .then_with(|| left.challenger.cmp(&right.challenger))
                    .then_with(|| left.evaluation_id.cmp(&right.evaluation_id))
            });
            IncomingProfile {
                candidate: candidate.clone(),
                descending_vector: coordinates
                    .iter()
                    .map(|row| row.activation_strength)
                    .collect(),
                binding_incoming: coordinates.first().cloned(),
                coordinates,
            }
        })
        .collect::<Vec<_>>();
    incoming_profiles.sort_by(|left, right| left.candidate.cmp(&right.candidate));
    let expected_incoming = s0.len().saturating_sub(1) * bundle.evaluations.len();
    if let Some(profile) = incoming_profiles
        .iter()
        .find(|profile| profile.coordinates.len() != expected_incoming)
    {
        bail!(
            "incomplete incoming profile for {}: expected {expected_incoming}, got {}",
            profile.candidate,
            profile.coordinates.len()
        )
    }
    let incoming_by_candidate: BTreeMap<_, _> = incoming_profiles
        .iter()
        .map(|profile| {
            (
                profile.candidate.as_str(),
                profile.descending_vector.as_slice(),
            )
        })
        .collect();
    let t = lex_minimizers(&s0, |candidate| incoming_by_candidate[candidate]);

    let mut range_regret_profiles = Vec::new();
    if t.len() > 1 {
        let work = t
            .iter()
            .flat_map(|candidate| {
                bundle
                    .evaluations
                    .keys()
                    .map(move |evaluation| (candidate.clone(), evaluation.clone()))
            })
            .collect::<Vec<_>>();
        let rows = work
            .par_iter()
            .map(|(candidate, evaluation_id)| {
                let evaluation = &bundle.evaluations[evaluation_id];
                let candidate_scores = &evaluation.scores[candidate];
                let multiplicities = vec![1_usize; evaluation.labels.len()];
                let mut alternatives = s0
                    .iter()
                    .map(|comparator| {
                        let paired = PairedEvaluation::new(
                            candidate_scores,
                            &evaluation.scores[comparator],
                            &evaluation.labels,
                        )?;
                        let (candidate_minus_comparator, _) = paired
                            .retained_effects_for_multiplicities(
                                &multiplicities,
                                &config.tournament.operational_prevalences,
                                config.tournament.operational_search,
                            )?;
                        Ok::<_, anyhow::Error>(RangeRegretRow {
                            candidate: candidate.clone(),
                            evaluation_id: evaluation_id.clone(),
                            regret: (-candidate_minus_comparator.value).max(0.0),
                            comparator: comparator.clone(),
                            prevalence: candidate_minus_comparator.limiting_prevalence.get(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                alternatives.sort_by(|left, right| {
                    right
                        .regret
                        .total_cmp(&left.regret)
                        .then_with(|| left.comparator.cmp(&right.comparator))
                        .then_with(|| left.prevalence.total_cmp(&right.prevalence))
                });
                alternatives
                    .into_iter()
                    .next()
                    .context("S0 unexpectedly empty")
            })
            .collect::<Result<Vec<_>>>()?;
        for candidate in &t {
            let mut candidate_rows = rows
                .iter()
                .filter(|row| row.candidate == *candidate)
                .cloned()
                .collect::<Vec<_>>();
            candidate_rows.sort_by(|left, right| {
                right
                    .regret
                    .total_cmp(&left.regret)
                    .then_with(|| left.evaluation_id.cmp(&right.evaluation_id))
            });
            range_regret_profiles.push(RangeRegretProfile {
                candidate: candidate.clone(),
                descending_vector: candidate_rows.iter().map(|row| row.regret).collect(),
                binding_range_regret: candidate_rows.first().cloned(),
                rows: candidate_rows,
            });
        }
    }
    let s_op = if t.len() <= 1 {
        t.clone()
    } else {
        let regret_by_candidate: BTreeMap<_, _> = range_regret_profiles
            .iter()
            .map(|profile| {
                (
                    profile.candidate.as_str(),
                    profile.descending_vector.as_slice(),
                )
            })
            .collect();
        lex_minimizers(&t, |candidate| regret_by_candidate[candidate])
    };
    let selection = Version2Selection {
        schema_version: 1,
        tournament_id: plan.tournament_id.clone(),
        metric: plan.metric,
        s0: s0.clone(),
        incoming_profiles,
        t: t.clone(),
        tertiary_rule_invoked: t.len() > 1,
        operational_prevalences: config.tournament.operational_prevalences.clone(),
        operational_search: config.tournament.operational_search,
        operational_scope_statement: config.tournament.operational_scope_statement.clone(),
        range_regret_profiles,
        s_op: s_op.clone(),
        identical_complete_vectors_remain_tied: true,
    };

    let stage = stage_path(output)?;
    let result: Result<()> = (|| {
        write_json_new(&stage.join("selection.json"), &selection)?;
        let mut source_files = vec![
            source_file(config_path.to_owned())?,
            source_file(bundle_root.join("bundle_manifest.json"))?,
            source_file(plan_root.join("plan.json"))?,
            source_file(reduction_root.join("reduction_manifest.json"))?,
        ];
        source_files.extend(
            plan.matches
                .par_iter()
                .map(|item| source_file(result_path(results_root, &item.match_id)))
                .collect::<Result<Vec<_>>>()?,
        );
        source_files.sort_by(|left, right| left.path.cmp(&right.path));
        let manifest = serde_json::json!({
            "schema_version": 1,
            "tournament_id": plan.tournament_id,
            "bundle_content_hash": plan.bundle_content_hash,
            "metric": plan.metric,
            "source_files": source_files,
            "files": collect_file_hashes(&stage)?,
        });
        write_json_new(&stage.join("manifest.json"), &manifest)?;
        fs::rename(&stage, output)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&stage);
    }
    result?;
    Ok(Version2Summary {
        output: output.to_owned(),
        s0,
        t,
        s_op,
    })
}

pub fn audit_version2(root: &Path) -> Result<()> {
    let manifest: serde_json::Value = read_json(&root.join("manifest.json"))?;
    let expected = manifest["files"].as_object().context("Version 2 files")?;
    let actual = collect_file_hashes(root)?;
    if expected.len() != actual.len() {
        bail!("Version 2 output file count differs from manifest")
    }
    for (path, hash) in expected {
        if actual.get(path).map(String::as_str) != hash.as_str() {
            bail!("Version 2 output hash mismatch: {path}")
        }
    }
    let sources: Vec<SourceFile> = serde_json::from_value(manifest["source_files"].clone())
        .context("Version 2 source_files")?;
    for source in sources {
        if sha256_file(&source.path)? != source.sha256 {
            bail!("Version 2 source hash mismatch: {}", source.path.display())
        }
    }
    let selection: Version2Selection = read_json(&root.join("selection.json"))?;
    if !selection.identical_complete_vectors_remain_tied {
        bail!("Version 2 output does not preserve identical-vector ties")
    }
    if !selection.s_op.iter().all(|item| selection.t.contains(item))
        || !selection.t.iter().all(|item| selection.s0.contains(item))
    {
        bail!("Version 2 selection sets are not nested")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lex_minimizers_preserve_identical_vectors() {
        let candidates = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
        let vectors = BTreeMap::from([
            ("a", vec![0.4, 0.2]),
            ("b", vec![0.3, 0.2]),
            ("c", vec![0.3, 0.2]),
        ]);
        assert_eq!(
            lex_minimizers(&candidates, |candidate| &vectors[candidate]),
            vec!["b", "c"]
        );
    }
}
