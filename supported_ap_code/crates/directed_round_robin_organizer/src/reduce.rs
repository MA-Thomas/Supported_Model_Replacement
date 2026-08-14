use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::artifact::{read_artifact, result_path};
use crate::audit::{CompletenessAudit, audit_results};
use crate::graph::{DirectedEdge, GraphSummary, analyze_graph};
use crate::identity::sha256_file;
use crate::input::LoadedBundle;
use crate::plan::TournamentPlan;
use crate::spec::{DirectedVerdict, MetricKind};
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
    UniqueConjecturedSystem { system_id: String },
    UnresolvedCandidateSet { system_ids: Vec<String> },
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
    pub graph_maximal_systems: Vec<String>,
    pub survivor_evidence: Vec<SurvivorEvidence>,
    pub outcome: SelectionOutcome,
    pub operational_choice: Option<String>,
    pub operational_choice_is_non_evidential: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reduction {
    pub schema_version: u32,
    pub tournament_id: String,
    pub plan_id: String,
    pub bundle_content_hash: String,
    pub metric: MetricKind,
    pub completeness_audit: CompletenessAudit,
    pub atomic_directed_verdicts: Vec<AtomicDirectedVerdict>,
    pub evaluation_conjunctive_verdicts: Vec<ConjunctiveDirectedVerdict>,
    pub system_graph: GraphSummary,
    pub selection: SelectionResult,
    pub descriptive_metrics: Vec<DescriptiveMetricRow>,
    pub score_vector_comparisons: Vec<ScoreVectorComparison>,
}

pub fn reduce_tournament(
    bundle: &LoadedBundle,
    plan: &TournamentPlan,
    results_root: &Path,
) -> Result<Reduction> {
    let completeness_audit = audit_results(bundle, plan, results_root)?;
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
    for item in &plan.matches {
        let artifact = read_artifact(&result_path(results_root, &item.match_id))?;
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
    let selection = select_system(bundle, &system_graph, &conjunctive)?;
    let score_vector_comparisons = compare_score_vectors(bundle);
    Ok(Reduction {
        schema_version: 1,
        tournament_id: plan.tournament_id.clone(),
        plan_id: plan.plan_id.clone(),
        bundle_content_hash: plan.bundle_content_hash.clone(),
        metric: plan.metric,
        completeness_audit,
        atomic_directed_verdicts: atomic,
        evaluation_conjunctive_verdicts: conjunctive,
        system_graph,
        selection,
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

fn conjunctive_table(
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

fn graph_from_conjunctive(
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

fn select_system(
    bundle: &LoadedBundle,
    graph: &GraphSummary,
    conjunctive: &[ConjunctiveDirectedVerdict],
) -> Result<SelectionResult> {
    let survivors = graph.maximal_vertices.clone();
    if survivors.is_empty() {
        return Err(Error::InvalidReduction(
            "selection ladder produced no system".into(),
        ));
    }
    let outcome = if survivors.len() == 1 {
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
    if bundle.spec.operational_tie_break.is_some() && operational_choice.is_none() {
        return Err(Error::InvalidReduction(
            "operational tie-break contains no surviving system".into(),
        ));
    }
    let verdict_lookup: BTreeMap<_, _> = conjunctive
        .iter()
        .map(|row| ((row.winner.as_str(), row.loser.as_str()), row.verdict))
        .collect();
    let all_system_ids: Vec<_> = bundle
        .registry
        .sorted_systems()
        .into_iter()
        .map(|system| system.system_id.as_str())
        .collect();
    let mut survivor_evidence = Vec::new();
    for survivor in &survivors {
        let mut comparisons = Vec::new();
        for opponent in &all_system_ids {
            if survivor == opponent {
                continue;
            }
            let survivor_over_opponent = verdict_lookup
                .get(&(survivor.as_str(), *opponent))
                .copied()
                .ok_or_else(|| Error::InvalidReduction("missing survivor comparison".into()))?;
            let opponent_over_survivor = verdict_lookup
                .get(&(*opponent, survivor.as_str()))
                .copied()
                .ok_or_else(|| {
                    Error::InvalidReduction("missing reverse survivor comparison".into())
                })?;
            comparisons.push(SurvivorComparison {
                opponent_system_id: (*opponent).to_owned(),
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
        graph_maximal_systems: graph.maximal_vertices.clone(),
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
    files: BTreeMap<String, String>,
}

pub fn write_reduction(reduction: &Reduction, output: &Path) -> Result<()> {
    fs::create_dir_all(output).map_err(|error| crate::error::io(output, error))?;
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
