use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::artifact::{create_staging_directory, publish_staging_directory};
use crate::audit::CompletenessAudit;
use crate::graph::{DirectedEdge, GraphSummary};
use crate::identity::{hash_serializable, sha256_file};
use crate::input::LoadedBundle;
use crate::plan::TournamentPlan;
use crate::reduce::{
    AtomicDirectedVerdict, ConjunctiveDirectedVerdict, DescriptiveMetricRow, EvaluationMaximalSet,
    Reduction, ScoreVectorComparison, SelectionResult, conjunctive_table, evaluation_maximal_sets,
    graph_from_conjunctive, reduce_tournament, strategy_selections,
};
use crate::spec::{MetricKind, SelectionStrategy, validate_id};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictSource {
    Base,
    Revision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcedAtomicDirectedVerdict {
    pub match_id: String,
    pub evaluation_id: String,
    pub winner: String,
    pub loser: String,
    pub verdict: crate::spec::DirectedVerdict,
    pub official_verdict: String,
    pub source: VerdictSource,
    pub source_bundle_content_hash: String,
    pub source_plan_id: String,
}

impl SourcedAtomicDirectedVerdict {
    fn unsourced(&self) -> AtomicDirectedVerdict {
        AtomicDirectedVerdict {
            match_id: self.match_id.clone(),
            evaluation_id: self.evaluation_id.clone(),
            winner: self.winner.clone(),
            loser: self.loser.clone(),
            verdict: self.verdict,
            official_verdict: self.official_verdict.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphDelta {
    pub retained_edges: Vec<DirectedEdge>,
    pub gained_edges: Vec<DirectedEdge>,
    pub lost_edges: Vec<DirectedEdge>,
    pub retained_maximal_systems: Vec<String>,
    pub gained_maximal_systems: Vec<String>,
    pub lost_maximal_systems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevisionComposition {
    pub schema_version: u32,
    pub composition_id: String,
    pub revision_id: String,
    pub replaced_evaluation: String,
    pub metric: MetricKind,
    pub selection_strategy: SelectionStrategy,
    pub base_tournament_id: String,
    pub base_plan_id: String,
    pub base_bundle_content_hash: String,
    pub revision_tournament_id: String,
    pub revision_plan_id: String,
    pub revision_bundle_content_hash: String,
    pub base_completeness_audit: CompletenessAudit,
    pub revision_completeness_audit: CompletenessAudit,
    pub atomic_directed_verdicts: Vec<SourcedAtomicDirectedVerdict>,
    pub evaluation_conjunctive_verdicts: Vec<ConjunctiveDirectedVerdict>,
    pub system_graph: GraphSummary,
    pub selection: SelectionResult,
    pub alternate_selection: SelectionResult,
    pub per_evaluation_maximal: Vec<EvaluationMaximalSet>,
    pub graph_delta: GraphDelta,
    pub descriptive_metrics: Vec<DescriptiveMetricRow>,
    pub score_vector_comparisons: Vec<ScoreVectorComparison>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RevisionCompositionManifest {
    schema_version: u32,
    composition_id: String,
    revision_id: String,
    replaced_evaluation: String,
    metric: MetricKind,
    base_tournament_id: String,
    base_plan_id: String,
    base_bundle_content_hash: String,
    revision_tournament_id: String,
    revision_plan_id: String,
    revision_bundle_content_hash: String,
    files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy)]
pub struct TournamentSource<'a> {
    pub bundle: &'a LoadedBundle,
    pub plan: &'a TournamentPlan,
    pub results: &'a Path,
}

#[derive(Debug, Clone, Copy)]
pub struct TournamentReductionSource<'a> {
    pub bundle: &'a LoadedBundle,
    pub plan: &'a TournamentPlan,
    pub reduction: &'a Reduction,
}

pub fn compose_revision(
    base: TournamentSource<'_>,
    revision: TournamentSource<'_>,
    replaced_evaluation: &str,
    revision_id: &str,
) -> Result<RevisionComposition> {
    validate_id("revision_id", revision_id)?;
    validate_id("replaced_evaluation", replaced_evaluation)?;
    validate_compatibility(
        base.bundle,
        base.plan,
        revision.bundle,
        revision.plan,
        replaced_evaluation,
    )?;
    let base_reduction = reduce_tournament(base.bundle, base.plan, base.results)?;
    let revision_reduction = reduce_tournament(revision.bundle, revision.plan, revision.results)?;
    compose_reductions(
        base.bundle,
        base.plan,
        base_reduction,
        revision.plan,
        revision_reduction,
        replaced_evaluation,
        revision_id,
    )
}

/// Composes a context revision from compact, independently audited
/// reductions. This is scientifically equivalent to `compose_revision` but
/// avoids re-reading every raw match report during finalization.
pub fn compose_revision_from_reductions(
    base: TournamentReductionSource<'_>,
    revision: TournamentReductionSource<'_>,
    replaced_evaluation: &str,
    revision_id: &str,
) -> Result<RevisionComposition> {
    validate_id("revision_id", revision_id)?;
    validate_id("replaced_evaluation", replaced_evaluation)?;
    validate_compatibility(
        base.bundle,
        base.plan,
        revision.bundle,
        revision.plan,
        replaced_evaluation,
    )?;
    compose_reductions(
        base.bundle,
        base.plan,
        base.reduction.clone(),
        revision.plan,
        revision.reduction.clone(),
        replaced_evaluation,
        revision_id,
    )
}

fn validate_compatibility(
    base_bundle: &LoadedBundle,
    base_plan: &TournamentPlan,
    revision_bundle: &LoadedBundle,
    revision_plan: &TournamentPlan,
    replaced_evaluation: &str,
) -> Result<()> {
    if !base_bundle.evaluations.contains_key(replaced_evaluation) {
        return Err(Error::InvalidReduction(format!(
            "base bundle does not contain replacement evaluation {replaced_evaluation}"
        )));
    }
    if revision_bundle.spec.evaluations != [replaced_evaluation] {
        return Err(Error::InvalidReduction(format!(
            "revision bundle must contain exactly evaluation {replaced_evaluation}"
        )));
    }
    if base_bundle.registry != revision_bundle.registry {
        return Err(Error::InvalidReduction(
            "base and revision system registries differ".into(),
        ));
    }
    let mut base_policy = base_bundle.spec.clone();
    let mut revision_policy = revision_bundle.spec.clone();
    base_policy.evaluations.clear();
    revision_policy.evaluations.clear();
    base_policy.annotations.clear();
    revision_policy.annotations.clear();
    if base_policy != revision_policy {
        return Err(Error::InvalidReduction(
            "base and revision tournament policies differ".into(),
        ));
    }
    if base_plan.metric != revision_plan.metric || base_plan.metric != base_bundle.spec.metric {
        return Err(Error::InvalidReduction(
            "base and revision metrics differ".into(),
        ));
    }
    let base_seeds: BTreeMap<_, _> = base_plan
        .matches
        .iter()
        .filter(|item| item.evaluation_id == replaced_evaluation)
        .map(|item| {
            (
                (item.system_low.as_str(), item.system_high.as_str()),
                item.seed,
            )
        })
        .collect();
    let revision_seeds: BTreeMap<_, _> = revision_plan
        .matches
        .iter()
        .map(|item| {
            (
                (item.system_low.as_str(), item.system_high.as_str()),
                item.seed,
            )
        })
        .collect();
    if base_seeds != revision_seeds {
        return Err(Error::InvalidReduction(
            "replacement evaluation does not preserve pairwise resampling seeds".into(),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn compose_reductions(
    base_bundle: &LoadedBundle,
    base_plan: &TournamentPlan,
    base: Reduction,
    revision_plan: &TournamentPlan,
    revision: Reduction,
    replaced_evaluation: &str,
    revision_id: &str,
) -> Result<RevisionComposition> {
    let mut sourced_atomic = Vec::with_capacity(base.atomic_directed_verdicts.len());
    sourced_atomic.extend(
        base.atomic_directed_verdicts
            .iter()
            .filter(|row| row.evaluation_id != replaced_evaluation)
            .map(|row| sourced(row, VerdictSource::Base, base_plan)),
    );
    for row in &revision.atomic_directed_verdicts {
        if row.evaluation_id != replaced_evaluation {
            return Err(Error::InvalidReduction(format!(
                "revision reduction contains unexpected evaluation {}",
                row.evaluation_id
            )));
        }
        sourced_atomic.push(sourced(row, VerdictSource::Revision, revision_plan));
    }
    sourced_atomic.sort_by(|left, right| {
        (&left.evaluation_id, &left.winner, &left.loser).cmp(&(
            &right.evaluation_id,
            &right.winner,
            &right.loser,
        ))
    });
    let atomic: Vec<_> = sourced_atomic
        .iter()
        .map(SourcedAtomicDirectedVerdict::unsourced)
        .collect();
    let conjunctive = conjunctive_table(&atomic, &base_bundle.spec.evaluations)?;
    let system_ids: Vec<_> = base_bundle
        .registry
        .sorted_systems()
        .iter()
        .map(|system| system.system_id.clone())
        .collect();
    let system_graph = graph_from_conjunctive(&system_ids, &conjunctive)?;
    let per_evaluation_maximal =
        evaluation_maximal_sets(&system_ids, &atomic, &base_bundle.spec.evaluations)?;
    let (selection, alternate_selection) = strategy_selections(
        base_bundle,
        base_bundle.spec.selection_strategy,
        &system_graph,
        &conjunctive,
        &per_evaluation_maximal,
        &system_ids,
    )?;
    let graph_delta = graph_delta(
        &base.selection.graph_maximal_systems,
        &selection.graph_maximal_systems,
        &base.system_graph,
        &system_graph,
    );

    let mut descriptive_metrics: Vec<_> = base
        .descriptive_metrics
        .into_iter()
        .filter(|row| row.evaluation_id != replaced_evaluation)
        .chain(revision.descriptive_metrics)
        .collect();
    descriptive_metrics.sort_by(|left, right| {
        (&left.evaluation_id, &left.system_low, &left.system_high).cmp(&(
            &right.evaluation_id,
            &right.system_low,
            &right.system_high,
        ))
    });
    let mut score_vector_comparisons: Vec<_> = base
        .score_vector_comparisons
        .into_iter()
        .filter(|row| row.evaluation_id != replaced_evaluation)
        .chain(revision.score_vector_comparisons)
        .collect();
    score_vector_comparisons.sort_by(|left, right| {
        (&left.evaluation_id, &left.system_a, &left.system_b).cmp(&(
            &right.evaluation_id,
            &right.system_a,
            &right.system_b,
        ))
    });
    let composition_id = hash_serializable(&(
        "directed-round-robin-revision-composition-v1",
        revision_id,
        replaced_evaluation,
        &base_plan.plan_id,
        &revision_plan.plan_id,
        &sourced_atomic,
    ))?;
    Ok(RevisionComposition {
        schema_version: 1,
        composition_id,
        revision_id: revision_id.to_owned(),
        replaced_evaluation: replaced_evaluation.to_owned(),
        metric: base_plan.metric,
        selection_strategy: base_bundle.spec.selection_strategy,
        base_tournament_id: base_plan.tournament_id.clone(),
        base_plan_id: base_plan.plan_id.clone(),
        base_bundle_content_hash: base_plan.bundle_content_hash.clone(),
        revision_tournament_id: revision_plan.tournament_id.clone(),
        revision_plan_id: revision_plan.plan_id.clone(),
        revision_bundle_content_hash: revision_plan.bundle_content_hash.clone(),
        base_completeness_audit: base.completeness_audit,
        revision_completeness_audit: revision.completeness_audit,
        atomic_directed_verdicts: sourced_atomic,
        evaluation_conjunctive_verdicts: conjunctive,
        system_graph,
        selection,
        alternate_selection,
        per_evaluation_maximal,
        graph_delta,
        descriptive_metrics,
        score_vector_comparisons,
    })
}

fn sourced(
    row: &AtomicDirectedVerdict,
    source: VerdictSource,
    plan: &TournamentPlan,
) -> SourcedAtomicDirectedVerdict {
    SourcedAtomicDirectedVerdict {
        match_id: row.match_id.clone(),
        evaluation_id: row.evaluation_id.clone(),
        winner: row.winner.clone(),
        loser: row.loser.clone(),
        verdict: row.verdict,
        official_verdict: row.official_verdict.clone(),
        source,
        source_bundle_content_hash: plan.bundle_content_hash.clone(),
        source_plan_id: plan.plan_id.clone(),
    }
}

fn graph_delta(
    base_maximal_systems: &[String],
    revised_maximal_systems: &[String],
    base: &GraphSummary,
    revised: &GraphSummary,
) -> GraphDelta {
    let base_edges: BTreeSet<_> = base.edges.iter().cloned().collect();
    let revised_edges: BTreeSet<_> = revised.edges.iter().cloned().collect();
    let base_maximal: BTreeSet<_> = base_maximal_systems.iter().cloned().collect();
    let revised_maximal: BTreeSet<_> = revised_maximal_systems.iter().cloned().collect();
    GraphDelta {
        retained_edges: base_edges.intersection(&revised_edges).cloned().collect(),
        gained_edges: revised_edges.difference(&base_edges).cloned().collect(),
        lost_edges: base_edges.difference(&revised_edges).cloned().collect(),
        retained_maximal_systems: base_maximal
            .intersection(&revised_maximal)
            .cloned()
            .collect(),
        gained_maximal_systems: revised_maximal.difference(&base_maximal).cloned().collect(),
        lost_maximal_systems: base_maximal.difference(&revised_maximal).cloned().collect(),
    }
}

pub fn write_revision_composition(composition: &RevisionComposition, output: &Path) -> Result<()> {
    let staging = create_staging_directory(output)?;
    write_revision_composition_into(composition, &staging)?;
    publish_staging_directory(&staging, output)
}

fn write_revision_composition_into(composition: &RevisionComposition, output: &Path) -> Result<()> {
    let mut files = BTreeMap::new();
    write_json_file(output, "composition.json", composition, &mut files)?;
    write_json_file(
        output,
        "base_completeness_audit.json",
        &composition.base_completeness_audit,
        &mut files,
    )?;
    write_json_file(
        output,
        "revision_completeness_audit.json",
        &composition.revision_completeness_audit,
        &mut files,
    )?;
    write_csv_file(
        output,
        "atomic_directed_verdicts.csv",
        &composition.atomic_directed_verdicts,
        &mut files,
    )?;
    write_csv_file(
        output,
        "evaluation_conjunctive_verdicts.csv",
        &composition.evaluation_conjunctive_verdicts,
        &mut files,
    )?;
    write_json_file(
        output,
        "system_graph.json",
        &composition.system_graph,
        &mut files,
    )?;
    write_json_file(output, "selection.json", &composition.selection, &mut files)?;
    write_json_file(
        output,
        "alternate_selection.json",
        &composition.alternate_selection,
        &mut files,
    )?;
    write_json_file(
        output,
        "per_evaluation_maximal.json",
        &composition.per_evaluation_maximal,
        &mut files,
    )?;
    write_json_file(
        output,
        "graph_delta.json",
        &composition.graph_delta,
        &mut files,
    )?;
    write_csv_file(
        output,
        "descriptive_metrics.csv",
        &composition.descriptive_metrics,
        &mut files,
    )?;
    write_csv_file(
        output,
        "score_vector_comparisons.csv",
        &composition.score_vector_comparisons,
        &mut files,
    )?;
    let manifest = RevisionCompositionManifest {
        schema_version: 1,
        composition_id: composition.composition_id.clone(),
        revision_id: composition.revision_id.clone(),
        replaced_evaluation: composition.replaced_evaluation.clone(),
        metric: composition.metric,
        base_tournament_id: composition.base_tournament_id.clone(),
        base_plan_id: composition.base_plan_id.clone(),
        base_bundle_content_hash: composition.base_bundle_content_hash.clone(),
        revision_tournament_id: composition.revision_tournament_id.clone(),
        revision_plan_id: composition.revision_plan_id.clone(),
        revision_bundle_content_hash: composition.revision_bundle_content_hash.clone(),
        files,
    };
    write_json_new(
        &output.join("revision_composition_manifest.json"),
        &manifest,
    )
}

pub fn audit_revision_composition_for(
    directory: &Path,
    base_plan: &TournamentPlan,
    revision_plan: &TournamentPlan,
    replaced_evaluation: &str,
    revision_id: &str,
) -> Result<()> {
    let manifest: RevisionCompositionManifest =
        read_json(&directory.join("revision_composition_manifest.json"))?;
    if manifest.schema_version != 1
        || manifest.revision_id != revision_id
        || manifest.replaced_evaluation != replaced_evaluation
        || manifest.metric != base_plan.metric
        || manifest.base_tournament_id != base_plan.tournament_id
        || manifest.base_plan_id != base_plan.plan_id
        || manifest.base_bundle_content_hash != base_plan.bundle_content_hash
        || manifest.revision_tournament_id != revision_plan.tournament_id
        || manifest.revision_plan_id != revision_plan.plan_id
        || manifest.revision_bundle_content_hash != revision_plan.bundle_content_hash
    {
        return Err(Error::InvalidReduction(
            "revision composition manifest does not match its declared sources".into(),
        ));
    }
    for (name, declared_hash) in manifest.files {
        let path = safe_path(directory, &name)?;
        if sha256_file(&path)? != declared_hash {
            return Err(Error::InvalidReduction(format!(
                "revision composition hash mismatch for {name}"
            )));
        }
    }
    let composition: RevisionComposition = read_json(&directory.join("composition.json"))?;
    let expected_composition_id = hash_serializable(&(
        "directed-round-robin-revision-composition-v1",
        composition.revision_id.as_str(),
        composition.replaced_evaluation.as_str(),
        composition.base_plan_id.as_str(),
        composition.revision_plan_id.as_str(),
        &composition.atomic_directed_verdicts,
    ))?;
    if composition.composition_id != manifest.composition_id
        || composition.composition_id != expected_composition_id
        || composition.revision_id != manifest.revision_id
        || composition.replaced_evaluation != manifest.replaced_evaluation
        || composition.metric != manifest.metric
        || composition.base_tournament_id != manifest.base_tournament_id
        || composition.base_plan_id != manifest.base_plan_id
        || composition.base_bundle_content_hash != manifest.base_bundle_content_hash
        || composition.revision_tournament_id != manifest.revision_tournament_id
        || composition.revision_plan_id != manifest.revision_plan_id
        || composition.revision_bundle_content_hash != manifest.revision_bundle_content_hash
        || !composition.base_completeness_audit.complete
        || !composition.revision_completeness_audit.complete
    {
        return Err(Error::InvalidReduction(
            "revision composition content does not match its manifest or audited sources".into(),
        ));
    }
    Ok(())
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
    if let Some(first_row) = json_rows.first() {
        let headers: Vec<String> = first_row
            .as_object()
            .ok_or_else(|| {
                Error::InvalidReduction(format!("{name} rows must serialize as objects"))
            })?
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

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file = File::open(path).map_err(|error| crate::error::io(path, error))?;
    serde_json::from_reader(file).map_err(|source| Error::Json {
        path: path.to_owned(),
        source,
    })
}

fn safe_path(directory: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Err(Error::InvalidReduction(format!(
            "unsafe revision-composition filename {name:?}"
        )));
    }
    Ok(directory.join(name))
}
