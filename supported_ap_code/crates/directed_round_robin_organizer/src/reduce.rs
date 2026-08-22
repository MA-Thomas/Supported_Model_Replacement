use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::artifact::{create_staging_directory, publish_staging_directory};
use crate::audit::{CompletenessAudit, audit_results_with_artifacts};
use crate::graph::{DirectedEdge, GraphSummary, analyze_graph};
use crate::identity::sha256_file;
use crate::input::LoadedBundle;
use crate::plan::TournamentPlan;
use crate::spec::{DirectedVerdict, MetricKind, SelectionStrategy};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtomicDirectedVerdict {
    pub match_id: String,
    pub evaluation_id: String,
    pub winner: String,
    pub loser: String,
    pub verdict: DirectedVerdict,
    pub official_verdict: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConjunctiveDirectedVerdict {
    pub winner: String,
    pub loser: String,
    pub verdict: DirectedVerdict,
    pub evaluation_verdicts: BTreeMap<String, DirectedVerdict>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DescriptiveMetricRow {
    pub match_id: String,
    pub evaluation_id: String,
    pub system_low: String,
    pub system_high: String,
    pub empirical_metric_low: Option<f64>,
    pub empirical_metric_high: Option<f64>,
    pub observed_difference_low_over_high: f64,
    pub observed_difference_high_over_low: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreVectorComparison {
    pub evaluation_id: String,
    pub system_a: String,
    pub system_b: String,
    pub exact_vector_identical: bool,
    pub rank_equivalent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SelectionOutcome {
    UniqueConjecturedSystem {
        system_id: String,
    },
    UnresolvedCandidateSet {
        system_ids: Vec<String>,
    },
    /// Candidate-conservative only: the intersection of per-evaluation maximal
    /// sets is empty, i.e. no system is maximal in every evaluation.
    NoSurvivingCandidate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurvivorComparison {
    pub opponent_system_id: String,
    pub survivor_over_opponent: DirectedVerdict,
    pub opponent_over_survivor: DirectedVerdict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurvivorEvidence {
    pub system_id: String,
    pub comparisons: Vec<SurvivorComparison>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionResult {
    pub metric: MetricKind,
    pub strategy: SelectionStrategy,
    pub graph_maximal_systems: Vec<String>,
    pub survivor_evidence: Vec<SurvivorEvidence>,
    pub outcome: SelectionOutcome,
    pub operational_choice: Option<String>,
    pub operational_choice_is_non_evidential: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationMaximalSet {
    pub evaluation_id: String,
    pub maximal_systems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reduction {
    pub schema_version: u32,
    pub tournament_id: String,
    pub plan_id: String,
    pub bundle_content_hash: String,
    pub metric: MetricKind,
    pub selection_strategy: SelectionStrategy,
    pub completeness_audit: CompletenessAudit,
    pub atomic_directed_verdicts: Vec<AtomicDirectedVerdict>,
    pub evaluation_conjunctive_verdicts: Vec<ConjunctiveDirectedVerdict>,
    pub system_graph: GraphSummary,
    pub selection: SelectionResult,
    pub alternate_selection: SelectionResult,
    pub per_evaluation_maximal: Vec<EvaluationMaximalSet>,
    pub descriptive_metrics: Vec<DescriptiveMetricRow>,
    pub score_vector_comparisons: Vec<ScoreVectorComparison>,
}

/// A graph-theoretic view induced from an already audited complete tournament.
/// It is deliberately not represented as a second tournament reduction: all
/// provenance continues to point to the one immutable full-roster plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InducedReductionView {
    pub schema_version: u32,
    pub tournament_id: String,
    pub plan_id: String,
    pub bundle_content_hash: String,
    pub metric: MetricKind,
    pub selection_strategy: SelectionStrategy,
    pub source_reduction_manifest_sha256: String,
    pub included_systems: Vec<String>,
    pub excluded_systems: Vec<String>,
    pub source_tournament_complete: bool,
    pub induced_pair_cohort_count: usize,
    pub atomic_directed_verdicts: Vec<AtomicDirectedVerdict>,
    pub evaluation_conjunctive_verdicts: Vec<ConjunctiveDirectedVerdict>,
    pub system_graph: GraphSummary,
    pub selection: SelectionResult,
    pub alternate_selection: SelectionResult,
    pub per_evaluation_maximal: Vec<EvaluationMaximalSet>,
    pub descriptive_metrics: Vec<DescriptiveMetricRow>,
    pub score_vector_comparisons: Vec<ScoreVectorComparison>,
}

pub fn reduce_tournament(
    bundle: &LoadedBundle,
    plan: &TournamentPlan,
    results_root: &Path,
) -> Result<Reduction> {
    let (completeness_audit, artifacts) = audit_results_with_artifacts(bundle, plan, results_root)?;
    if !completeness_audit.complete {
        return Err(Error::Incomplete(format!(
            "{} missing, {} invalid, {} incompatible result artifact(s)",
            completeness_audit.missing_matches,
            completeness_audit.invalid_or_corrupt_artifacts,
            completeness_audit.incompatible_results
        )));
    }
    let mut atomic = Vec::with_capacity(plan.matches.len() * 2);
    let mut descriptive_metrics = Vec::with_capacity(plan.matches.len());
    for (item, artifact) in plan.matches.iter().zip(artifacts) {
        let artifact = artifact.ok_or_else(|| {
            Error::Incomplete(format!(
                "validated artifact disappeared from reduction input: {}",
                item.match_id
            ))
        })?;
        for direction in [
            &artifact.judged.low_over_high,
            &artifact.judged.high_over_low,
        ] {
            atomic.push(AtomicDirectedVerdict {
                match_id: item.match_id.clone(),
                evaluation_id: item.evaluation_id.clone(),
                winner: direction.winner.clone(),
                loser: direction.loser.clone(),
                verdict: direction.verdict,
                official_verdict: direction.official_verdict.clone(),
            });
        }
        let values = &artifact.judged.descriptive;
        descriptive_metrics.push(DescriptiveMetricRow {
            match_id: item.match_id.clone(),
            evaluation_id: item.evaluation_id.clone(),
            system_low: item.system_low.clone(),
            system_high: item.system_high.clone(),
            empirical_metric_low: values.empirical_metric_low,
            empirical_metric_high: values.empirical_metric_high,
            observed_difference_low_over_high: values.observed_difference_low_over_high,
            observed_difference_high_over_low: values.observed_difference_high_over_low,
        });
    }
    atomic.sort_by(|left, right| {
        (&left.evaluation_id, &left.winner, &left.loser).cmp(&(
            &right.evaluation_id,
            &right.winner,
            &right.loser,
        ))
    });
    let conjunctive = conjunctive_table(&atomic, &bundle.spec.evaluations)?;
    let system_ids: Vec<_> = bundle
        .registry
        .sorted_systems()
        .iter()
        .map(|system| system.system_id.clone())
        .collect();
    let system_graph = graph_from_conjunctive(&system_ids, &conjunctive)?;
    let per_evaluation_maximal =
        evaluation_maximal_sets(&system_ids, &atomic, &bundle.spec.evaluations)?;
    let (selection, alternate_selection) = strategy_selections(
        bundle,
        bundle.spec.selection_strategy,
        &system_graph,
        &conjunctive,
        &per_evaluation_maximal,
        &system_ids,
    )?;
    let score_vector_comparisons = compare_score_vectors(bundle);
    Ok(Reduction {
        schema_version: 1,
        tournament_id: plan.tournament_id.clone(),
        plan_id: plan.plan_id.clone(),
        bundle_content_hash: plan.bundle_content_hash.clone(),
        metric: plan.metric,
        selection_strategy: bundle.spec.selection_strategy,
        completeness_audit,
        atomic_directed_verdicts: atomic,
        evaluation_conjunctive_verdicts: conjunctive,
        system_graph,
        selection,
        alternate_selection,
        per_evaluation_maximal,
        descriptive_metrics,
        score_vector_comparisons,
    })
}

pub fn conjoin_verdicts(values: impl IntoIterator<Item = DirectedVerdict>) -> DirectedVerdict {
    let values: Vec<_> = values.into_iter().collect();
    if values
        .iter()
        .all(|value| *value == DirectedVerdict::Supported)
    {
        DirectedVerdict::Supported
    } else if values
        .iter()
        .all(|value| *value != DirectedVerdict::NotSupported)
        && values.contains(&DirectedVerdict::Unresolved)
    {
        DirectedVerdict::Unresolved
    } else {
        DirectedVerdict::NotSupported
    }
}

pub(crate) fn conjunctive_table(
    atomic: &[AtomicDirectedVerdict],
    evaluations: &[String],
) -> Result<Vec<ConjunctiveDirectedVerdict>> {
    let mut grouped: BTreeMap<(String, String), BTreeMap<String, DirectedVerdict>> =
        BTreeMap::new();
    for row in atomic {
        if grouped
            .entry((row.winner.clone(), row.loser.clone()))
            .or_default()
            .insert(row.evaluation_id.clone(), row.verdict)
            .is_some()
        {
            return Err(Error::InvalidReduction(format!(
                "duplicate atomic direction {} -> {} in {}",
                row.winner, row.loser, row.evaluation_id
            )));
        }
    }
    let expected: BTreeSet<_> = evaluations.iter().cloned().collect();
    let mut output = Vec::with_capacity(grouped.len());
    for ((winner, loser), evaluation_verdicts) in grouped {
        if evaluation_verdicts.keys().cloned().collect::<BTreeSet<_>>() != expected {
            return Err(Error::InvalidReduction(format!(
                "direction {winner} -> {loser} lacks a complete evaluation set"
            )));
        }
        let verdict = conjoin_verdicts(evaluation_verdicts.values().copied());
        output.push(ConjunctiveDirectedVerdict {
            winner,
            loser,
            verdict,
            evaluation_verdicts,
        });
    }
    Ok(output)
}

pub(crate) fn graph_from_conjunctive(
    vertices: &[String],
    rows: &[ConjunctiveDirectedVerdict],
) -> Result<GraphSummary> {
    let edges = rows
        .iter()
        .filter(|row| row.verdict == DirectedVerdict::Supported)
        .map(|row| DirectedEdge {
            from: row.winner.clone(),
            to: row.loser.clone(),
        });
    let unresolved = rows
        .iter()
        .filter(|row| row.verdict == DirectedVerdict::Unresolved)
        .map(|row| DirectedEdge {
            from: row.winner.clone(),
            to: row.loser.clone(),
        });
    analyze_graph(vertices.iter().cloned(), edges, unresolved)
}

/// Per-evaluation source-SCC maximal sets. Each evaluation's supported directed
/// verdicts form its own graph; a system is maximal in that evaluation when no
/// supported edge enters it there. Deterministic in `evaluations` order.
pub(crate) fn evaluation_maximal_sets(
    system_ids: &[String],
    atomic: &[AtomicDirectedVerdict],
    evaluations: &[String],
) -> Result<Vec<EvaluationMaximalSet>> {
    let mut out = Vec::with_capacity(evaluations.len());
    for evaluation in evaluations {
        let edges = atomic
            .iter()
            .filter(|row| {
                row.evaluation_id == *evaluation && row.verdict == DirectedVerdict::Supported
            })
            .map(|row| DirectedEdge {
                from: row.winner.clone(),
                to: row.loser.clone(),
            });
        let unresolved = atomic
            .iter()
            .filter(|row| {
                row.evaluation_id == *evaluation && row.verdict == DirectedVerdict::Unresolved
            })
            .map(|row| DirectedEdge {
                from: row.winner.clone(),
                to: row.loser.clone(),
            });
        let graph = analyze_graph(system_ids.iter().cloned(), edges, unresolved)?;
        out.push(EvaluationMaximalSet {
            evaluation_id: evaluation.clone(),
            maximal_systems: graph.maximal_vertices,
        });
    }
    Ok(out)
}

/// Candidate-conservative survivors: systems maximal in *every* evaluation
/// (the intersection of the per-evaluation maximal sets). May be empty.
pub(crate) fn candidate_conservative_survivors(
    per_evaluation_maximal: &[EvaluationMaximalSet],
) -> Vec<String> {
    let mut iterator = per_evaluation_maximal.iter();
    let Some(first) = iterator.next() else {
        return Vec::new();
    };
    let mut survivors: BTreeSet<String> = first.maximal_systems.iter().cloned().collect();
    for evaluation in iterator {
        let current: BTreeSet<String> = evaluation.maximal_systems.iter().cloned().collect();
        survivors = survivors.intersection(&current).cloned().collect();
    }
    survivors.into_iter().collect()
}

/// Computes both strategy selections and returns `(active, alternate)` for the
/// requested `strategy`. Both are always produced so a reduction can report each
/// regardless of which one is primary.
pub(crate) fn strategy_selections(
    bundle: &LoadedBundle,
    strategy: SelectionStrategy,
    system_graph: &GraphSummary,
    conjunctive: &[ConjunctiveDirectedVerdict],
    per_evaluation_maximal: &[EvaluationMaximalSet],
    all_system_ids: &[String],
) -> Result<(SelectionResult, SelectionResult)> {
    let replacement = build_selection_result(
        bundle,
        SelectionStrategy::ReplacementConservative,
        &system_graph.maximal_vertices,
        conjunctive,
        all_system_ids,
        false,
    )?;
    let survivors = candidate_conservative_survivors(per_evaluation_maximal);
    let candidate = build_selection_result(
        bundle,
        SelectionStrategy::CandidateConservative,
        &survivors,
        conjunctive,
        all_system_ids,
        true,
    )?;
    Ok(match strategy {
        SelectionStrategy::ReplacementConservative => (replacement, candidate),
        SelectionStrategy::CandidateConservative => (candidate, replacement),
    })
}

/// Assembles a `SelectionResult` from an explicit survivor set under a given
/// strategy. `allow_empty` permits the candidate-conservative outcome where the
/// intersection of per-evaluation maximal sets is empty; the replacement rule
/// always yields a nonempty source component and never sets it.
fn build_selection_result(
    bundle: &LoadedBundle,
    strategy: SelectionStrategy,
    survivors: &[String],
    conjunctive: &[ConjunctiveDirectedVerdict],
    all_system_ids: &[String],
    allow_empty: bool,
) -> Result<SelectionResult> {
    let survivors: Vec<String> = survivors.to_vec();
    if survivors.is_empty() && !allow_empty {
        return Err(Error::InvalidReduction(
            "selection ladder produced no system".into(),
        ));
    }
    let outcome = if survivors.is_empty() {
        SelectionOutcome::NoSurvivingCandidate
    } else if survivors.len() == 1 {
        SelectionOutcome::UniqueConjecturedSystem {
            system_id: survivors[0].clone(),
        }
    } else {
        SelectionOutcome::UnresolvedCandidateSet {
            system_ids: survivors.clone(),
        }
    };
    let operational_choice = bundle
        .spec
        .operational_tie_break
        .as_ref()
        .and_then(|order| {
            order
                .iter()
                .find(|system_id| survivors.contains(system_id))
                .cloned()
        });
    if bundle.spec.operational_tie_break.is_some()
        && !survivors.is_empty()
        && operational_choice.is_none()
    {
        return Err(Error::InvalidReduction(
            "operational tie-break contains no surviving system".into(),
        ));
    }
    let verdict_lookup: BTreeMap<_, _> = conjunctive
        .iter()
        .map(|row| ((row.winner.as_str(), row.loser.as_str()), row.verdict))
        .collect();
    let mut survivor_evidence = Vec::new();
    for survivor in &survivors {
        let mut comparisons = Vec::new();
        for opponent in all_system_ids {
            if survivor == opponent {
                continue;
            }
            let survivor_over_opponent = verdict_lookup
                .get(&(survivor.as_str(), opponent.as_str()))
                .copied()
                .ok_or_else(|| Error::InvalidReduction("missing survivor comparison".into()))?;
            let opponent_over_survivor = verdict_lookup
                .get(&(opponent.as_str(), survivor.as_str()))
                .copied()
                .ok_or_else(|| {
                    Error::InvalidReduction("missing reverse survivor comparison".into())
                })?;
            comparisons.push(SurvivorComparison {
                opponent_system_id: opponent.clone(),
                survivor_over_opponent,
                opponent_over_survivor,
            });
        }
        survivor_evidence.push(SurvivorEvidence {
            system_id: survivor.clone(),
            comparisons,
        });
    }
    Ok(SelectionResult {
        metric: bundle.spec.metric,
        strategy,
        graph_maximal_systems: survivors,
        survivor_evidence,
        outcome,
        operational_choice,
        operational_choice_is_non_evidential: bundle.spec.operational_tie_break.is_some(),
    })
}

fn compare_score_vectors(bundle: &LoadedBundle) -> Vec<ScoreVectorComparison> {
    let systems = bundle.registry.sorted_systems();
    let mut comparisons = Vec::new();
    for (evaluation_id, evaluation) in &bundle.evaluations {
        for first in 0..systems.len() {
            for second in first + 1..systems.len() {
                let system_a = systems[first];
                let system_b = systems[second];
                let scores_a = &evaluation.scores[&system_a.system_id];
                let scores_b = &evaluation.scores[&system_b.system_id];
                comparisons.push(ScoreVectorComparison {
                    evaluation_id: evaluation_id.clone(),
                    system_a: system_a.system_id.clone(),
                    system_b: system_b.system_id.clone(),
                    exact_vector_identical: evaluation.score_vector_hashes[&system_a.system_id]
                        == evaluation.score_vector_hashes[&system_b.system_id],
                    rank_equivalent: rank_groups(scores_a) == rank_groups(scores_b),
                });
            }
        }
    }
    comparisons
}

pub fn induce_reduction_view(
    bundle: &LoadedBundle,
    source: &Reduction,
    source_reduction_directory: &Path,
    excluded_systems: &[String],
) -> Result<InducedReductionView> {
    if !source.completeness_audit.complete {
        return Err(Error::Incomplete(
            "cannot induce a view from an incomplete tournament reduction".into(),
        ));
    }
    let all_systems: BTreeSet<String> = bundle
        .registry
        .sorted_systems()
        .into_iter()
        .map(|system| system.system_id.clone())
        .collect();
    let excluded: BTreeSet<String> = excluded_systems.iter().cloned().collect();
    if excluded.is_empty() || excluded.len() != excluded_systems.len() {
        return Err(Error::InvalidReduction(
            "an induced view requires a nonempty, duplicate-free exclusion set".into(),
        ));
    }
    if !excluded.is_subset(&all_systems) {
        let unknown: Vec<_> = excluded.difference(&all_systems).cloned().collect();
        return Err(Error::InvalidReduction(format!(
            "induced view excludes unknown system(s): {}",
            unknown.join(", ")
        )));
    }
    let included: Vec<String> = all_systems.difference(&excluded).cloned().collect();
    if included.len() < 2 {
        return Err(Error::InvalidReduction(
            "an induced view must retain at least two systems".into(),
        ));
    }
    let included_set: BTreeSet<&str> = included.iter().map(String::as_str).collect();
    let atomic: Vec<_> = source
        .atomic_directed_verdicts
        .iter()
        .filter(|row| {
            included_set.contains(row.winner.as_str()) && included_set.contains(row.loser.as_str())
        })
        .cloned()
        .collect();
    let conjunctive = conjunctive_table(&atomic, &bundle.spec.evaluations)?;
    let graph = graph_from_conjunctive(&included, &conjunctive)?;
    let per_evaluation_maximal =
        evaluation_maximal_sets(&included, &atomic, &bundle.spec.evaluations)?;
    let (selection, alternate_selection) = strategy_selections(
        bundle,
        bundle.spec.selection_strategy,
        &graph,
        &conjunctive,
        &per_evaluation_maximal,
        &included,
    )?;
    let descriptive_metrics: Vec<_> = source
        .descriptive_metrics
        .iter()
        .filter(|row| {
            included_set.contains(row.system_low.as_str())
                && included_set.contains(row.system_high.as_str())
        })
        .cloned()
        .collect();
    let score_vector_comparisons: Vec<_> = source
        .score_vector_comparisons
        .iter()
        .filter(|row| {
            included_set.contains(row.system_a.as_str())
                && included_set.contains(row.system_b.as_str())
        })
        .cloned()
        .collect();
    let pair_cohorts = bundle.spec.evaluations.len() * included.len() * (included.len() - 1) / 2;
    if descriptive_metrics.len() != pair_cohorts || atomic.len() != 2 * pair_cohorts {
        return Err(Error::InvalidReduction(
            "induced view does not contain the expected complete pair-cohort set".into(),
        ));
    }
    let source_manifest = source_reduction_directory.join("reduction_manifest.json");
    Ok(InducedReductionView {
        schema_version: 1,
        tournament_id: source.tournament_id.clone(),
        plan_id: source.plan_id.clone(),
        bundle_content_hash: source.bundle_content_hash.clone(),
        metric: source.metric,
        selection_strategy: bundle.spec.selection_strategy,
        source_reduction_manifest_sha256: sha256_file(&source_manifest)?,
        included_systems: included,
        excluded_systems: excluded.into_iter().collect(),
        source_tournament_complete: true,
        induced_pair_cohort_count: pair_cohorts,
        atomic_directed_verdicts: atomic,
        evaluation_conjunctive_verdicts: conjunctive,
        system_graph: graph,
        selection,
        alternate_selection,
        per_evaluation_maximal,
        descriptive_metrics,
        score_vector_comparisons,
    })
}

#[derive(Debug, Serialize)]
struct InducedViewManifest {
    schema_version: u32,
    tournament_id: String,
    plan_id: String,
    bundle_content_hash: String,
    metric: MetricKind,
    source_reduction_manifest_sha256: String,
    included_system_count: usize,
    excluded_systems: Vec<String>,
    induced_pair_cohort_count: usize,
    induced_view_sha256: String,
}

pub fn write_induced_reduction_view(view: &InducedReductionView, output: &Path) -> Result<()> {
    let staging = create_staging_directory(output)?;
    let view_path = staging.join("induced_view.json");
    write_json_new(&view_path, view)?;
    let manifest = InducedViewManifest {
        schema_version: 1,
        tournament_id: view.tournament_id.clone(),
        plan_id: view.plan_id.clone(),
        bundle_content_hash: view.bundle_content_hash.clone(),
        metric: view.metric,
        source_reduction_manifest_sha256: view.source_reduction_manifest_sha256.clone(),
        included_system_count: view.included_systems.len(),
        excluded_systems: view.excluded_systems.clone(),
        induced_pair_cohort_count: view.induced_pair_cohort_count,
        induced_view_sha256: sha256_file(&view_path)?,
    };
    write_json_new(&staging.join("induced_view_manifest.json"), &manifest)?;
    publish_staging_directory(&staging, output)
}

fn rank_groups(scores: &[f64]) -> Vec<usize> {
    let mut indices: Vec<_> = (0..scores.len()).collect();
    indices.sort_by(|&left, &right| {
        scores[right]
            .total_cmp(&scores[left])
            .then_with(|| left.cmp(&right))
    });
    let mut groups = vec![0; scores.len()];
    let mut group = 0;
    for (position, &index) in indices.iter().enumerate() {
        if position > 0 && scores[index] != scores[indices[position - 1]] {
            group += 1;
        }
        groups[index] = group;
    }
    groups
}

#[derive(Debug, Serialize, Deserialize)]
struct ReductionManifest {
    schema_version: u32,
    tournament_id: String,
    plan_id: String,
    bundle_content_hash: String,
    metric: MetricKind,
    selection_strategy: SelectionStrategy,
    files: BTreeMap<String, String>,
}

pub fn write_reduction(reduction: &Reduction, output: &Path) -> Result<()> {
    let staging = create_staging_directory(output)?;
    write_reduction_into(reduction, &staging)?;
    publish_staging_directory(&staging, output)
}

fn write_reduction_into(reduction: &Reduction, output: &Path) -> Result<()> {
    let mut files = BTreeMap::new();
    write_json_file(
        output,
        "completeness_audit.json",
        &reduction.completeness_audit,
        &mut files,
    )?;
    write_csv_file(
        output,
        "atomic_directed_verdicts.csv",
        &reduction.atomic_directed_verdicts,
        &mut files,
    )?;
    write_csv_file(
        output,
        "evaluation_conjunctive_verdicts.csv",
        &reduction.evaluation_conjunctive_verdicts,
        &mut files,
    )?;
    write_json_file(
        output,
        "system_graph.json",
        &reduction.system_graph,
        &mut files,
    )?;
    write_json_file(
        output,
        "system_components.json",
        &serde_json::json!({
            "strongly_connected_components": reduction.system_graph.strongly_connected_components,
            "condensation_edges": reduction.system_graph.condensation_edges,
            "source_component_indices": reduction.system_graph.source_component_indices,
            "maximal_vertices": reduction.system_graph.maximal_vertices,
            "vertex_status": reduction.system_graph.vertex_status,
        }),
        &mut files,
    )?;
    write_json_file(output, "selection.json", &reduction.selection, &mut files)?;
    write_json_file(
        output,
        "alternate_selection.json",
        &reduction.alternate_selection,
        &mut files,
    )?;
    write_json_file(
        output,
        "per_evaluation_maximal.json",
        &reduction.per_evaluation_maximal,
        &mut files,
    )?;
    write_csv_file(
        output,
        "descriptive_metrics.csv",
        &reduction.descriptive_metrics,
        &mut files,
    )?;
    write_csv_file(
        output,
        "score_vector_comparisons.csv",
        &reduction.score_vector_comparisons,
        &mut files,
    )?;
    let manifest = ReductionManifest {
        schema_version: 1,
        tournament_id: reduction.tournament_id.clone(),
        plan_id: reduction.plan_id.clone(),
        bundle_content_hash: reduction.bundle_content_hash.clone(),
        metric: reduction.metric,
        selection_strategy: reduction.selection_strategy,
        files,
    };
    write_json_new(&output.join("reduction_manifest.json"), &manifest)
}

fn write_json_file<T: Serialize>(
    output: &Path,
    name: &str,
    value: &T,
    files: &mut BTreeMap<String, String>,
) -> Result<()> {
    let path = output.join(name);
    write_json_new(&path, value)?;
    files.insert(name.to_owned(), sha256_file(&path)?);
    Ok(())
}

fn write_csv_file<T: Serialize>(
    output: &Path,
    name: &str,
    rows: &[T],
    files: &mut BTreeMap<String, String>,
) -> Result<()> {
    let path = output.join(name);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| crate::error::io(&path, error))?;
    let mut writer = csv::Writer::from_writer(file);
    let json_rows = rows
        .iter()
        .map(|row| {
            serde_json::to_value(row).map_err(|source| Error::Json {
                path: path.clone(),
                source,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let Some(first_row) = json_rows.first() else {
        writer
            .flush()
            .map_err(|error| crate::error::io(&path, error))?;
        files.insert(name.to_owned(), sha256_file(&path)?);
        return Ok(());
    };
    let headers: Vec<String> = first_row
        .as_object()
        .ok_or_else(|| Error::InvalidReduction(format!("{name} rows must serialize as objects")))?
        .keys()
        .cloned()
        .collect();
    writer.write_record(&headers).map_err(|source| Error::Csv {
        path: path.clone(),
        source,
    })?;
    for row in json_rows {
        let object = row.as_object().ok_or_else(|| {
            Error::InvalidReduction(format!("{name} row did not serialize as an object"))
        })?;
        let record = headers
            .iter()
            .map(|header| json_cell(&object[header]))
            .collect::<Result<Vec<_>>>()?;
        writer.write_record(record).map_err(|source| Error::Csv {
            path: path.clone(),
            source,
        })?;
    }
    writer
        .flush()
        .map_err(|error| crate::error::io(&path, error))?;
    files.insert(name.to_owned(), sha256_file(&path)?);
    Ok(())
}

fn json_cell(value: &serde_json::Value) -> Result<String> {
    match value {
        serde_json::Value::Null => Ok(String::new()),
        serde_json::Value::Bool(value) => Ok(value.to_string()),
        serde_json::Value::Number(value) => Ok(value.to_string()),
        serde_json::Value::String(value) => Ok(value.clone()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => serde_json::to_string(value)
            .map_err(|source| Error::Json {
                path: "<csv-nested-cell>".into(),
                source,
            }),
    }
}

fn write_json_new<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| crate::error::io(path, error))?;
    serde_json::to_writer_pretty(&mut file, value).map_err(|source| Error::Json {
        path: path.to_owned(),
        source,
    })?;
    file.write_all(b"\n")
        .map_err(|error| crate::error::io(path, error))?;
    file.sync_all()
        .map_err(|error| crate::error::io(path, error))
}

pub fn audit_reduction(directory: &Path) -> Result<()> {
    audit_reduction_manifest(directory).map(|_| ())
}

pub fn audit_reduction_for(directory: &Path, plan: &TournamentPlan) -> Result<()> {
    let manifest = audit_reduction_manifest(directory)?;
    if manifest.tournament_id != plan.tournament_id
        || manifest.plan_id != plan.plan_id
        || manifest.bundle_content_hash != plan.bundle_content_hash
        || manifest.metric != plan.metric
    {
        return Err(Error::InvalidReduction(
            "reduction manifest does not belong to the supplied tournament plan".into(),
        ));
    }
    Ok(())
}

/// Loads an already-audited reduction and revalidates its scientific content
/// against the immutable bundle and plan. Revision composition can therefore
/// consume compact reductions instead of repeatedly parsing every raw match
/// report.
pub fn load_reduction_for(
    directory: &Path,
    bundle: &LoadedBundle,
    plan: &TournamentPlan,
) -> Result<Reduction> {
    audit_reduction_for(directory, plan)?;
    let selection: SelectionResult = read_json_file(&directory.join("selection.json"))?;
    let selection_strategy = selection.strategy;
    let reduction = Reduction {
        schema_version: 1,
        tournament_id: plan.tournament_id.clone(),
        plan_id: plan.plan_id.clone(),
        bundle_content_hash: plan.bundle_content_hash.clone(),
        metric: plan.metric,
        selection_strategy,
        completeness_audit: read_json_file(&directory.join("completeness_audit.json"))?,
        atomic_directed_verdicts: read_csv_rows(&directory.join("atomic_directed_verdicts.csv"))?,
        evaluation_conjunctive_verdicts: read_csv_rows(
            &directory.join("evaluation_conjunctive_verdicts.csv"),
        )?,
        system_graph: read_json_file(&directory.join("system_graph.json"))?,
        selection,
        alternate_selection: read_json_file(&directory.join("alternate_selection.json"))?,
        per_evaluation_maximal: read_json_file(&directory.join("per_evaluation_maximal.json"))?,
        descriptive_metrics: read_csv_rows(&directory.join("descriptive_metrics.csv"))?,
        score_vector_comparisons: read_csv_rows(&directory.join("score_vector_comparisons.csv"))?,
    };
    validate_loaded_reduction(&reduction, bundle, plan)?;
    Ok(reduction)
}

fn validate_loaded_reduction(
    reduction: &Reduction,
    bundle: &LoadedBundle,
    plan: &TournamentPlan,
) -> Result<()> {
    if reduction.schema_version != 1
        || reduction.tournament_id != plan.tournament_id
        || reduction.plan_id != plan.plan_id
        || reduction.bundle_content_hash != plan.bundle_content_hash
        || reduction.metric != plan.metric
        || !reduction.completeness_audit.complete
        || reduction.completeness_audit.plan_id != plan.plan_id
        || reduction.completeness_audit.planned_matches != plan.expected_match_count
        || reduction.completeness_audit.valid_completed_matches != plan.expected_match_count
    {
        return Err(Error::InvalidReduction(
            "reduction identity or completeness audit does not match its plan".into(),
        ));
    }

    let mut directions_by_match: BTreeMap<&str, Vec<&AtomicDirectedVerdict>> = BTreeMap::new();
    for row in &reduction.atomic_directed_verdicts {
        directions_by_match
            .entry(row.match_id.as_str())
            .or_default()
            .push(row);
    }
    if directions_by_match.len() != plan.matches.len() {
        return Err(Error::InvalidReduction(
            "reduction does not contain exactly one directed pair per planned match".into(),
        ));
    }
    for item in &plan.matches {
        let rows = directions_by_match
            .get(item.match_id.as_str())
            .ok_or_else(|| {
                Error::InvalidReduction(format!("reduction is missing match {}", item.match_id))
            })?;
        if rows.len() != 2
            || rows
                .iter()
                .any(|row| row.evaluation_id != item.evaluation_id)
            || !rows
                .iter()
                .any(|row| row.winner == item.system_low && row.loser == item.system_high)
            || !rows
                .iter()
                .any(|row| row.winner == item.system_high && row.loser == item.system_low)
        {
            return Err(Error::InvalidReduction(format!(
                "reduction directions do not match planned match {}",
                item.match_id
            )));
        }
    }

    let expected_conjunctive = conjunctive_table(
        &reduction.atomic_directed_verdicts,
        &bundle.spec.evaluations,
    )?;
    if reduction.evaluation_conjunctive_verdicts != expected_conjunctive {
        return Err(Error::InvalidReduction(
            "stored conjunctive verdicts do not match atomic verdicts".into(),
        ));
    }
    let system_ids: Vec<_> = bundle
        .registry
        .sorted_systems()
        .iter()
        .map(|system| system.system_id.clone())
        .collect();
    let expected_graph = graph_from_conjunctive(&system_ids, &expected_conjunctive)?;
    if reduction.system_graph != expected_graph {
        return Err(Error::InvalidReduction(
            "stored graph does not match conjunctive verdicts".into(),
        ));
    }
    let expected_per_evaluation = evaluation_maximal_sets(
        &system_ids,
        &reduction.atomic_directed_verdicts,
        &bundle.spec.evaluations,
    )?;
    if reduction.per_evaluation_maximal != expected_per_evaluation {
        return Err(Error::InvalidReduction(
            "stored per-evaluation maximal sets do not match atomic verdicts".into(),
        ));
    }
    // Validate against the reduction's own recorded strategy: it is a reduce-time
    // choice that may override the bundle default. Both selections are computed
    // identically regardless; only their primary/alternate roles depend on it.
    let (expected_selection, expected_alternate) = strategy_selections(
        bundle,
        reduction.selection_strategy,
        &expected_graph,
        &expected_conjunctive,
        &expected_per_evaluation,
        &system_ids,
    )?;
    if reduction.selection != expected_selection
        || reduction.alternate_selection != expected_alternate
    {
        return Err(Error::InvalidReduction(
            "stored selection does not match the audited graph".into(),
        ));
    }
    if reduction.descriptive_metrics.len() != plan.matches.len() {
        return Err(Error::InvalidReduction(
            "descriptive metric row count does not match the plan".into(),
        ));
    }
    let descriptive_ids: BTreeSet<_> = reduction
        .descriptive_metrics
        .iter()
        .map(|row| row.match_id.as_str())
        .collect();
    if descriptive_ids.len() != plan.matches.len()
        || plan
            .matches
            .iter()
            .any(|item| !descriptive_ids.contains(item.match_id.as_str()))
    {
        return Err(Error::InvalidReduction(
            "descriptive metric rows do not match planned matches".into(),
        ));
    }
    if reduction.score_vector_comparisons != compare_score_vectors(bundle) {
        return Err(Error::InvalidReduction(
            "stored score-vector comparisons do not match the bundle".into(),
        ));
    }
    Ok(())
}

fn read_json_file<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file = File::open(path).map_err(|error| crate::error::io(path, error))?;
    serde_json::from_reader(file).map_err(|source| Error::Json {
        path: path.to_owned(),
        source,
    })
}

fn read_csv_rows<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    let mut reader = csv::Reader::from_path(path).map_err(|source| Error::Csv {
        path: path.to_owned(),
        source,
    })?;
    let headers = reader
        .headers()
        .map_err(|source| Error::Csv {
            path: path.to_owned(),
            source,
        })?
        .clone();
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|source| Error::Csv {
            path: path.to_owned(),
            source,
        })?;
        let mut object = serde_json::Map::new();
        for (header, cell) in headers.iter().zip(record.iter()) {
            object.insert(header.to_owned(), csv_cell_value(cell));
        }
        rows.push(
            serde_json::from_value(serde_json::Value::Object(object)).map_err(|source| {
                Error::Json {
                    path: path.to_owned(),
                    source,
                }
            })?,
        );
    }
    Ok(rows)
}

fn csv_cell_value(cell: &str) -> serde_json::Value {
    if cell.is_empty() {
        return serde_json::Value::Null;
    }
    if cell.starts_with('{') || cell.starts_with('[') {
        if let Ok(value) = serde_json::from_str(cell) {
            return value;
        }
    }
    match cell {
        "true" => serde_json::Value::Bool(true),
        "false" => serde_json::Value::Bool(false),
        _ => serde_json::from_str::<serde_json::Number>(cell)
            .map(serde_json::Value::Number)
            .unwrap_or_else(|_| serde_json::Value::String(cell.to_owned())),
    }
}

fn audit_reduction_manifest(directory: &Path) -> Result<ReductionManifest> {
    let manifest_path = directory.join("reduction_manifest.json");
    let file =
        File::open(&manifest_path).map_err(|error| crate::error::io(&manifest_path, error))?;
    let manifest: ReductionManifest =
        serde_json::from_reader(file).map_err(|source| Error::Json {
            path: manifest_path,
            source,
        })?;
    for (name, declared_hash) in &manifest.files {
        let path = safe_reduction_path(directory, name)?;
        let actual = sha256_file(&path)?;
        if actual != *declared_hash {
            return Err(Error::InvalidReduction(format!(
                "reduction hash mismatch for {name}"
            )));
        }
    }
    Ok(manifest)
}

fn safe_reduction_path(directory: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Err(Error::InvalidReduction(format!(
            "unsafe reduction filename {name:?}"
        )));
    }
    Ok(directory.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conjunction_has_the_declared_three_state_truth_table() {
        assert_eq!(
            conjoin_verdicts([DirectedVerdict::Supported, DirectedVerdict::Supported]),
            DirectedVerdict::Supported
        );
        assert_eq!(
            conjoin_verdicts([DirectedVerdict::Supported, DirectedVerdict::Unresolved]),
            DirectedVerdict::Unresolved
        );
        assert_eq!(
            conjoin_verdicts([DirectedVerdict::Unresolved, DirectedVerdict::NotSupported]),
            DirectedVerdict::NotSupported
        );
        assert_eq!(
            conjoin_verdicts([DirectedVerdict::Supported, DirectedVerdict::NotSupported]),
            DirectedVerdict::NotSupported
        );
    }

    #[test]
    fn candidate_conservative_intersects_per_evaluation_maximal_sets() {
        let shared = vec![
            EvaluationMaximalSet {
                evaluation_id: "a".into(),
                maximal_systems: vec!["x".into(), "y".into()],
            },
            EvaluationMaximalSet {
                evaluation_id: "b".into(),
                maximal_systems: vec!["y".into(), "z".into()],
            },
        ];
        assert_eq!(
            candidate_conservative_survivors(&shared),
            vec!["y".to_owned()]
        );
        let disjoint = vec![
            EvaluationMaximalSet {
                evaluation_id: "a".into(),
                maximal_systems: vec!["x".into()],
            },
            EvaluationMaximalSet {
                evaluation_id: "b".into(),
                maximal_systems: vec!["z".into()],
            },
        ];
        assert!(candidate_conservative_survivors(&disjoint).is_empty());
    }

    #[test]
    fn rank_equivalence_preserves_ties_and_ignores_monotone_scale() {
        assert_eq!(
            rank_groups(&[3.0, 2.0, 2.0, 1.0]),
            rank_groups(&[9.0, 4.0, 4.0, -1.0])
        );
        assert_ne!(
            rank_groups(&[3.0, 2.0, 2.0, 1.0]),
            rank_groups(&[9.0, 4.0, 3.0, -1.0])
        );
    }
}
