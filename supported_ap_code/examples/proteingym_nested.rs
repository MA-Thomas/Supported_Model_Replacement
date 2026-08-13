use std::collections::BTreeMap;
use std::fs::File;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use serde::Serialize;
use supported_ap::{
    AnchoredEffectRow, ComputationalReplicationCount, DirectionalFiniteEvidence,
    DirectionalNestedFiniteEvidence, Execution, FiniteEvidenceVerdict, MagnitudeThreshold,
    NestedRetainedEffectRow, ObservedApAssessment, PairedEvaluation, Prevalence,
    ReferenceAssessment, ReplacementPolicy, RetainedEffect, ScoreTransportAssumption,
    SearchOptions, StagedDirectionalNestedAp, StagedNestedApResult, SupportOrder, SurvivalFloor,
    SurvivalRequirement, TargetPrevalences, anchored_nested_support, assess_observed_ap,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const CLUSTER_BOOTSTRAP_DOMAIN: u64 = 0x5052_4f54_434c_5354;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ExecutionArg {
    Sequential,
    Parallel,
}

impl From<ExecutionArg> for Execution {
    fn from(value: ExecutionArg) -> Self {
        match value {
            ExecutionArg::Sequential => Self::Sequential,
            ExecutionArg::Parallel => Self::Parallel,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "proteingym_nested",
    about = "Conditional ProteinGym full-nesting illustration with a residue-position bootstrap"
)]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value = "assay_id")]
    evaluation_id_col: String,
    #[arg(long, default_value = "position_id")]
    cluster_id_col: String,
    #[arg(long, default_value = "label")]
    label_col: String,
    #[arg(long)]
    model_a_col: String,
    #[arg(long)]
    model_b_col: String,
    #[arg(long)]
    interval_lower: f64,
    #[arg(long)]
    interval_upper: f64,
    #[arg(long, default_value_t = 257)]
    search_grid_points: usize,
    #[arg(long, default_value_t = 1e-8)]
    search_tolerance: f64,
    #[arg(long, default_value_t = 128)]
    search_max_iterations: usize,
    #[arg(long)]
    empirical_order: usize,
    #[arg(long)]
    computational_order: usize,
    #[arg(long)]
    computational_replications: usize,
    #[arg(long)]
    resampling_seed: u64,
    #[arg(long, default_value_t = 10_000)]
    maximum_redraws: usize,
    #[arg(long)]
    magnitude_threshold: f64,
    #[arg(long)]
    survival_floor: f64,
    #[arg(long)]
    survival_requirement: f64,
    #[arg(long)]
    transport_justification: String,
    #[arg(long)]
    reference_limitation: String,
    #[arg(long, value_enum, default_value_t = ExecutionArg::Parallel)]
    execution: ExecutionArg,
}

#[derive(Default)]
struct RowBuilder {
    labels: Vec<bool>,
    scores_a: Vec<f64>,
    scores_b: Vec<f64>,
    cluster_ids: Vec<String>,
}

struct RowData {
    id: String,
    labels: Vec<bool>,
    evaluation: PairedEvaluation,
    clusters: Vec<Vec<usize>>,
}

#[derive(Debug, Serialize)]
struct RowDesign {
    evaluation_id: String,
    observation_count: usize,
    position_cluster_count: usize,
    minimum_cluster_size: usize,
    maximum_cluster_size: usize,
}

#[derive(Debug, Serialize)]
struct RowComputationDiagnostic {
    evaluation_id: String,
    replications: usize,
    rejected_missing_class_draws: usize,
}

#[derive(Debug, Serialize)]
struct ClusterResamplingDescription {
    unit: &'static str,
    draws_per_replication: &'static str,
    cluster_contents: &'static str,
    class_requirement: &'static str,
    pairing: &'static str,
    maximum_redraws: usize,
}

#[derive(Debug, Serialize)]
struct ConditionalNestedAssessment {
    target_prevalences: TargetPrevalences,
    transport: ScoreTransportAssumption,
    search: SearchOptions,
    empirical_order: SupportOrder,
    computational_order: SupportOrder,
    computational_replications_per_row: ComputationalReplicationCount,
    resampling_seed: u64,
    resampling: ClusterResamplingDescription,
    execution: Execution,
    reference_assessment: ReferenceAssessment,
    row_designs: Vec<RowDesign>,
}

#[derive(Debug, Serialize)]
struct Report {
    schema_version: u32,
    manuscript_version: &'static str,
    analysis: &'static str,
    conditional_illustration: bool,
    assessment: ConditionalNestedAssessment,
    evaluation_ids: Vec<String>,
    computational_diagnostics: Vec<RowComputationDiagnostic>,
    result: StagedNestedApResult,
}

struct ComputationalRow {
    forward: Vec<RetainedEffect>,
    reverse: Vec<RetainedEffect>,
    rejected_draws: usize,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    if args.maximum_redraws == 0 {
        return Err("--maximum-redraws must be positive".into());
    }
    let rows = read_rows(&args)?;
    let evaluation_ids = rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>();
    let evaluations = rows
        .iter()
        .map(|row| row.evaluation.clone())
        .collect::<Vec<_>>();
    let target_prevalences = TargetPrevalences::closed_interval(
        Prevalence::new(args.interval_lower)?,
        Prevalence::new(args.interval_upper)?,
    )?;
    let search = SearchOptions::new(
        args.search_grid_points,
        args.search_tolerance,
        args.search_max_iterations,
    )?;
    let empirical_order = SupportOrder::new(args.empirical_order)?;
    let computational_order = SupportOrder::new(args.computational_order)?;
    let computational_replications =
        ComputationalReplicationCount::new(args.computational_replications)?;
    if computational_order.get() > computational_replications.get() {
        return Err("computational order exceeds the replication count".into());
    }
    let transport = ScoreTransportAssumption::new(&args.transport_justification)?;
    let reference_assessment = ReferenceAssessment::not_asserted(&args.reference_limitation)?;
    let policy = ReplacementPolicy::new(
        MagnitudeThreshold::new(args.magnitude_threshold)?,
        SurvivalFloor::new(args.survival_floor)?,
        SurvivalRequirement::new(args.survival_requirement)?,
    );
    let execution: Execution = args.execution.into();
    let observed_assessment = ObservedApAssessment {
        target_prevalences: target_prevalences.clone(),
        transport: transport.clone(),
        search,
        empirical_order,
        execution,
        reference_assessment: reference_assessment.clone(),
    };
    let observed = assess_observed_ap(&evaluations, &observed_assessment, policy)?;
    let forward_passes = gate_passes(&observed.forward, policy);
    let reverse_passes = gate_passes(&observed.reverse, policy);

    let computational = if forward_passes || reverse_passes {
        compute_rows(
            &rows,
            computational_replications,
            args.resampling_seed,
            args.maximum_redraws,
            &target_prevalences,
            search,
            execution,
        )?
    } else {
        Vec::new()
    };

    let forward_rows =
        build_retained_rows(&observed.forward.retained_effects, &computational, true);
    let reverse_rows =
        build_retained_rows(&observed.reverse.retained_effects, &computational, false);
    let forward = staged_direction(
        observed.forward.clone(),
        forward_rows,
        empirical_order,
        computational_order,
        policy,
    )?;
    let reverse = staged_direction(
        observed.reverse.clone(),
        reverse_rows,
        empirical_order,
        computational_order,
        policy,
    )?;
    let result = StagedNestedApResult {
        policy,
        empirical_order,
        computational_order,
        evaluation_count: rows.len(),
        evaluations: observed.evaluations,
        forward,
        reverse,
        reference_assessment: reference_assessment.clone(),
    };
    let row_designs = rows.iter().map(row_design).collect();
    let computational_diagnostics = rows
        .iter()
        .enumerate()
        .map(|(index, row)| RowComputationDiagnostic {
            evaluation_id: row.id.clone(),
            replications: if computational.is_empty() {
                0
            } else {
                computational_replications.get()
            },
            rejected_missing_class_draws: computational
                .get(index)
                .map_or(0, |computed| computed.rejected_draws),
        })
        .collect();
    let report = Report {
        schema_version: 17,
        manuscript_version: "v17",
        analysis: "ap_conditional_clustered_nested_illustration",
        conditional_illustration: true,
        assessment: ConditionalNestedAssessment {
            target_prevalences,
            transport,
            search,
            empirical_order,
            computational_order,
            computational_replications_per_row: computational_replications,
            resampling_seed: args.resampling_seed,
            resampling: ClusterResamplingDescription {
                unit: "residue_position_within_assay",
                draws_per_replication: "observed_number_of_distinct_positions",
                cluster_contents: "all_retained_substitutions_at_the_selected_position",
                class_requirement: "redraw_the_complete_position_bootstrap_until_both_outcome_classes_are_present",
                pairing: "the_same_cluster_multiplicities_are_used_for_both_models_and_every_target_prevalence",
                maximum_redraws: args.maximum_redraws,
            },
            execution,
            reference_assessment,
            row_designs,
        },
        evaluation_ids,
        computational_diagnostics,
        result,
    };
    serde_json::to_writer_pretty(File::create(&args.output)?, &report)?;
    Ok(())
}

fn read_rows(args: &Args) -> Result<Vec<RowData>> {
    let mut reader = csv::Reader::from_path(&args.input)?;
    let headers = reader.headers()?.clone();
    let find = |column: &str| {
        headers
            .iter()
            .position(|header| header == column)
            .ok_or_else(|| format!("column not found: {column}"))
    };
    let evaluation_index = find(&args.evaluation_id_col)?;
    let cluster_index = find(&args.cluster_id_col)?;
    let label_index = find(&args.label_col)?;
    let model_a_index = find(&args.model_a_col)?;
    let model_b_index = find(&args.model_b_col)?;
    let mut grouped: BTreeMap<String, RowBuilder> = BTreeMap::new();
    for (record_index, record) in reader.records().enumerate() {
        let record = record?;
        let row_number = record_index + 2;
        let group = grouped
            .entry(record[evaluation_index].to_owned())
            .or_default();
        group
            .labels
            .push(parse_label(&record[label_index], row_number)?);
        group.scores_a.push(parse_score(
            &record[model_a_index],
            row_number,
            &args.model_a_col,
        )?);
        group.scores_b.push(parse_score(
            &record[model_b_index],
            row_number,
            &args.model_b_col,
        )?);
        let cluster = record[cluster_index].trim();
        if cluster.is_empty() {
            return Err(format!("empty cluster identifier at CSV row {row_number}").into());
        }
        group.cluster_ids.push(cluster.to_owned());
    }
    if grouped.is_empty() {
        return Err("at least one empirical evaluation is required".into());
    }
    grouped
        .into_iter()
        .map(|(id, group)| {
            let mut cluster_map: BTreeMap<String, Vec<usize>> = BTreeMap::new();
            for (index, cluster_id) in group.cluster_ids.iter().enumerate() {
                cluster_map
                    .entry(cluster_id.clone())
                    .or_default()
                    .push(index);
            }
            let clusters = cluster_map.into_values().collect::<Vec<_>>();
            if clusters.len() < 2 {
                return Err(format!("evaluation {id} has fewer than two position clusters").into());
            }
            let evaluation =
                PairedEvaluation::new(&group.scores_a, &group.scores_b, &group.labels)?;
            Ok(RowData {
                id,
                labels: group.labels,
                evaluation,
                clusters,
            })
        })
        .collect()
}

fn compute_rows(
    rows: &[RowData],
    replications: ComputationalReplicationCount,
    master_seed: u64,
    maximum_redraws: usize,
    target_prevalences: &TargetPrevalences,
    search: SearchOptions,
    execution: Execution,
) -> Result<Vec<ComputationalRow>> {
    let compute = |(row_index, row): (usize, &RowData)| -> Result<ComputationalRow> {
        let mut forward = Vec::with_capacity(replications.get());
        let mut reverse = Vec::with_capacity(replications.get());
        let mut rejected_draws = 0usize;
        for replicate in 0..replications.get() {
            let (multiplicities, rejected) =
                clustered_multiplicities(row, master_seed, row_index, replicate, maximum_redraws)?;
            rejected_draws += rejected;
            let (forward_effect, reverse_effect) = row
                .evaluation
                .retained_effects_for_multiplicities(&multiplicities, target_prevalences, search)?;
            forward.push(forward_effect);
            reverse.push(reverse_effect);
        }
        Ok(ComputationalRow {
            forward,
            reverse,
            rejected_draws,
        })
    };
    match execution {
        Execution::Sequential => rows.iter().enumerate().map(compute).collect(),
        Execution::Parallel => rows
            .par_iter()
            .enumerate()
            .map(compute)
            .collect::<std::result::Result<Vec<_>, _>>(),
    }
}

fn clustered_multiplicities(
    row: &RowData,
    master_seed: u64,
    row_index: usize,
    replicate: usize,
    maximum_redraws: usize,
) -> Result<(Vec<usize>, usize)> {
    let seed = mix_seed(master_seed, &[row_index, replicate]);
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut multiplicities = vec![0usize; row.labels.len()];
    for rejected in 0..maximum_redraws {
        multiplicities.fill(0);
        for _ in 0..row.clusters.len() {
            let cluster = &row.clusters[rng.gen_range(0..row.clusters.len())];
            for &observation in cluster {
                multiplicities[observation] += 1;
            }
        }
        let positive = row
            .labels
            .iter()
            .zip(&multiplicities)
            .any(|(&label, &count)| label && count > 0);
        let negative = row
            .labels
            .iter()
            .zip(&multiplicities)
            .any(|(&label, &count)| !label && count > 0);
        if positive && negative {
            return Ok((multiplicities, rejected));
        }
    }
    Err(format!(
        "evaluation {} did not produce both classes within {} complete cluster-bootstrap attempts",
        row.id, maximum_redraws
    )
    .into())
}

fn build_retained_rows(
    observed: &[RetainedEffect],
    computational: &[ComputationalRow],
    forward: bool,
) -> Vec<NestedRetainedEffectRow> {
    if computational.is_empty() {
        return Vec::new();
    }
    observed
        .iter()
        .copied()
        .zip(computational)
        .map(|(anchor, row)| NestedRetainedEffectRow {
            observed: anchor,
            computational: if forward {
                row.forward.clone()
            } else {
                row.reverse.clone()
            },
        })
        .collect()
}

fn staged_direction(
    observed_gate: DirectionalFiniteEvidence,
    rows: Vec<NestedRetainedEffectRow>,
    empirical_order: SupportOrder,
    computational_order: SupportOrder,
    policy: ReplacementPolicy,
) -> Result<StagedDirectionalNestedAp> {
    if !gate_passes(&observed_gate, policy) {
        return Ok(StagedDirectionalNestedAp {
            observed_gate,
            full_assessment: None,
            staged_verdict: FiniteEvidenceVerdict::NoVerdict,
        });
    }
    let effects = rows
        .iter()
        .map(|row| AnchoredEffectRow {
            observed_effect: row.observed.value,
            computational_effects: row
                .computational
                .iter()
                .map(|effect| effect.value)
                .collect(),
        })
        .collect::<Vec<_>>();
    let nested = anchored_nested_support(
        &effects,
        empirical_order,
        computational_order,
        policy.survival_floor,
    )?;
    let verdict = policy_verdict(
        policy,
        nested.supported_magnitude,
        nested.literal_survival.subset_fraction,
    );
    Ok(StagedDirectionalNestedAp {
        observed_gate,
        full_assessment: Some(DirectionalNestedFiniteEvidence {
            supported_magnitude: nested.supported_magnitude,
            literal_survival: nested.literal_survival,
            verdict,
            retained_effect_rows: rows,
        }),
        staged_verdict: verdict,
    })
}

fn gate_passes(gate: &DirectionalFiniteEvidence, policy: ReplacementPolicy) -> bool {
    policy_verdict(
        policy,
        gate.supported_magnitude,
        gate.literal_survival.subset_fraction,
    ) == FiniteEvidenceVerdict::SupportedReplacement
}

fn policy_verdict(
    policy: ReplacementPolicy,
    magnitude: f64,
    survival: f64,
) -> FiniteEvidenceVerdict {
    if magnitude > policy.magnitude_threshold.get() && survival > policy.survival_requirement.get()
    {
        FiniteEvidenceVerdict::SupportedReplacement
    } else {
        FiniteEvidenceVerdict::NoVerdict
    }
}

fn row_design(row: &RowData) -> RowDesign {
    let minimum_cluster_size = row.clusters.iter().map(Vec::len).min().unwrap_or(0);
    let maximum_cluster_size = row.clusters.iter().map(Vec::len).max().unwrap_or(0);
    RowDesign {
        evaluation_id: row.id.clone(),
        observation_count: row.labels.len(),
        position_cluster_count: row.clusters.len(),
        minimum_cluster_size,
        maximum_cluster_size,
    }
}

fn parse_label(raw: &str, row: usize) -> Result<bool> {
    match raw.trim() {
        "1" | "true" | "TRUE" => Ok(true),
        "0" | "false" | "FALSE" => Ok(false),
        _ => Err(format!("invalid label at row {row}: {raw}").into()),
    }
}

fn parse_score(raw: &str, row: usize, column: &str) -> Result<f64> {
    let score = raw
        .parse::<f64>()
        .map_err(|_| format!("invalid score at row {row}, column {column}: {raw}"))?;
    if !score.is_finite() {
        return Err(format!("nonfinite score at row {row}, column {column}: {raw}").into());
    }
    Ok(score)
}

fn mix_seed(seed: u64, indices: &[usize]) -> u64 {
    let mut mixed = splitmix64(seed ^ CLUSTER_BOOTSTRAP_DOMAIN);
    for &index in indices {
        mixed = splitmix64(mixed ^ index as u64);
    }
    mixed
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = value;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
