use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;
use supported_ap::{
    AuRocOptimizationOptions, AuRocPolicy, ClassCounts, ComputationalReplicationCount,
    ConcentrationSearchOptions, Execution, MagnitudeThreshold, ObservedApAssessment,
    ObservedAuRocAssessment, PairedEvaluation, Prevalence, ProjectedApAssessment,
    ProjectedAuRocAssessment, ProjectedResampling, ReferenceAssessment, ReplacementPolicy,
    ResamplingUnit, ScoreTransportAssumption, SearchOptions, SupportOrder, SurvivalFloor,
    SurvivalRequirement, TargetPrevalences, assess_observed_ap, assess_staged_projected_ap,
    estimate_observed_auroc_breakdown, estimate_projected_auroc_breakdown,
};

type CliResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Parser)]
#[command(
    name = "supported_ap",
    version,
    about = "V17 staged finite-evidence model-replacement assessments"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Ap(ApArgs),
    Auroc(AuRocArgs),
}

#[derive(Debug, Args)]
struct ApArgs {
    #[command(subcommand)]
    command: ApCommand,
}

#[derive(Debug, Subcommand)]
enum ApCommand {
    Projected(ProjectedApArgs),
    Observed(ObservedApArgs),
}

#[derive(Debug, Args)]
struct AuRocArgs {
    #[command(subcommand)]
    command: AuRocCommand,
}

#[derive(Debug, Subcommand)]
enum AuRocCommand {
    Projected(ProjectedAuRocArgs),
    Observed(ObservedAuRocArgs),
}

#[derive(Debug, Clone, Args)]
struct ColumnArgs {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    label_col: String,
    #[arg(long)]
    model_a_col: String,
    #[arg(long)]
    model_b_col: String,
}

#[derive(Debug, Clone, Copy, Args)]
struct PolicyArgs {
    #[arg(long)]
    magnitude_threshold: MagnitudeThreshold,
    #[arg(long)]
    survival_floor: SurvivalFloor,
    #[arg(long)]
    survival_requirement: SurvivalRequirement,
}

impl PolicyArgs {
    const fn policy(self) -> ReplacementPolicy {
        ReplacementPolicy::new(
            self.magnitude_threshold,
            self.survival_floor,
            self.survival_requirement,
        )
    }
}

#[derive(Debug, Clone, Args)]
struct ReferenceArgs {
    #[arg(long, conflicts_with = "reference_limitation")]
    exchangeability_justification: Option<String>,
    #[arg(long, conflicts_with = "exchangeability_justification")]
    reference_limitation: Option<String>,
}

impl ReferenceArgs {
    fn assessment(&self) -> CliResult<ReferenceAssessment> {
        match (
            &self.exchangeability_justification,
            &self.reference_limitation,
        ) {
            (Some(justification), None) => {
                Ok(ReferenceAssessment::exchangeable_labels(justification)?)
            }
            (None, Some(reason)) => Ok(ReferenceAssessment::not_asserted(reason)?),
            (None, None) => Err(
                "provide --exchangeability-justification or --reference-limitation"
                    .to_owned()
                    .into(),
            ),
            (Some(_), Some(_)) => unreachable!("clap rejects conflicting reference arguments"),
        }
    }
}

#[derive(Debug, Clone, Args)]
struct PrevalenceArgs {
    #[arg(long, conflicts_with_all = ["interval_lower", "interval_upper"])]
    prevalences: Option<String>,
    #[arg(long, requires = "interval_upper", conflicts_with = "prevalences")]
    interval_lower: Option<f64>,
    #[arg(long, requires = "interval_lower", conflicts_with = "prevalences")]
    interval_upper: Option<f64>,
    #[arg(long)]
    transport_justification: String,
    #[arg(long, default_value_t = 3)]
    /// Initial interval grid; additional points are chosen adaptively.
    search_grid_points: usize,
    #[arg(long, default_value_t = 1e-8)]
    /// Absolute error tolerance in the retained effect.
    search_tolerance: f64,
    #[arg(long, default_value_t = 128)]
    /// Maximum total interval bisections, shared by both directions.
    search_max_iterations: usize,
}

impl PrevalenceArgs {
    fn target_prevalences(&self) -> CliResult<TargetPrevalences> {
        if let Some(raw) = &self.prevalences {
            let values = raw
                .split(',')
                .map(str::parse::<f64>)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .map(Prevalence::new)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(TargetPrevalences::finite(values)?)
        } else if let (Some(lower), Some(upper)) = (self.interval_lower, self.interval_upper) {
            Ok(TargetPrevalences::closed_interval(
                Prevalence::new(lower)?,
                Prevalence::new(upper)?,
            )?)
        } else {
            Err("provide --prevalences or both interval endpoints"
                .to_owned()
                .into())
        }
    }

    fn search(&self) -> CliResult<SearchOptions> {
        Ok(SearchOptions::new(
            self.search_grid_points,
            self.search_tolerance,
            self.search_max_iterations,
        )?)
    }

    fn transport(&self) -> CliResult<ScoreTransportAssumption> {
        Ok(ScoreTransportAssumption::new(
            &self.transport_justification,
        )?)
    }
}

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

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ResamplingUnitArg {
    IndependentObservation,
}

impl From<ResamplingUnitArg> for ResamplingUnit {
    fn from(value: ResamplingUnitArg) -> Self {
        match value {
            ResamplingUnitArg::IndependentObservation => Self::IndependentObservation,
        }
    }
}

#[derive(Debug, Args)]
struct ProjectedApArgs {
    #[command(flatten)]
    columns: ColumnArgs,
    #[command(flatten)]
    policy: PolicyArgs,
    #[command(flatten)]
    reference: ReferenceArgs,
    #[command(flatten)]
    prevalence: PrevalenceArgs,
    #[arg(long)]
    computational_order: usize,
    #[arg(long)]
    computational_replications: usize,
    #[arg(long)]
    resampling_seed: u64,
    #[arg(long)]
    replication_positive_count: Option<usize>,
    #[arg(long)]
    replication_negative_count: Option<usize>,
    #[arg(long, value_enum)]
    resampling_unit: ResamplingUnitArg,
    #[arg(long, value_enum, default_value_t = ExecutionArg::Parallel)]
    execution: ExecutionArg,
    #[arg(long, default_value_t = false)]
    retain_replication_profiles: bool,
}

#[derive(Debug, Args)]
struct ObservedApArgs {
    #[command(flatten)]
    columns: ColumnArgs,
    #[command(flatten)]
    policy: PolicyArgs,
    #[command(flatten)]
    reference: ReferenceArgs,
    #[command(flatten)]
    prevalence: PrevalenceArgs,
    #[arg(long)]
    evaluation_id_col: String,
    #[arg(long)]
    empirical_order: usize,
    #[arg(long, value_enum, default_value_t = ExecutionArg::Parallel)]
    execution: ExecutionArg,
}

#[derive(Debug, Clone, Args)]
struct AuRocCommonArgs {
    #[command(flatten)]
    columns: ColumnArgs,
    #[command(flatten)]
    policy: PolicyArgs,
    #[command(flatten)]
    reference: ReferenceArgs,
    #[arg(long, value_enum, default_value_t = ExecutionArg::Parallel)]
    execution: ExecutionArg,
    #[arg(long, default_value_t = 1e-9)]
    optimization_absolute_gap: f64,
    #[arg(long, default_value_t = 1e-9)]
    optimization_relative_gap: f64,
    #[arg(long)]
    optimization_time_limit_seconds: Option<f64>,
    #[arg(long, default_value_t = 1)]
    solver_threads: u32,
    #[arg(long, default_value_t = 1e-4)]
    concentration_tolerance: f64,
    #[arg(long, default_value_t = 64)]
    concentration_max_iterations: usize,
}

impl AuRocCommonArgs {
    fn optimization(&self) -> CliResult<AuRocOptimizationOptions> {
        Ok(AuRocOptimizationOptions::new(
            self.optimization_absolute_gap,
            self.optimization_relative_gap,
            self.optimization_time_limit_seconds,
            self.solver_threads,
        )?)
    }

    fn search(&self) -> CliResult<ConcentrationSearchOptions> {
        Ok(ConcentrationSearchOptions::new(
            self.concentration_tolerance,
            self.concentration_max_iterations,
        )?)
    }
}

#[derive(Debug, Args)]
struct ProjectedAuRocArgs {
    #[command(flatten)]
    common: AuRocCommonArgs,
    #[arg(long)]
    computational_order: usize,
    #[arg(long)]
    computational_replications: usize,
    #[arg(long)]
    resampling_seed: u64,
    #[arg(long, value_enum)]
    resampling_unit: ResamplingUnitArg,
}

#[derive(Debug, Args)]
struct ObservedAuRocArgs {
    #[command(flatten)]
    common: AuRocCommonArgs,
    #[arg(long)]
    empirical_order: usize,
    #[arg(long)]
    evaluation_id_col: String,
}

#[derive(Serialize)]
#[serde(tag = "analysis", rename_all = "snake_case")]
enum Output {
    #[serde(rename = "ap_staged_projected_finite_evidence")]
    ApProjected {
        assessment: ProjectedApAssessment,
        result: Box<supported_ap::StagedProjectedApResult>,
    },
    #[serde(rename = "ap_observed_empirical_gate")]
    ApObserved {
        assessment: ObservedApAssessment,
        evaluation_ids: Vec<String>,
        result: Box<supported_ap::ObservedApResult>,
    },
    #[serde(rename = "auroc_staged_projected_breakdown")]
    AurocProjected {
        assessment: ProjectedAuRocAssessment,
        result: Box<supported_ap::AuRocBreakdownEstimate>,
    },
    #[serde(rename = "auroc_observed_empirical_breakdown")]
    AurocObserved {
        assessment: ObservedAuRocAssessment,
        evaluation_ids: Vec<String>,
        result: Box<supported_ap::AuRocBreakdownEstimate>,
    },
}

#[derive(Serialize)]
struct Report<'a> {
    schema_version: u32,
    manuscript_version: &'static str,
    #[serde(flatten)]
    output: &'a Output,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> CliResult<()> {
    match Cli::parse().command {
        Command::Ap(args) => match args.command {
            ApCommand::Projected(args) => run_ap_projected(args),
            ApCommand::Observed(args) => run_ap_observed(args),
        },
        Command::Auroc(args) => match args.command {
            AuRocCommand::Projected(args) => run_auroc_projected(args),
            AuRocCommand::Observed(args) => run_auroc_observed(args),
        },
    }
}

fn run_ap_projected(args: ProjectedApArgs) -> CliResult<()> {
    let evaluation = read_paired_evaluation(&args.columns)?;
    let observed_counts = evaluation.class_counts();
    let replication_counts = ClassCounts::new(
        args.replication_positive_count
            .unwrap_or(observed_counts.positive()),
        args.replication_negative_count
            .unwrap_or(observed_counts.negative()),
    )?;
    let assessment = ProjectedApAssessment {
        target_prevalences: args.prevalence.target_prevalences()?,
        transport: args.prevalence.transport()?,
        search: args.prevalence.search()?,
        resampling: ProjectedResampling::new(
            ComputationalReplicationCount::new(args.computational_replications)?,
            SupportOrder::new(args.computational_order)?,
            replication_counts,
            args.resampling_seed,
            args.execution.into(),
            args.resampling_unit.into(),
        )?,
        reference_assessment: args.reference.assessment()?,
    };
    let result = assess_staged_projected_ap(
        &evaluation,
        &assessment,
        args.policy.policy(),
        args.retain_replication_profiles,
    )?;
    write_output(
        &args.columns.output,
        &Output::ApProjected {
            assessment,
            result: Box::new(result),
        },
    )
}

fn run_ap_observed(args: ObservedApArgs) -> CliResult<()> {
    let grouped = read_grouped_paired_columns(&args.columns, &args.evaluation_id_col)?;
    let assessment = ObservedApAssessment {
        target_prevalences: args.prevalence.target_prevalences()?,
        transport: args.prevalence.transport()?,
        search: args.prevalence.search()?,
        empirical_order: SupportOrder::new(args.empirical_order)?,
        execution: args.execution.into(),
        reference_assessment: args.reference.assessment()?,
    };
    let result = assess_observed_ap(&grouped.evaluations, &assessment, args.policy.policy())?;
    write_output(
        &args.columns.output,
        &Output::ApObserved {
            assessment,
            evaluation_ids: grouped.ids,
            result: Box::new(result),
        },
    )
}

fn run_auroc_projected(args: ProjectedAuRocArgs) -> CliResult<()> {
    let evaluation = read_paired_evaluation(&args.common.columns)?;
    let assessment = ProjectedAuRocAssessment {
        resampling: ProjectedResampling::new(
            ComputationalReplicationCount::new(args.computational_replications)?,
            SupportOrder::new(args.computational_order)?,
            evaluation.class_counts(),
            args.resampling_seed,
            args.common.execution.into(),
            args.resampling_unit.into(),
        )?,
        optimization: args.common.optimization()?,
        concentration_search: args.common.search()?,
        reference_assessment: args.common.reference.assessment()?,
    };
    let policy: AuRocPolicy = args.common.policy.policy();
    let result = estimate_projected_auroc_breakdown(&evaluation, &assessment, policy)?;
    write_output(
        &args.common.columns.output,
        &Output::AurocProjected {
            assessment,
            result: Box::new(result),
        },
    )
}

fn run_auroc_observed(args: ObservedAuRocArgs) -> CliResult<()> {
    let grouped = read_grouped_paired_columns(&args.common.columns, &args.evaluation_id_col)?;
    let assessment = ObservedAuRocAssessment {
        empirical_order: SupportOrder::new(args.empirical_order)?,
        optimization: args.common.optimization()?,
        concentration_search: args.common.search()?,
        execution: args.common.execution.into(),
        reference_assessment: args.common.reference.assessment()?,
    };
    let result = estimate_observed_auroc_breakdown(
        &grouped.evaluations,
        &assessment,
        args.common.policy.policy(),
    )?;
    write_output(
        &args.common.columns.output,
        &Output::AurocObserved {
            assessment,
            evaluation_ids: grouped.ids,
            result: Box::new(result),
        },
    )
}

fn read_paired_evaluation(columns: &ColumnArgs) -> CliResult<PairedEvaluation> {
    let values = read_columns(
        &columns.input,
        &columns.label_col,
        &[&columns.model_a_col, &columns.model_b_col],
    )?;
    Ok(PairedEvaluation::new(
        &values.scores[0],
        &values.scores[1],
        &values.labels,
    )?)
}

struct Columns {
    labels: Vec<bool>,
    scores: Vec<Vec<f64>>,
}

#[derive(Default)]
struct GroupedColumns {
    labels: Vec<bool>,
    scores_a: Vec<f64>,
    scores_b: Vec<f64>,
}

struct GroupedPairedEvaluations {
    ids: Vec<String>,
    evaluations: Vec<PairedEvaluation>,
}

fn read_grouped_paired_columns(
    columns: &ColumnArgs,
    evaluation_id_col: &str,
) -> CliResult<GroupedPairedEvaluations> {
    let mut reader = csv::Reader::from_path(&columns.input)?;
    let headers = reader.headers()?.clone();
    let find = |column: &str| {
        headers
            .iter()
            .position(|header| header == column)
            .ok_or_else(|| supported_ap::Error::MissingColumn(column.to_owned()))
    };
    let id_index = find(evaluation_id_col)?;
    let label_index = find(&columns.label_col)?;
    let model_a_index = find(&columns.model_a_col)?;
    let model_b_index = find(&columns.model_b_col)?;
    let mut groups: BTreeMap<String, GroupedColumns> = BTreeMap::new();
    for (row_index, record) in reader.records().enumerate() {
        let record = record?;
        let row = row_index + 2;
        let group = groups.entry(record[id_index].to_owned()).or_default();
        group.labels.push(parse_label(&record[label_index], row)?);
        group.scores_a.push(parse_score(
            &record[model_a_index],
            row,
            &columns.model_a_col,
        )?);
        group.scores_b.push(parse_score(
            &record[model_b_index],
            row,
            &columns.model_b_col,
        )?);
    }
    if groups.is_empty() {
        return Err(supported_ap::Error::EmptyEmpiricalEvaluations.into());
    }
    let mut ids = Vec::with_capacity(groups.len());
    let mut evaluations = Vec::with_capacity(groups.len());
    for (id, group) in groups {
        ids.push(id);
        evaluations.push(PairedEvaluation::new(
            &group.scores_a,
            &group.scores_b,
            &group.labels,
        )?);
    }
    Ok(GroupedPairedEvaluations { ids, evaluations })
}

fn read_columns(path: &Path, label_col: &str, score_cols: &[&str]) -> CliResult<Columns> {
    let mut reader = csv::Reader::from_path(path)?;
    let headers = reader.headers()?.clone();
    let label_index = headers
        .iter()
        .position(|header| header == label_col)
        .ok_or_else(|| supported_ap::Error::MissingColumn(label_col.to_owned()))?;
    let score_indices = score_cols
        .iter()
        .map(|column| {
            headers
                .iter()
                .position(|header| header == *column)
                .ok_or_else(|| supported_ap::Error::MissingColumn((*column).to_owned()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut labels = Vec::new();
    let mut scores = vec![Vec::new(); score_cols.len()];
    for (row_index, record) in reader.records().enumerate() {
        let record = record?;
        labels.push(parse_label(&record[label_index], row_index + 2)?);
        for ((values, &column_index), column) in
            scores.iter_mut().zip(&score_indices).zip(score_cols)
        {
            values.push(parse_score(&record[column_index], row_index + 2, column)?);
        }
    }
    Ok(Columns { labels, scores })
}

fn parse_label(raw: &str, row: usize) -> CliResult<bool> {
    match raw.trim() {
        "1" | "true" | "TRUE" => Ok(true),
        "0" | "false" | "FALSE" => Ok(false),
        _ => Err(supported_ap::Error::InvalidLabel {
            row,
            value: raw.to_owned(),
        }
        .into()),
    }
}

fn parse_score(raw: &str, row: usize, column: &str) -> CliResult<f64> {
    raw.parse::<f64>().map_err(|_| {
        supported_ap::Error::InvalidCsvScore {
            row,
            column: column.to_owned(),
            value: raw.to_owned(),
        }
        .into()
    })
}

fn write_output(path: &Path, output: &Output) -> CliResult<()> {
    let report = Report {
        schema_version: 17,
        manuscript_version: "v17",
        output,
    };
    serde_json::to_writer_pretty(File::create(path)?, &report)?;
    Ok(())
}
