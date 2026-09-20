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
    observed_auroc_order_key,
};

use crate::config::PipelineConfig;
use crate::io::{collect_file_hashes, read_json, sha256_file, stage_path, write_json_new};

const REFINEMENT_MULTIPLIER: usize = 8;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct NumericalBounds {
    pub lower: f64,
    pub upper: f64,
}
impl NumericalBounds {
    fn point(value: f64) -> Self {
        Self {
            lower: value,
            upper: value,
        }
    }
}

// Sorted order statistics are monotone in each coordinate. Sorting endpoints
// separately therefore encloses the entire descending vector.
fn sorted_bounds(values: impl Iterator<Item = NumericalBounds>) -> (Vec<f64>, Vec<f64>) {
    let (mut lower, mut upper): (Vec<_>, Vec<_>) = values.map(|b| (b.lower, b.upper)).unzip();
    lower.sort_by(|a, b| b.total_cmp(a));
    upper.sort_by(|a, b| b.total_cmp(a));
    (lower, upper)
}

fn definitely_less(left: &(Vec<f64>, Vec<f64>), right: &(Vec<f64>, Vec<f64>)) -> bool {
    for i in 0..left.0.len() {
        if left.1[i] < right.0[i] {
            return true;
        }
        if left.0[i] == left.1[i] && right.0[i] == right.1[i] && left.0[i] == right.0[i] {
            continue;
        }
        return false;
    }
    false
}

fn interval_minimizers(
    candidates: &[String],
    vectors: &BTreeMap<String, (Vec<f64>, Vec<f64>)>,
) -> (Vec<String>, bool) {
    let possible: Vec<_> = candidates
        .iter()
        .filter(|candidate| {
            !candidates.iter().any(|other| {
                other != *candidate && definitely_less(&vectors[other], &vectors[*candidate])
            })
        })
        .cloned()
        .collect();
    let unresolved = possible.len() > 1 && possible.iter().any(|c| vectors[c].0 != vectors[c].1);
    (possible, unresolved)
}

fn refine_search(search: &mut SearchOptions, limit: usize) {
    search.max_iterations = search.max_iterations.saturating_mul(2).min(limit);
    search.tolerance = (search.tolerance * 0.1).max(f64::from_bits(1));
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActivationRow {
    #[serde(default)]
    pub activation_bounds: Option<NumericalBounds>,
    #[serde(default)]
    pub refinement_assessment: Option<supported_ap::ProjectedApAssessment>,
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
    #[serde(default)]
    pub activation_bounds: Option<NumericalBounds>,
    pub activation_strength: f64,
    pub challenger: String,
    pub evaluation_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IncomingProfile {
    #[serde(default)]
    pub descending_upper_vector: Vec<f64>,
    pub candidate: String,
    pub descending_vector: Vec<f64>,
    pub binding_incoming: Option<IncomingCoordinate>,
    pub coordinates: Vec<IncomingCoordinate>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RangeRegretRow {
    #[serde(default)]
    pub regret_bounds: Option<NumericalBounds>,
    #[serde(default)]
    pub search: Option<supported_ap::PrevalenceSearchCertificate>,
    pub candidate: String,
    pub evaluation_id: String,
    pub regret: f64,
    pub comparator: String,
    pub prevalence: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RangeRegretProfile {
    #[serde(default)]
    pub descending_lower_vector: Vec<f64>,
    pub candidate: String,
    pub descending_vector: Vec<f64>,
    pub binding_range_regret: Option<RangeRegretRow>,
    pub rows: Vec<RangeRegretRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TertiaryRule {
    WorstTargetCnapRegret,
    EmpiricalAurocRegret,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AurocRegretRow {
    pub candidate: String,
    pub evaluation_id: String,
    pub regret: f64,
    pub comparator: String,
    pub candidate_auroc: f64,
    pub comparator_auroc: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AurocRegretProfile {
    pub candidate: String,
    pub descending_vector: Vec<f64>,
    pub binding_auroc_regret: Option<AurocRegretRow>,
    pub rows: Vec<AurocRegretRow>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version2Selection {
    #[serde(default)]
    pub numerical_status: Option<String>,
    #[serde(default)]
    pub refinement_multiplier: Option<usize>,
    pub schema_version: u32,
    pub tournament_id: String,
    pub metric: MetricKind,
    pub s0: Vec<String>,
    pub incoming_profiles: Vec<IncomingProfile>,
    pub t: Vec<String>,
    pub tertiary_rule_invoked: bool,
    // Schema 1 always used CNAP, including for an AUROC primary tournament.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tertiary_rule: Option<TertiaryRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operational_prevalences: Option<TargetPrevalences>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operational_search: Option<SearchOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operational_auroc_gamma: Option<f64>,
    pub operational_scope_statement: String,
    pub range_regret_profiles: Vec<RangeRegretProfile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auroc_regret_profiles: Vec<AurocRegretProfile>,
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
    let observed_upper = observed
        .prevalence_search
        .map_or(observed_magnitude, |b| b.supported_magnitude_upper);
    let full_upper = full.map(|e| {
        e.prevalence_search
            .map_or(e.supported_magnitude, |b| b.supported_magnitude_upper)
    });
    let observed_survival_upper = observed
        .prevalence_search
        .map_or(observed_survival, |b| b.literal_survival_upper);
    let full_survival_upper = full.map(|e| {
        e.prevalence_search
            .map_or(e.literal_survival.subset_fraction, |b| {
                b.literal_survival_upper
            })
    });
    let lower = if passes {
        observed_survival.min(full_survival.unwrap())
    } else {
        0.0
    };
    let upper = if observed_upper <= magnitude_threshold
        || full_upper.is_some_and(|m| m <= magnitude_threshold)
    {
        0.0
    } else if full.is_some() {
        observed_survival_upper.min(full_survival_upper.unwrap())
    } else if observed
        .prevalence_search
        .is_some_and(|b| b.verdict == supported_ap::PolicyVerdict::Unresolved)
    {
        observed_survival_upper
    } else {
        0.0
    };
    Ok(ActivationRow {
        activation_bounds: Some(NumericalBounds { lower, upper }),
        refinement_assessment: None,
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
        activation_bounds: None,
        refinement_assessment: None,
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

fn activation_rows(
    artifact: &directed_round_robin_organizer::MatchArtifact,
    magnitude_threshold: f64,
) -> Result<Vec<ActivationRow>> {
    Ok(match &artifact.judged.report {
        JudgeReport::PrCnap { result, assessment } => {
            let mut rows = vec![
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
            ];
            for row in &mut rows {
                row.refinement_assessment = Some(assessment.clone());
            }
            rows
        }
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
    })
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
    if let Some(expected) = bundle
        .spec
        .annotations
        .get("config_sha256")
        .and_then(|v| v.as_str())
    {
        anyhow::ensure!(
            sha256_file(config_path)? == expected,
            "finalizer configuration differs from bundle"
        );
    }
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
    let magnitude_threshold = bundle.spec.evidence_policy.magnitude_threshold;

    let match_rows = plan
        .matches
        .par_iter()
        .map(|item| {
            let path = result_path(results_root, &item.match_id);
            let artifact = read_artifact_with_checksum(&path).map_err(anyhow::Error::msg)?;
            validate_artifact(&artifact, &bundle, &plan, item).map_err(anyhow::Error::msg)?;
            let rows = activation_rows(&artifact, magnitude_threshold)?;
            Ok::<_, anyhow::Error>(rows)
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

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
    finalize_from_rows(
        &config,
        &bundle,
        &plan.tournament_id,
        s0,
        match_rows,
        source_files,
        output,
    )
}

/// Version 2 over independently audited accelerated evidence. No incomplete
/// Reduction is fabricated, and the challenger/comparator cohort remains S0.
pub fn finalize_version2_accelerated(
    config_path: &Path,
    bundle_root: &Path,
    plan_root: &Path,
    results_root: &Path,
    selection_root: &Path,
    output: &Path,
) -> Result<Version2Summary> {
    use directed_round_robin_organizer::accelerated::{
        audit_selection, read_accelerated_plan, read_certificate,
    };
    let config = PipelineConfig::load(config_path)?;
    let bundle = load_bundle(bundle_root).map_err(anyhow::Error::msg)?;
    if let Some(expected) = bundle
        .spec
        .annotations
        .get("config_sha256")
        .and_then(|v| v.as_str())
    {
        anyhow::ensure!(
            sha256_file(config_path)? == expected,
            "finalizer configuration differs from bundle"
        );
    }
    let plan = read_accelerated_plan(plan_root).map_err(anyhow::Error::msg)?;
    let certificate = read_certificate(selection_root).map_err(anyhow::Error::msg)?;
    let audited =
        audit_selection(&bundle, &plan, results_root, &certificate).map_err(anyhow::Error::msg)?;
    let references = audited
        .operational_match_references()
        .map_err(anyhow::Error::msg)?;
    let match_rows = references
        .par_iter()
        .map(|reference| {
            let artifact = audited
                .read_match(&bundle, results_root, reference)
                .map_err(anyhow::Error::msg)?;
            activation_rows(&artifact, bundle.spec.evidence_policy.magnitude_threshold)
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    let mut sources = vec![
        source_file(config_path.to_owned())?,
        source_file(bundle_root.join("bundle_manifest.json"))?,
        source_file(plan_root.join("accelerated_plan.json"))?,
        source_file(selection_root.join("selection_certificate.json"))?,
    ];
    // Bind every exclusion/survival witness as well as the operational cohort.
    sources.extend(
        audited
            .match_references()
            .map(|reference| source_file(result_path(results_root, &reference.match_id)))
            .collect::<Result<Vec<_>>>()?,
    );
    finalize_from_rows(
        &config,
        &bundle,
        &plan.tournament_id,
        audited.survivors().to_vec(),
        match_rows,
        sources,
        output,
    )
}

/// At Gamma = 1 every candidate faces the same uniform class weights. Compute
/// each S0 score once; maximizing the comparator does not depend on the target.
fn empirical_auroc_regrets(
    evaluation_id: &str,
    labels: &[bool],
    scores: &BTreeMap<String, Vec<f64>>,
    s0: &[String],
    t: &[String],
) -> Result<Vec<AurocRegretRow>> {
    let credits = s0
        .iter()
        .map(|id| {
            let values = scores
                .get(id)
                .with_context(|| format!("missing scores for {id}"))?;
            Ok((id.as_str(), observed_auroc_order_key(values, labels)?))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    // Resolve only the displayed binding comparator deterministically. Equal
    // candidate regrets remain tied in the operational selection.
    let (&comparator, &best_credit) = credits
        .iter()
        .min_by(|(left_id, left), (right_id, right)| {
            right.cmp(left).then_with(|| left_id.cmp(right_id))
        })
        .context("S0 unexpectedly empty")?;
    let positive = labels.iter().filter(|&&value| value).count() as u64;
    let negative = labels.len() as u64 - positive;
    let denominator = positive
        .checked_mul(negative)
        .and_then(|n| n.checked_mul(2))
        .filter(|&n| n > 0)
        .context("invalid AUROC class counts")? as f64;
    t.iter()
        .map(|candidate| {
            let &credit = credits
                .get(candidate.as_str())
                .context("T is not a subset of S0")?;
            Ok(AurocRegretRow {
                candidate: candidate.clone(),
                evaluation_id: evaluation_id.into(),
                // Subtract exact doubled Mann-Whitney credits before dividing to
                // avoid cancellation and preserve equal empirical regrets.
                regret: (best_credit - credit) as f64 / denominator,
                comparator: comparator.into(),
                candidate_auroc: credit as f64 / denominator,
                comparator_auroc: best_credit as f64 / denominator,
            })
        })
        .collect()
}

fn incoming_profiles_for(
    s0: &[String],
    match_rows: &[ActivationRow],
    evaluation_count: usize,
) -> Result<Vec<IncomingProfile>> {
    let admissible: BTreeSet<_> = s0.iter().map(String::as_str).collect();
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
                    activation_bounds: row.activation_bounds,
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
            let (_, upper) = sorted_bounds(coordinates.iter().map(|r| {
                r.activation_bounds
                    .unwrap_or(NumericalBounds::point(r.activation_strength))
            }));
            IncomingProfile {
                descending_upper_vector: upper,
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
    let expected_incoming = s0.len().saturating_sub(1) * evaluation_count;
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
    Ok(incoming_profiles)
}

fn incoming_minimizers(s0: &[String], profiles: &[IncomingProfile]) -> (Vec<String>, bool) {
    let vectors = profiles
        .iter()
        .map(|p| {
            (
                p.candidate.clone(),
                (
                    p.descending_vector.clone(),
                    p.descending_upper_vector.clone(),
                ),
            )
        })
        .collect();
    interval_minimizers(s0, &vectors)
}

fn regret_profiles_for(
    bundle: &directed_round_robin_organizer::LoadedBundle,
    t: &[String],
    s0: &[String],
    target: &TargetPrevalences,
    search: SearchOptions,
) -> Result<Vec<RangeRegretProfile>> {
    let mut range_regret_profiles = Vec::new();
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
                        .retained_effects_for_multiplicities(&multiplicities, target, search)?;
                    Ok::<_, anyhow::Error>(RangeRegretRow {
                        candidate: candidate.clone(),
                        evaluation_id: evaluation_id.clone(),
                        regret: (-candidate_minus_comparator.value).max(0.0),
                        regret_bounds: Some(NumericalBounds {
                            lower: (-candidate_minus_comparator
                                .search
                                .map_or(candidate_minus_comparator.value, |b| b.upper_bound))
                            .max(0.0),
                            upper: (-candidate_minus_comparator.value).max(0.0),
                        }),
                        search: candidate_minus_comparator.search,
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
            let lower = alternatives
                .iter()
                .map(|r| r.regret_bounds.unwrap().lower)
                .fold(0.0, f64::max);
            let mut worst = alternatives
                .into_iter()
                .next()
                .context("S0 unexpectedly empty")?;
            worst.regret_bounds.as_mut().unwrap().lower = lower;
            Ok(worst)
        })
        .collect::<Result<Vec<_>>>()?;
    for candidate in t {
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
        let (lower, upper) = sorted_bounds(candidate_rows.iter().map(|r| r.regret_bounds.unwrap()));
        range_regret_profiles.push(RangeRegretProfile {
            descending_lower_vector: lower,
            candidate: candidate.clone(),
            descending_vector: upper,
            binding_range_regret: candidate_rows.first().cloned(),
            rows: candidate_rows,
        });
    }
    Ok(range_regret_profiles)
}

#[allow(clippy::too_many_arguments)]
fn finalize_from_rows(
    config: &PipelineConfig,
    bundle: &directed_round_robin_organizer::LoadedBundle,
    tournament_id: &str,
    s0: Vec<String>,
    match_rows: Vec<ActivationRow>,
    mut source_files: Vec<SourceFile>,
    output: &Path,
) -> Result<Version2Summary> {
    let mut match_rows = match_rows;
    let limits: Vec<_> = match_rows
        .iter()
        .map(|r| {
            r.refinement_assessment.as_ref().map_or(0, |a| {
                a.search
                    .max_iterations
                    .saturating_mul(REFINEMENT_MULTIPLIER)
            })
        })
        .collect();
    let (incoming_profiles, t, incoming_unresolved) = loop {
        let profiles = incoming_profiles_for(&s0, &match_rows, bundle.evaluations.len())?;
        let (possible, unresolved) = incoming_minimizers(&s0, &profiles);
        if !unresolved {
            break (profiles, possible, false);
        }
        let mut refined = false;
        for (row, limit) in match_rows.iter_mut().zip(&limits) {
            if !possible.contains(&row.loser)
                || !row.activation_bounds.is_some_and(|b| b.lower < b.upper)
            {
                continue;
            }
            let Some(mut assessment) = row.refinement_assessment.clone() else {
                continue;
            };
            if assessment.search.max_iterations >= *limit {
                continue;
            }
            refine_search(&mut assessment.search, *limit);
            let evaluation = &bundle.evaluations[&row.evaluation_id];
            let paired = PairedEvaluation::new(
                &evaluation.scores[&row.winner],
                &evaluation.scores[&row.loser],
                &evaluation.labels,
            )?;
            let p = &bundle.spec.evidence_policy;
            let policy = supported_ap::ReplacementPolicy::new(
                supported_ap::MagnitudeThreshold::new(p.magnitude_threshold)?,
                supported_ap::SurvivalFloor::new(p.survival_floor)?,
                supported_ap::SurvivalRequirement::new(p.survival_requirement)?,
            );
            let assessed =
                supported_ap::assess_staged_projected_ap(&paired, &assessment, policy, false)?;
            let mut next = pr_activation(
                &row.evaluation_id,
                &row.winner,
                &row.loser,
                &assessed.forward,
                p.magnitude_threshold,
            )?;
            next.refinement_assessment = Some(assessment);
            *row = next;
            refined = true;
        }
        if !refined {
            break (profiles, possible, true);
        }
    };

    let mut range_regret_profiles = Vec::new();
    let mut regret_possible = t.clone();
    let mut regret_unresolved = false;
    let mut operational_search = config.tournament.operational_search;
    if !incoming_unresolved && t.len() > 1 && bundle.spec.metric == MetricKind::PrCnap {
        let limit = operational_search
            .max_iterations
            .saturating_mul(REFINEMENT_MULTIPLIER);
        loop {
            range_regret_profiles = regret_profiles_for(
                bundle,
                &t,
                &s0,
                &config.tournament.operational_prevalences,
                operational_search,
            )?;
            let vectors = range_regret_profiles
                .iter()
                .map(|p| {
                    (
                        p.candidate.clone(),
                        (
                            p.descending_lower_vector.clone(),
                            p.descending_vector.clone(),
                        ),
                    )
                })
                .collect();
            (regret_possible, regret_unresolved) = interval_minimizers(&t, &vectors);
            if !regret_unresolved || operational_search.max_iterations >= limit {
                break;
            }
            refine_search(&mut operational_search, limit);
        }
    }
    let mut auroc_regret_profiles = Vec::new();
    if t.len() > 1 && bundle.spec.metric == MetricKind::Auroc {
        let rows = bundle
            .evaluations
            .par_iter()
            .map(|(id, evaluation)| {
                empirical_auroc_regrets(id, &evaluation.labels, &evaluation.scores, &s0, &t)
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
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
            auroc_regret_profiles.push(AurocRegretProfile {
                candidate: candidate.clone(),
                descending_vector: candidate_rows.iter().map(|row| row.regret).collect(),
                binding_auroc_regret: candidate_rows.first().cloned(),
                rows: candidate_rows,
            });
        }
    }
    let s_op = if incoming_unresolved {
        t.clone()
    } else if bundle.spec.metric == MetricKind::PrCnap {
        regret_possible
    } else if t.len() <= 1 {
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
            .chain(auroc_regret_profiles.iter().map(|profile| {
                (
                    profile.candidate.as_str(),
                    profile.descending_vector.as_slice(),
                )
            }))
            .collect();
        lex_minimizers(&t, |candidate| regret_by_candidate[candidate])
    };
    let selection = Version2Selection {
        numerical_status: Some(if incoming_unresolved { "unresolved_incoming" } else if regret_unresolved { "unresolved_regret" } else { "resolved" }.into()),
        refinement_multiplier: Some(REFINEMENT_MULTIPLIER),
        schema_version: 3,
        tournament_id: tournament_id.to_owned(),
        metric: bundle.spec.metric,
        s0: s0.clone(),
        incoming_profiles,
        t: t.clone(),
        tertiary_rule_invoked: !incoming_unresolved && t.len() > 1,
        tertiary_rule: Some(match bundle.spec.metric {
            MetricKind::PrCnap => TertiaryRule::WorstTargetCnapRegret,
            MetricKind::Auroc => TertiaryRule::EmpiricalAurocRegret,
        }),
        operational_prevalences: (bundle.spec.metric == MetricKind::PrCnap)
            .then(|| config.tournament.operational_prevalences.clone()),
        operational_search: (bundle.spec.metric == MetricKind::PrCnap)
            .then_some(operational_search),
        operational_auroc_gamma: (bundle.spec.metric == MetricKind::Auroc)
            .then_some(config.tournament.operational_auroc_gamma),
        operational_scope_statement: match bundle.spec.metric {
            MetricKind::PrCnap => config.tournament.operational_scope_statement.clone(),
            MetricKind::Auroc => "Empirical AUROC regret at Gamma_op = 1, with uniform within-class weights on all observed units; candidates are T and benchmarks remain all of S0.".into(),
        },
        range_regret_profiles,
        auroc_regret_profiles,
        s_op: s_op.clone(),
        identical_complete_vectors_remain_tied: true,
    };

    let stage = stage_path(output)?;
    let result: Result<()> = (|| {
        write_json_new(&stage.join("selection.json"), &selection)?;
        for source in &mut source_files {
            source.path = crate::io::relative_path(output, &source.path)?;
        }
        source_files.sort_by(|left, right| left.path.cmp(&right.path));
        let manifest = serde_json::json!({
            "schema_version": 3,
            "tournament_id": tournament_id,
            "bundle_content_hash": bundle.manifest.bundle_content_hash,
            "metric": bundle.spec.metric,
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
        if sha256_file(&root.join(&source.path))? != source.sha256 {
            bail!("Version 2 source hash mismatch: {}", source.path.display())
        }
    }
    let selection: Version2Selection = read_json(&root.join("selection.json"))?;
    match selection.schema_version {
        1 => {
            if selection.tertiary_rule.is_some()
                || selection.operational_auroc_gamma.is_some()
                || !selection.auroc_regret_profiles.is_empty()
            {
                bail!("schema 1 uses the historical CNAP tertiary rule")
            }
        }
        2 | 3 => match selection.metric {
            MetricKind::PrCnap => {
                if selection.tertiary_rule != Some(TertiaryRule::WorstTargetCnapRegret)
                    || selection.operational_prevalences.is_none()
                    || selection.operational_search.is_none()
                    || selection.operational_auroc_gamma.is_some()
                    || !selection.auroc_regret_profiles.is_empty()
                {
                    bail!("inconsistent CNAP tertiary rule metadata")
                }
            }
            MetricKind::Auroc => {
                if selection.tertiary_rule != Some(TertiaryRule::EmpiricalAurocRegret)
                    || selection.operational_auroc_gamma != Some(1.0)
                    || selection.operational_prevalences.is_some()
                    || selection.operational_search.is_some()
                    || !selection.range_regret_profiles.is_empty()
                {
                    bail!("inconsistent empirical AUROC tertiary rule metadata")
                }
            }
        },
        schema => bail!("unsupported Version 2 selection schema {schema}"),
    }
    if selection.schema_version == 3 {
        let status = selection
            .numerical_status
            .as_deref()
            .context("missing numerical status")?;
        if !["resolved", "unresolved_incoming", "unresolved_regret"].contains(&status)
            || selection.refinement_multiplier != Some(REFINEMENT_MULTIPLIER)
        {
            bail!("invalid numerical resolution metadata");
        }
        if status == "unresolved_incoming"
            && (selection.tertiary_rule_invoked || selection.s_op != selection.t)
        {
            bail!("unresolved incoming comparisons must retain T without tertiary elimination");
        }
        for profile in &selection.incoming_profiles {
            if profile.descending_vector.len() != profile.descending_upper_vector.len()
                || profile
                    .descending_vector
                    .iter()
                    .zip(&profile.descending_upper_vector)
                    .any(|(lo, hi)| !lo.is_finite() || !hi.is_finite() || lo > hi)
            {
                bail!("invalid incoming bounds");
            }
        }
        for profile in &selection.range_regret_profiles {
            if profile.descending_vector.len() != profile.descending_lower_vector.len()
                || profile
                    .descending_lower_vector
                    .iter()
                    .zip(&profile.descending_vector)
                    .any(|(lo, hi)| !lo.is_finite() || !hi.is_finite() || lo > hi)
            {
                bail!("invalid regret bounds");
            }
        }
        let incoming_ids: BTreeSet<_> = selection
            .incoming_profiles
            .iter()
            .map(|p| p.candidate.clone())
            .collect();
        if incoming_ids != selection.s0.iter().cloned().collect()
            || incoming_ids.len() != selection.incoming_profiles.len()
        {
            bail!("incoming profiles do not cover S0 exactly");
        }
        let (expected_t, incoming_pending) =
            incoming_minimizers(&selection.s0, &selection.incoming_profiles);
        if selection.t != expected_t || incoming_pending != (status == "unresolved_incoming") {
            bail!("incoming selection or status differs from certified bounds");
        }
        if selection.metric == MetricKind::PrCnap && selection.tertiary_rule_invoked {
            let vectors: BTreeMap<_, _> = selection
                .range_regret_profiles
                .iter()
                .map(|p| {
                    (
                        p.candidate.clone(),
                        (
                            p.descending_lower_vector.clone(),
                            p.descending_vector.clone(),
                        ),
                    )
                })
                .collect();
            if vectors.keys().cloned().collect::<BTreeSet<_>>()
                != selection.t.iter().cloned().collect()
                || vectors.len() != selection.range_regret_profiles.len()
            {
                bail!("regret profiles do not cover T exactly");
            }
            let (expected_s_op, regret_pending) = interval_minimizers(&selection.t, &vectors);
            if selection.s_op != expected_s_op || regret_pending != (status == "unresolved_regret")
            {
                bail!("operational selection or status differs from certified bounds");
            }
        } else if status == "unresolved_regret" {
            bail!("regret cannot be unresolved without a CNAP tertiary comparison");
        }
    }
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
    fn overlapping_regrets_remain_unresolved_instead_of_ranking_upper_bounds() {
        let candidates = vec!["a".into(), "b".into()];
        let vectors = BTreeMap::from([
            ("a".into(), (vec![0.0], vec![2.0])),
            ("b".into(), (vec![1.0], vec![1.1])),
        ]);
        assert_eq!(
            interval_minimizers(&candidates, &vectors),
            (candidates.clone(), true)
        );
        let separated = BTreeMap::from([
            ("a".into(), (vec![0.1], vec![0.2])),
            ("b".into(), (vec![0.4], vec![0.5])),
        ]);
        assert_eq!(
            interval_minimizers(&candidates, &separated),
            (vec!["a".into()], false)
        );
        let exact_ties = BTreeMap::from([
            ("a".into(), (vec![0.2], vec![0.2])),
            ("b".into(), (vec![0.2], vec![0.2])),
        ]);
        assert_eq!(
            interval_minimizers(&candidates, &exact_ties),
            (candidates, false)
        );
    }

    #[test]
    fn uncertain_earlier_coordinate_cannot_be_skipped_in_lexicographic_order() {
        assert!(!definitely_less(
            &(vec![0.5, 0.0], vec![0.6, 0.0]),
            &(vec![0.5, 0.3], vec![0.6, 0.3])
        ));
        assert!(definitely_less(
            &(vec![1.0, 0.1], vec![1.0, 0.2]),
            &(vec![1.0, 0.3], vec![1.0, 0.4])
        ));
        let (lo, hi) = sorted_bounds(
            [
                NumericalBounds {
                    lower: 0.0,
                    upper: 0.9,
                },
                NumericalBounds {
                    lower: 0.5,
                    upper: 0.6,
                },
            ]
            .into_iter(),
        );
        assert_eq!(lo, [0.5, 0.0]);
        assert_eq!(hi, [0.9, 0.6]);
    }

    #[test]
    fn accelerated_and_exhaustive_finalizers_produce_identical_version2_profiles() {
        use crate::config::{
            HashedInput, MODELS, ModelConfig, SharedTournamentConfig, TensorInputs,
            TournamentConfig,
        };
        use directed_round_robin_organizer::{
            accelerated::*, input::*, plan::*, spec::*, system::*, *,
        };
        use supported_ap::{Prevalence, ReferenceAssessment};
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let dummy = root.join("input.json");
        fs::write(&dummy, "{}\n").unwrap();
        let input = HashedInput {
            path: dummy.clone(),
            sha256: sha256_file(&dummy).unwrap(),
        };
        let target = TargetPrevalences::closed_interval(
            Prevalence::new(0.01).unwrap(),
            Prevalence::new(0.3).unwrap(),
        )
        .unwrap();
        let config = PipelineConfig {
            roster_mode: Default::default(),
            schema_version: 1,
            provider_identity: "test".into(),
            candidate_count: 3,
            log_epsilon: 1e-8,
            models: MODELS
                .into_iter()
                .map(|id| {
                    (
                        id.into(),
                        ModelConfig {
                            display_label: id.into(),
                            metrics: input.clone(),
                            primary: TensorInputs {
                                tensor: input.clone(),
                                observations: input.clone(),
                                metadata: input.clone(),
                            },
                            query_q: None,
                        },
                    )
                })
                .collect(),
            tournament: TournamentConfig {
                selection_strategy: SelectionStrategy::CandidateConservative,
                shared: SharedTournamentConfig {
                    evidence_policy: EvidencePolicy {
                        magnitude_threshold: 0.0,
                        survival_floor: 0.0,
                        survival_requirement: 0.2,
                    },
                    computational_design: ComputationalDesign {
                        replications: 6,
                        computational_order: 1,
                        replication_positive_count: None,
                        replication_negative_count: None,
                        resampling_unit: ResamplingUnitSpec::IndependentObservation,
                    },
                    reference_assessment: ReferenceAssessment::not_asserted("test").unwrap(),
                    master_seed: 42,
                },
                pr: PrCnapSpecification {
                    target_prevalences: target.clone(),
                    search: SearchOptions::new(9, 1e-6, 8).unwrap(),
                    transport_justification: "test".into(),
                    retain_replication_profiles: false,
                },
                roc: AurocSpecification {
                    optimization: Default::default(),
                    concentration_search: supported_ap::ConcentrationSearchOptions::new(0.01, 4)
                        .unwrap(),
                },
                operational_prevalences: target,
                operational_search: SearchOptions::new(9, 1e-6, 8).unwrap(),
                operational_scope_statement: "S0 comparators".into(),
                operational_auroc_gamma: 1.0,
            },
        };
        let config_path = root.join("config.json");
        write_json_new(&config_path, &config).unwrap();
        for branch in ["pr", "roc"] {
            let directory = root.join(branch);
            fs::create_dir(&directory).unwrap();
            let spec = config.tournament_spec(branch).unwrap();
            let registry = SystemRegistry {
                schema_version: 2,
                systems: ["a", "b", "c"]
                    .into_iter()
                    .map(|id| SystemRecord {
                        system_id: id.into(),
                        display_label: id.into(),
                        score_column: id.into(),
                        annotations: BTreeMap::new(),
                    })
                    .collect(),
            };
            write_json_new(&directory.join("tournament_spec.json"), &spec).unwrap();
            write_json_new(&directory.join("systems.json"), &registry).unwrap();
            fs::write(
                directory.join("endpoints.csv"),
                "endpoint_id,label\ne1,1\ne2,1\ne3,1\ne4,0\ne5,0\ne6,0\n",
            )
            .unwrap();
            // A and B have identical rankings on different scales. Both survive
            // C, forcing complete incoming ties and the S0 regret tie breaker.
            fs::write(
                directory.join("scores.csv"),
                "endpoint_id,a,b,c\ne1,1,2,0\ne2,1,2,0\ne3,1,2,0\ne4,0,0,1\ne5,0,0,1\ne6,0,0,1\n",
            )
            .unwrap();
            fs::write(directory.join("provenance.json"), "{}\n").unwrap();
            let hashed = |name: &str| HashedPath {
                path: name.into(),
                sha256: sha256_file(&directory.join(name)).unwrap(),
            };
            let mut manifest = BundleManifest {
                schema_name: BUNDLE_SCHEMA_NAME.into(),
                schema_version: BUNDLE_SCHEMA_VERSION,
                metric: spec.metric,
                bundle_creation_time: "2026-09-09T00:00:00Z".into(),
                systems: hashed("systems.json"),
                tournament_spec: hashed("tournament_spec.json"),
                evaluations: vec![EvaluationManifest {
                    evaluation_id: "nci".into(),
                    endpoints: hashed("endpoints.csv"),
                    scores: hashed("scores.csv"),
                    source_provenance: hashed("provenance.json"),
                }],
                score_provider_identity: "test".into(),
                minimum_organizer_schema_version: BUNDLE_SCHEMA_VERSION,
                bundle_content_hash: String::new(),
            };
            let endpoints: Vec<_> = (1..=6).map(|i| format!("e{i}")).collect();
            let scores = [
                ("a", vec![1., 1., 1., 0., 0., 0.]),
                ("b", vec![2., 2., 2., 0., 0., 0.]),
                ("c", vec![0., 0., 0., 1., 1., 1.]),
            ];
            let hashes = PortableEvaluationHashes {
                evaluation_id: "nci".into(),
                label_vector_hash: identity::canonical_label_hash(
                    &endpoints,
                    &[true, true, true, false, false, false],
                ),
                score_vector_hashes: scores
                    .into_iter()
                    .map(|(id, v)| (id.into(), identity::canonical_vector_hash(&endpoints, &v)))
                    .collect(),
            };
            manifest.bundle_content_hash =
                compute_bundle_content_hash(&spec, &registry, &[hashes]).unwrap();
            write_json_new(&directory.join("bundle_manifest.json"), &manifest).unwrap();
            let bundle = load_bundle(&directory).unwrap();
            let full = plan_with_shard_count(&bundle, 1).unwrap();
            write_plan(&full, &directory.join("full_plan")).unwrap();
            run_shard(&bundle, &full, 0, &directory.join("full_results"), 2).unwrap();
            let reduction =
                reduce_tournament(&bundle, &full, &directory.join("full_results")).unwrap();
            write_reduction(&reduction, &directory.join("reduction")).unwrap();
            finalize_version2(
                &config_path,
                &directory,
                &directory.join("full_plan"),
                &directory.join("full_results"),
                &directory.join("reduction"),
                &directory.join("full_v2"),
            )
            .unwrap();
            let fast = plan_accelerated(
                &bundle,
                AcceleratedOptions {
                    batch_size: 1,
                    ..Default::default()
                },
            )
            .unwrap();
            write_accelerated_plan(&fast, &directory.join("fast_plan")).unwrap();
            run_accelerated(
                &bundle,
                &fast,
                &directory.join("fast_results"),
                &directory.join("selection"),
                2,
            )
            .unwrap();
            finalize_version2_accelerated(
                &config_path,
                &directory,
                &directory.join("fast_plan"),
                &directory.join("fast_results"),
                &directory.join("selection"),
                &directory.join("fast_v2"),
            )
            .unwrap();
            let old: serde_json::Value =
                read_json(&directory.join("full_v2/selection.json")).unwrap();
            let new: serde_json::Value =
                read_json(&directory.join("fast_v2/selection.json")).unwrap();
            assert_eq!(old, new, "{branch}");
            assert_eq!(new["s0"], serde_json::json!(["a", "b"]));
            assert_eq!(new["tertiary_rule_invoked"], true);
            assert_eq!(new["s_op"], serde_json::json!(["a", "b"]));
            assert_eq!(new["schema_version"], 3);
            if branch == "roc" {
                assert_eq!(new["tertiary_rule"], "empirical_auroc_regret");
                assert_eq!(new["operational_auroc_gamma"], 1.0);
                assert!(new.get("operational_prevalences").is_none());
                assert_eq!(new["range_regret_profiles"], serde_json::json!([]));
                assert_eq!(new["auroc_regret_profiles"].as_array().unwrap().len(), 2);
            } else {
                assert_eq!(new["tertiary_rule"], "worst_target_cnap_regret");
                assert!(new.get("operational_auroc_gamma").is_none());
                assert_eq!(new["range_regret_profiles"].as_array().unwrap().len(), 2);
            }
            audit_version2(&directory.join("fast_v2")).unwrap();
            if branch == "roc" {
                // Exercise the operational stage with two contexts and a
                // benchmark in S0 outside T. Worst regrets tie, so the second
                // coordinate must decide, independent of context ordering.
                let mut operational_bundle = bundle.clone();
                let mut first = bundle.evaluations["nci"].clone();
                first.scores = BTreeMap::from([
                    ("a".into(), vec![6., 2., 1., 5., 4., 3.]),
                    ("b".into(), vec![5., 4., 2., 6., 3., 1.]),
                    ("c".into(), vec![6., 5., 3., 4., 2., 1.]),
                ]);
                let mut second = first.clone();
                second.evaluation_id = "second".into();
                second.scores.insert("a".into(), first.scores["c"].clone());
                second.scores.insert("b".into(), first.scores["a"].clone());
                second
                    .scores
                    .insert("c".into(), vec![0., 0., 0., 1., 1., 1.]);
                operational_bundle.evaluations =
                    BTreeMap::from([("nci".into(), first), ("second".into(), second)]);
                let s0 = ["a", "b", "c"].map(String::from).to_vec();
                let mut incoming = Vec::new();
                for evaluation in operational_bundle.evaluations.keys() {
                    for winner in &s0 {
                        for loser in &s0 {
                            if winner != loser {
                                let strength = if evaluation == "second" && loser == "c" {
                                    0.1
                                } else {
                                    0.0
                                };
                                incoming.push(ActivationRow {
                                    activation_bounds: None,
                                    refinement_assessment: None,
                                    evaluation_id: evaluation.clone(),
                                    winner: winner.clone(),
                                    loser: loser.clone(),
                                    observed_supported_magnitude: strength,
                                    full_supported_magnitude: Some(strength),
                                    observed_survival_fraction: strength,
                                    full_survival_fraction: Some(strength),
                                    magnitude_requirements_pass: strength > 0.0,
                                    activation_strength: strength,
                                });
                            }
                        }
                    }
                }
                let summary = finalize_from_rows(
                    &config,
                    &operational_bundle,
                    "operational_test",
                    s0,
                    incoming,
                    vec![],
                    &directory.join("multi_context_v2"),
                )
                .unwrap();
                assert_eq!(summary.t, ["a", "b"]);
                assert_eq!(summary.s_op, ["a"]);
                let selection: Version2Selection =
                    read_json(&summary.output.join("selection.json")).unwrap();
                assert_eq!(
                    selection.auroc_regret_profiles[0].descending_vector,
                    [5. / 9., 0.]
                );
                assert_eq!(
                    selection.auroc_regret_profiles[1].descending_vector,
                    [5. / 9., 3. / 9.]
                );
                audit_version2(&summary.output).unwrap();
            }
        }
    }

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

    #[test]
    fn auroc_regret_uses_all_s0_and_can_disagree_with_cnap() {
        let labels = [true, true, true, false, false, false];
        let scores = BTreeMap::from([
            ("a".into(), vec![6., 2., 1., 5., 4., 3.]),
            ("b".into(), vec![5., 4., 2., 6., 3., 1.]),
            ("b_tied".into(), vec![10., 8., 4., 12., 6., 2.]),
            ("c".into(), vec![6., 5., 3., 4., 2., 1.]),
            ("excluded".into(), vec![1., 1., 1., 0., 0., 0.]),
        ]);
        let s0 = ["a", "b", "b_tied", "c"].map(String::from);
        let t = ["a", "b", "b_tied"].map(String::from);
        let rows = empirical_auroc_regrets("nci", &labels, &scores, &s0, &t).unwrap();
        assert!(rows.iter().all(|row| row.comparator == "c"));
        assert_eq!(rows[0].regret, 5. / 9.);
        assert_eq!(rows[1].regret, 3. / 9.);
        assert_eq!(rows[1].regret, rows[2].regret);
        let vectors = rows
            .iter()
            .map(|row| (row.candidate.as_str(), vec![row.regret]))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(lex_minimizers(&t, |id| &vectors[id]), vec!["b", "b_tied"]);
        let prevalence = supported_ap::Prevalence::new(0.1).unwrap();
        assert!(
            supported_ap::observed_cnap(&scores["a"], &labels, prevalence).unwrap()
                > supported_ap::observed_cnap(&scores["b"], &labels, prevalence).unwrap()
        );
    }

    #[test]
    fn auroc_regret_awards_half_credit_for_score_ties() {
        let labels = [true, false];
        let scores = BTreeMap::from([("tied".into(), vec![1., 1.]), ("best".into(), vec![1., 0.])]);
        let s0 = ["tied", "best"].map(String::from);
        let rows = empirical_auroc_regrets("nci", &labels, &scores, &s0, &s0).unwrap();
        assert_eq!(rows[0].candidate_auroc, 0.5);
        assert_eq!(rows[0].regret, 0.5);
        assert_eq!(rows[1].regret, 0.0);
    }
}
