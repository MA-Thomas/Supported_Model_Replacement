//! Command line front end for the `supported_ap` crate.
//!
//! Reads a CSV of scores and outcomes and writes a JSON report. Input is
//! streamed and projected to the requested columns, so memory tracks the
//! retained columns rather than the file. Every model selected from one file
//! is evaluated on the same rows and against the same resampled units, so the
//! results are comparable. A reference-prevalence sweep reuses one set of
//! replicates across all its values.

use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::Serialize;
use supported_ap::{
    BootstrapOptions, CalibratedSupportedCnapEstimate, ClassCounts,
    ConditionalNullCalibrationOptions, Evaluation, NullDistributionStorage, PairedEvaluation,
    Parallelism, PermutationCount, PermutationOptions, ReferencePrevalence, ReplicateCount,
    SupportOrder,
};
use tempfile::NamedTempFile;

type CliResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

const SCHEMA_VERSION: u32 = 8;
const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");
const PARALLEL_ENABLED: bool = cfg!(feature = "parallel");

#[derive(Debug)]
struct Args {
    input: PathBuf,
    output: PathBuf,
    label_col: String,
    score_cols: Vec<String>,
    score_prefix: Option<String>,
    prevalences: Vec<PrevalenceSpec>,
    paired_baseline: Option<String>,
    replication_positive_count: Option<usize>,
    replication_negative_count: Option<usize>,
    bootstrap_replicates: usize,
    permutations: usize,
    support_order: usize,
    survival_frequency: f64,
    bootstrap_seed: u64,
    permutation_seed: u64,
    threads: Option<usize>,
    common_rows: bool,
    fail_on_error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PrevalenceSpec {
    Observed,
    Fixed(f64),
}

impl PrevalenceSpec {
    fn parse(token: &str) -> Result<Self, String> {
        let token = token.trim();
        if token.eq_ignore_ascii_case("observed") {
            return Ok(Self::Observed);
        }
        token
            .parse::<f64>()
            .map(Self::Fixed)
            .map_err(|error| format!("invalid --reference-prevalence `{token}`: {error}"))
    }

    fn resolve(self, n_positive: usize, n_negative: usize) -> Result<f64, String> {
        match self {
            Self::Fixed(value) => Ok(value),
            Self::Observed => {
                let total = n_positive + n_negative;
                if total == 0 {
                    return Err("observed reference prevalence needs at least one valid row".into());
                }
                Ok(n_positive as f64 / total as f64)
            }
        }
    }

    fn label(self) -> String {
        match self {
            Self::Observed => "observed".to_string(),
            Self::Fixed(value) => value.to_string(),
        }
    }
}

impl Args {
    fn parse() -> CliResult<Option<Self>> {
        let mut input = None;
        let mut output = None;
        let mut label_col = None;
        let mut score_cols = Vec::new();
        let mut score_prefix = Some("score_".to_string());
        let mut prevalence_tokens: Vec<String> = Vec::new();
        let mut paired_baseline = None;
        let mut replication_positive_count = None;
        let mut replication_negative_count = None;
        let mut bootstrap_replicates = 5_000usize;
        let mut permutations = 500usize;
        let mut support_order = 2usize;
        let mut survival_frequency = 0.5f64;
        let mut bootstrap_seed = 0x5355_5041_5042_4f4f_u64;
        let mut permutation_seed = 0x5355_5041_5050_4552_u64;
        let mut threads = None;
        let mut common_rows = true;
        let mut fail_on_error = false;

        let mut argv = env::args().skip(1);
        while let Some(flag) = argv.next() {
            match flag.as_str() {
                "--input" => input = Some(parse_path(&mut argv, &flag)?),
                "--output" => output = Some(parse_path(&mut argv, &flag)?),
                "--label-col" => label_col = Some(parse_value(&mut argv, &flag)?),
                "--score-col" => score_cols.push(parse_value(&mut argv, &flag)?),
                "--score-prefix" => score_prefix = Some(parse_value(&mut argv, &flag)?),
                "--no-score-prefix" => score_prefix = None,
                "--reference-prevalence" => {
                    let raw: String = parse_value(&mut argv, &flag)?;
                    prevalence_tokens.extend(raw.split(',').map(|part| part.to_string()));
                }
                "--paired-baseline" => paired_baseline = Some(parse_value(&mut argv, &flag)?),
                "--replication-positive-count" => {
                    replication_positive_count = Some(parse_value::<usize>(&mut argv, &flag)?)
                }
                "--replication-negative-count" => {
                    replication_negative_count = Some(parse_value::<usize>(&mut argv, &flag)?)
                }
                "--bootstrap-replicates" => {
                    bootstrap_replicates = parse_value::<usize>(&mut argv, &flag)?
                }
                "--permutations" => permutations = parse_value::<usize>(&mut argv, &flag)?,
                "--support-order" => support_order = parse_value::<usize>(&mut argv, &flag)?,
                "--survival-frequency" => {
                    survival_frequency = parse_value::<f64>(&mut argv, &flag)?
                }
                "--bootstrap-seed" => bootstrap_seed = parse_value::<u64>(&mut argv, &flag)?,
                "--permutation-seed" => permutation_seed = parse_value::<u64>(&mut argv, &flag)?,
                "--threads" => threads = Some(parse_value::<usize>(&mut argv, &flag)?),
                "--allow-ragged-rows" => common_rows = false,
                "--fail-on-error" => fail_on_error = true,
                "--version" | "-V" => {
                    println!("supported_ap_metrics {CRATE_VERSION}");
                    return Ok(None);
                }
                "--help" | "-h" => {
                    print_help();
                    return Ok(None);
                }
                other => return Err(format!("unknown argument: {other}").into()),
            }
        }

        if prevalence_tokens.is_empty() {
            return Err(
                "--reference-prevalence is required; pass a value in (0,1), \
                        a comma-separated sweep, or the literal `observed`"
                    .into(),
            );
        }
        let mut prevalences = Vec::new();
        for token in &prevalence_tokens {
            let spec = PrevalenceSpec::parse(token)?;
            if !prevalences.contains(&spec) {
                prevalences.push(spec);
            }
        }
        if !survival_frequency.is_finite() || survival_frequency <= 0.0 || survival_frequency > 1.0
        {
            return Err(format!(
                "--survival-frequency must be finite and in (0, 1], received {survival_frequency}"
            )
            .into());
        }
        if paired_baseline.is_none()
            && (replication_positive_count.is_some() || replication_negative_count.is_some())
        {
            return Err(
                "--replication-positive-count and --replication-negative-count require \
                 --paired-baseline"
                    .into(),
            );
        }
        if replication_positive_count == Some(0) || replication_negative_count == Some(0) {
            return Err("prospective replication class counts must be positive".into());
        }

        // An explicit thread count without the feature can never succeed, so
        // it fails here rather than after the input has been read.
        if threads.is_some() && !PARALLEL_ENABLED {
            return Err("--threads requires a build with --features parallel".into());
        }

        Ok(Some(Self {
            input: input.ok_or("--input is required")?,
            output: output.ok_or("--output is required")?,
            label_col: label_col.ok_or("--label-col is required")?,
            score_cols,
            score_prefix,
            prevalences,
            paired_baseline,
            replication_positive_count,
            replication_negative_count,
            bootstrap_replicates,
            permutations,
            support_order,
            survival_frequency,
            bootstrap_seed,
            permutation_seed,
            threads,
            common_rows,
            fail_on_error,
        }))
    }
}

fn parse_value<T>(argv: &mut impl Iterator<Item = String>, flag: &str) -> CliResult<T>
where
    T: std::str::FromStr,
    T::Err: Error + Send + Sync + 'static,
{
    let value = argv
        .next()
        .ok_or_else(|| format!("{flag} requires a value"))?;
    Ok(value.parse::<T>()?)
}

fn parse_path(argv: &mut impl Iterator<Item = String>, flag: &str) -> CliResult<PathBuf> {
    Ok(PathBuf::from(parse_value::<String>(argv, flag)?))
}

fn print_help() {
    println!(
        "\
supported_ap_metrics --input predictions.csv --output supported_ap_metrics.json --label-col label \\
    --reference-prevalence 0.10 [--score-col score_a ... | --score-prefix score_] [options]

Required:
  --input PATH
  --output PATH
  --label-col NAME
  --reference-prevalence observed|FLOAT[,FLOAT...]
        A comma-separated list runs a sweep from one set of replicates. Model
        orderings can reverse across reference prevalences, and the conditional
        null is recomputed at each value.

Options:
  --score-col NAME              Repeatable. Overrides --score-prefix.
  --score-prefix PREFIX         Default: score_
  --no-score-prefix
  --paired-baseline NAME        Also report estimated supported superiority of
                                every other model over this one, using shared
                                resamples.
  --replication-positive-count N
                                Positive units in each prospective paired
                                replication. Default: observed positive count.
  --replication-negative-count N
                                Negative units in each prospective paired
                                replication. Default: observed negative count.
  --bootstrap-replicates N      Default: 5000
  --permutations N              Default: 500
  --support-order K             Default: 2. Replications a claim must survive.
  --survival-frequency GAMMA    Default: 0.5. Reports m_gamma.
  --bootstrap-seed N
  --permutation-seed N
  --threads N                   Requires build with --features parallel
  --allow-ragged-rows           Evaluate each model on its own valid rows.
                                Off by default, because differing row sets make
                                models incomparable.
  --fail-on-error               Exit non-zero if any result failed.
  --version, -V
  --help, -h
"
    );
}

/// One model's parsed column together with the rows it was evaluated on.
///
/// Under the default shared row set every column holds identical labels, which
/// is what makes the shared resample plan meaningful: equal class counts and
/// equal seeds draw the same units for every model.
#[derive(Debug)]
struct Column {
    name: String,
    scores: Vec<f64>,
    labels: Vec<bool>,
    n_positive: usize,
    n_negative: usize,
    skipped_score: usize,
}

#[derive(Debug)]
struct Extraction {
    columns: Vec<Column>,
    rows_read: usize,
    skipped_label: usize,
    shared_rows: Option<usize>,
    skipped_any_score: Option<usize>,
}

fn column_index(header: &[String], name: &str) -> CliResult<usize> {
    header
        .iter()
        .position(|candidate| candidate == name)
        .ok_or_else(|| format!("column not found: {name}").into())
}

fn select_score_columns(args: &Args, header: &[String]) -> CliResult<Vec<String>> {
    let mut selected = args.score_cols.clone();
    if selected.is_empty() {
        if let Some(prefix) = &args.score_prefix {
            selected = header
                .iter()
                .filter(|column| column.starts_with(prefix))
                .cloned()
                .collect();
        }
    }
    if selected.is_empty() {
        return Err("no score columns selected".into());
    }

    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for column in selected {
        if seen.insert(column.clone()) {
            deduped.push(column);
        }
    }
    Ok(deduped)
}

/// Stream the input, projecting the label column and the selected score
/// columns straight into typed vectors.
///
/// By default a row is retained only when the label and every selected score
/// parse, so all models are evaluated on one row set. Differing row sets would
/// make the models incomparable and would silently break the shared resample
/// plan.
fn load(args: &Args) -> CliResult<Extraction> {
    let file = File::open(&args.input)
        .map_err(|error| format!("failed to open {}: {error}", args.input.display()))?;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(false)
        .from_reader(file);
    let mut header: Vec<String> = reader
        .headers()
        .map_err(|error| format!("failed to read {} header: {error}", args.input.display()))?
        .iter()
        .map(str::to_owned)
        .collect();
    if header.is_empty() {
        return Err(format!("{} is empty", args.input.display()).into());
    }
    // Spreadsheets commonly write a byte order mark, which would otherwise
    // become part of the first column name.
    if let Some(first) = header.first_mut() {
        if let Some(trimmed) = first.strip_prefix('\u{feff}') {
            *first = trimmed.to_string();
        }
    }
    let mut header_names = HashSet::new();
    if let Some(duplicate) = header
        .iter()
        .find(|name| !header_names.insert((*name).clone()))
    {
        return Err(format!("duplicate column name in header: {duplicate}").into());
    }

    let label_idx = column_index(&header, &args.label_col)?;
    let selected = select_score_columns(args, &header)?;
    if selected.iter().any(|name| name == &args.label_col) {
        return Err(format!(
            "the label column `{}` cannot also be selected as a score column",
            args.label_col
        )
        .into());
    }
    if let Some(baseline) = &args.paired_baseline {
        if !selected.iter().any(|name| name == baseline) {
            return Err(format!(
                "--paired-baseline {baseline} is not among the selected score columns"
            )
            .into());
        }
        if !args.common_rows {
            return Err("--paired-baseline requires a shared row set; \
                        remove --allow-ragged-rows"
                .into());
        }
    }
    let indices: Vec<usize> = selected
        .iter()
        .map(|name| column_index(&header, name))
        .collect::<CliResult<Vec<_>>>()?;

    let mut scores: Vec<Vec<f64>> = vec![Vec::new(); selected.len()];
    let mut labels: Vec<Vec<bool>> = vec![Vec::new(); selected.len()];
    let mut skipped_score = vec![0usize; selected.len()];
    let mut parsed: Vec<Option<f64>> = vec![None; selected.len()];
    let mut rows_read = 0usize;
    let mut skipped_label = 0usize;
    let mut shared_rows = 0usize;
    let mut skipped_any_score = 0usize;

    for record in reader.records() {
        let fields = record.map_err(|error| {
            format!(
                "failed to read {} record {}: {error}",
                args.input.display(),
                rows_read + 2
            )
        })?;
        rows_read += 1;

        let label_field = fields
            .get(label_idx)
            .ok_or_else(|| format!("record {} is missing the label column", rows_read + 1))?;
        let Some(label) = parse_label(label_field) else {
            skipped_label += 1;
            continue;
        };
        for (slot, &index) in parsed.iter_mut().zip(indices.iter()) {
            *slot = match fields
                .get(index)
                .and_then(|field| field.trim().parse::<f64>().ok())
            {
                Some(value) if value.is_finite() => Some(value),
                _ => None,
            };
        }
        for (position, value) in parsed.iter().enumerate() {
            if value.is_none() {
                skipped_score[position] += 1;
            }
        }
        let usable_everywhere = parsed.iter().all(Option::is_some);
        if usable_everywhere {
            shared_rows += 1;
        }
        if args.common_rows && !usable_everywhere {
            skipped_any_score += 1;
            continue;
        }
        for position in 0..selected.len() {
            if let Some(value) = parsed[position] {
                scores[position].push(value);
                labels[position].push(label);
            }
        }
    }

    let mut columns = Vec::with_capacity(selected.len());
    for (position, name) in selected.into_iter().enumerate() {
        let column_labels = std::mem::take(&mut labels[position]);
        let n_positive = column_labels.iter().filter(|&&label| label).count();
        let n_negative = column_labels.len() - n_positive;
        columns.push(Column {
            name,
            scores: std::mem::take(&mut scores[position]),
            labels: column_labels,
            n_positive,
            n_negative,
            skipped_score: skipped_score[position],
        });
    }

    Ok(Extraction {
        columns,
        rows_read,
        skipped_label,
        shared_rows: if args.common_rows {
            Some(shared_rows)
        } else {
            None
        },
        skipped_any_score: if args.common_rows {
            Some(skipped_any_score)
        } else {
            None
        },
    })
}

fn parse_label(value: &str) -> Option<bool> {
    let trimmed = value.trim();
    match trimmed.to_ascii_lowercase().as_str() {
        "1" | "1.0" | "true" | "t" | "yes" | "y" => Some(true),
        "0" | "0.0" | "false" | "f" | "no" | "n" => Some(false),
        _ => {
            let parsed = trimmed.parse::<f64>().ok()?;
            if parsed == 1.0 {
                Some(true)
            } else if parsed == 0.0 {
                Some(false)
            } else {
                None
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct MetricContext {
    score_col: String,
    n: usize,
    n_positive: usize,
    n_negative: usize,
    #[serde(rename = "n_skipped_score")]
    skipped_score: usize,
    #[serde(rename = "reference_prevalence_mode")]
    prevalence_mode: String,
}

#[derive(Debug, Serialize)]
struct MetricSuccess {
    #[serde(flatten)]
    context: MetricContext,
    status: &'static str,
    reference_prevalence: f64,
    prior_standardized_ap: f64,
    chance_normalized_ap: f64,
    replicate_mean: f64,
    support_penalty: f64,
    supported_cnap: f64,
    #[serde(rename = "supported_cnap_mc_se")]
    supported_cnap_se: f64,
    conditional_null_supported_cnap: f64,
    #[serde(rename = "conditional_null_supported_cnap_mc_se")]
    conditional_null_supported_cnap_se: f64,
    calibrated_supported_cnap: f64,
    #[serde(rename = "calibrated_supported_cnap_mc_se")]
    calibrated_supported_cnap_se: f64,
    permutation_p_value: f64,
    survival_level: f64,
    #[serde(rename = "conditional_null_q05")]
    conditional_null_p05: f64,
    #[serde(rename = "conditional_null_q50")]
    conditional_null_p50: f64,
    #[serde(rename = "conditional_null_q95")]
    conditional_null_p95: f64,
    above_conditional_null: bool,
    calibrated_supported_cnap_is_extrapolation: bool,
}

#[derive(Debug, Serialize)]
struct MetricFailure {
    #[serde(flatten)]
    context: MetricContext,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference_prevalence: Option<f64>,
    error: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum MetricResult {
    Success(MetricSuccess),
    Failure(MetricFailure),
}

impl MetricResult {
    const fn is_failure(&self) -> bool {
        matches!(self, Self::Failure(_))
    }
}

#[derive(Debug, Clone, Serialize)]
struct PairedContext {
    model_a: String,
    model_b: String,
    observed_positive_count: usize,
    observed_negative_count: usize,
    replication_positive_count: usize,
    replication_negative_count: usize,
    #[serde(rename = "reference_prevalence_mode")]
    prevalence_mode: String,
}

#[derive(Debug, Serialize)]
struct PairedSuccess {
    #[serde(flatten)]
    context: PairedContext,
    status: &'static str,
    reference_prevalence: f64,
    observed_cnap_difference: f64,
    replicate_mean_cnap_difference: f64,
    estimated_supported_superiority: f64,
    support_penalty: f64,
    #[serde(rename = "estimated_supported_superiority_mc_se")]
    estimated_supported_superiority_se: f64,
}

#[derive(Debug, Serialize)]
struct PairedFailure {
    #[serde(flatten)]
    context: PairedContext,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference_prevalence: Option<f64>,
    error: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum PairedResult {
    Success(PairedSuccess),
    Failure(PairedFailure),
}

impl PairedResult {
    const fn is_failure(&self) -> bool {
        matches!(self, Self::Failure(_))
    }
}

#[derive(Debug)]
struct PendingMetric {
    context: MetricContext,
    prevalence: ReferencePrevalence,
}

#[derive(Debug)]
struct PendingPaired {
    context: PairedContext,
    prevalence: ReferencePrevalence,
}

fn bootstrap_options(args: &Args, parallelism: Parallelism) -> CliResult<BootstrapOptions> {
    Ok(BootstrapOptions {
        replicates: ReplicateCount::new(args.bootstrap_replicates)?,
        support_order: SupportOrder::new(args.support_order)?,
        seed: args.bootstrap_seed,
        parallelism,
    })
}

fn permutation_options(args: &Args, parallelism: Parallelism) -> CliResult<PermutationOptions> {
    Ok(PermutationOptions {
        permutations: PermutationCount::new(args.permutations)?,
        seed: args.permutation_seed,
        parallelism,
        storage: NullDistributionStorage::Full,
    })
}

/// Every reference prevalence for one model, from a single set of replicates.
fn compute_column(column: &Column, args: &Args, parallelism: Parallelism) -> Vec<MetricResult> {
    let resolved: Vec<Result<PendingMetric, MetricFailure>> = args
        .prevalences
        .iter()
        .map(|spec| {
            let context = MetricContext {
                score_col: column.name.clone(),
                n: column.labels.len(),
                n_positive: column.n_positive,
                n_negative: column.n_negative,
                skipped_score: column.skipped_score,
                prevalence_mode: spec.label(),
            };
            spec.resolve(column.n_positive, column.n_negative)
                .and_then(|value| {
                    ReferencePrevalence::new(value).map_err(|error| error.to_string())
                })
                .map(|prevalence| PendingMetric {
                    context: context.clone(),
                    prevalence,
                })
                .map_err(|error| MetricFailure {
                    context,
                    status: "error",
                    reference_prevalence: None,
                    error,
                })
        })
        .collect();

    let usable: Vec<ReferencePrevalence> = resolved
        .iter()
        .filter_map(|entry| entry.as_ref().ok().map(|pending| pending.prevalence))
        .collect();
    if usable.is_empty() {
        return resolved
            .into_iter()
            .filter_map(Result::err)
            .map(MetricResult::Failure)
            .collect();
    }

    let computed = (|| -> CliResult<Vec<CalibratedSupportedCnapEstimate>> {
        let evaluation = Evaluation::new(&column.scores, &column.labels)?;
        let profile = evaluation.conditionally_calibrated_supported_cnap_profile(
            &usable,
            ConditionalNullCalibrationOptions {
                bootstrap: bootstrap_options(args, parallelism)?,
                permutation: permutation_options(args, parallelism)?,
            },
        )?;
        Ok(profile)
    })();

    match computed {
        Ok(profile) => {
            let mut estimates = profile.into_iter();
            resolved
                .into_iter()
                .map(|entry| match entry {
                    Err(failure) => MetricResult::Failure(failure),
                    Ok(pending) => {
                        let prevalence = pending.prevalence.value();
                        let Some(estimate) = estimates.next() else {
                            return MetricResult::Failure(MetricFailure {
                                context: pending.context,
                                status: "error",
                                reference_prevalence: Some(prevalence),
                                error: "missing profile entry".to_string(),
                            });
                        };
                        match metric_success(
                            pending.context.clone(),
                            pending.prevalence,
                            &estimate,
                            args.survival_frequency,
                        ) {
                            Ok(success) => MetricResult::Success(success),
                            Err(error) => MetricResult::Failure(MetricFailure {
                                context: pending.context,
                                status: "error",
                                reference_prevalence: Some(prevalence),
                                error,
                            }),
                        }
                    }
                })
                .collect()
        }
        Err(error) => {
            let message = error.to_string();
            resolved
                .into_iter()
                .map(|entry| match entry {
                    Err(failure) => MetricResult::Failure(failure),
                    Ok(pending) => MetricResult::Failure(MetricFailure {
                        context: pending.context,
                        status: "error",
                        reference_prevalence: Some(pending.prevalence.value()),
                        error: message.clone(),
                    }),
                })
                .collect()
        }
    }
}

fn metric_success(
    context: MetricContext,
    prevalence: ReferencePrevalence,
    estimate: &CalibratedSupportedCnapEstimate,
    survival_frequency: f64,
) -> Result<MetricSuccess, String> {
    let inner = estimate.estimate();
    let above_conditional_null = estimate.is_above_conditional_null();
    Ok(MetricSuccess {
        context,
        status: "ok",
        reference_prevalence: prevalence.value(),
        prior_standardized_ap: finite_result(
            "prior_standardized_ap",
            inner.observed().prior_standardized_ap().value(),
        )?,
        chance_normalized_ap: finite_result(
            "chance_normalized_ap",
            inner.observed().cnap().value(),
        )?,
        replicate_mean: finite_result("replicate_mean", inner.replicate_mean().value())?,
        support_penalty: finite_result("support_penalty", inner.support_penalty())?,
        supported_cnap: finite_result("supported_cnap", inner.supported().value())?,
        supported_cnap_se: finite_result(
            "supported_cnap_mc_se",
            inner.monte_carlo_standard_error(),
        )?,
        conditional_null_supported_cnap: finite_result(
            "conditional_null_supported_cnap",
            estimate.conditional_null().value(),
        )?,
        conditional_null_supported_cnap_se: finite_result(
            "conditional_null_supported_cnap_mc_se",
            estimate.conditional_null_monte_carlo_standard_error(),
        )?,
        calibrated_supported_cnap: finite_result(
            "calibrated_supported_cnap",
            estimate.calibrated().value(),
        )?,
        calibrated_supported_cnap_se: finite_result(
            "calibrated_supported_cnap_mc_se",
            estimate.monte_carlo_standard_error(),
        )?,
        permutation_p_value: finite_result("permutation_p_value", estimate.p_value().value())?,
        survival_level: finite_result(
            "survival_level",
            inner
                .survival_level(survival_frequency)
                .map_err(|error| error.to_string())?,
        )?,
        conditional_null_p05: finite_result(
            "conditional_null_q05",
            estimate
                .conditional_null_quantile(0.05)
                .map_err(|error| error.to_string())?,
        )?,
        conditional_null_p50: finite_result(
            "conditional_null_q50",
            estimate
                .conditional_null_quantile(0.50)
                .map_err(|error| error.to_string())?,
        )?,
        conditional_null_p95: finite_result(
            "conditional_null_q95",
            estimate
                .conditional_null_quantile(0.95)
                .map_err(|error| error.to_string())?,
        )?,
        above_conditional_null,
        calibrated_supported_cnap_is_extrapolation: !above_conditional_null,
    })
}

fn finite_result(name: &str, value: f64) -> Result<f64, String> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("{name} is not finite: {value}"))
    }
}

/// Estimated supported superiority of `column` over `baseline` at each prevalence.
fn compute_paired(
    column: &Column,
    baseline: &str,
    extraction: &Extraction,
    args: &Args,
    parallelism: Parallelism,
) -> Vec<PairedResult> {
    let Some(baseline_scores) = extraction
        .columns
        .iter()
        .find(|candidate| candidate.name == baseline)
        .map(|found| &found.scores)
    else {
        return args
            .prevalences
            .iter()
            .map(|spec| {
                PairedResult::Failure(PairedFailure {
                    context: PairedContext {
                        model_a: column.name.clone(),
                        model_b: baseline.to_string(),
                        observed_positive_count: column.n_positive,
                        observed_negative_count: column.n_negative,
                        replication_positive_count: args
                            .replication_positive_count
                            .unwrap_or(column.n_positive),
                        replication_negative_count: args
                            .replication_negative_count
                            .unwrap_or(column.n_negative),
                        prevalence_mode: spec.label(),
                    },
                    status: "error",
                    reference_prevalence: None,
                    error: format!("baseline column not found: {baseline}"),
                })
            })
            .collect();
    };

    let resolved: Vec<Result<PendingPaired, PairedFailure>> = args
        .prevalences
        .iter()
        .map(|spec| {
            let context = PairedContext {
                model_a: column.name.clone(),
                model_b: baseline.to_string(),
                observed_positive_count: column.n_positive,
                observed_negative_count: column.n_negative,
                replication_positive_count: args
                    .replication_positive_count
                    .unwrap_or(column.n_positive),
                replication_negative_count: args
                    .replication_negative_count
                    .unwrap_or(column.n_negative),
                prevalence_mode: spec.label(),
            };
            spec.resolve(column.n_positive, column.n_negative)
                .and_then(|value| {
                    ReferencePrevalence::new(value).map_err(|error| error.to_string())
                })
                .map(|prevalence| PendingPaired {
                    context: context.clone(),
                    prevalence,
                })
                .map_err(|error| PairedFailure {
                    context,
                    status: "error",
                    reference_prevalence: None,
                    error,
                })
        })
        .collect();

    let usable: Vec<ReferencePrevalence> = resolved
        .iter()
        .filter_map(|entry| entry.as_ref().ok().map(|pending| pending.prevalence))
        .collect();
    if usable.is_empty() {
        return resolved
            .into_iter()
            .filter_map(Result::err)
            .map(PairedResult::Failure)
            .collect();
    }

    let computed = (|| -> CliResult<_> {
        let paired = PairedEvaluation::new(&column.scores, baseline_scores, &column.labels)?;
        let replication_counts = ClassCounts::new(
            args.replication_positive_count.unwrap_or(column.n_positive),
            args.replication_negative_count.unwrap_or(column.n_negative),
        )?;
        Ok(
            paired.estimated_supported_superiority_profile_with_replication_counts(
                &usable,
                replication_counts,
                bootstrap_options(args, parallelism)?,
            )?,
        )
    })();

    match computed {
        Ok(profile) => {
            let mut estimates = profile.into_iter();
            resolved
                .into_iter()
                .map(|entry| match entry {
                    Err(failure) => PairedResult::Failure(failure),
                    Ok(pending) => {
                        let prevalence = pending.prevalence.value();
                        let Some(estimate) = estimates.next() else {
                            return PairedResult::Failure(PairedFailure {
                                context: pending.context,
                                status: "error",
                                reference_prevalence: Some(prevalence),
                                error: "missing paired profile entry".to_string(),
                            });
                        };
                        match paired_success(pending.context.clone(), pending.prevalence, &estimate)
                        {
                            Ok(success) => PairedResult::Success(success),
                            Err(error) => PairedResult::Failure(PairedFailure {
                                context: pending.context,
                                status: "error",
                                reference_prevalence: Some(prevalence),
                                error,
                            }),
                        }
                    }
                })
                .collect()
        }
        Err(error) => {
            let message = error.to_string();
            resolved
                .into_iter()
                .map(|entry| match entry {
                    Err(failure) => PairedResult::Failure(failure),
                    Ok(pending) => PairedResult::Failure(PairedFailure {
                        context: pending.context,
                        status: "error",
                        reference_prevalence: Some(pending.prevalence.value()),
                        error: message.clone(),
                    }),
                })
                .collect()
        }
    }
}

fn paired_success(
    context: PairedContext,
    prevalence: ReferencePrevalence,
    estimate: &supported_ap::SupportedSuperiorityEstimate,
) -> Result<PairedSuccess, String> {
    Ok(PairedSuccess {
        context,
        status: "ok",
        reference_prevalence: prevalence.value(),
        observed_cnap_difference: finite_result(
            "observed_cnap_difference",
            estimate.observed_difference().value(),
        )?,
        replicate_mean_cnap_difference: finite_result(
            "replicate_mean_cnap_difference",
            estimate.replicate_mean_difference().value(),
        )?,
        estimated_supported_superiority: finite_result(
            "estimated_supported_superiority",
            estimate.estimated_supported_superiority().value(),
        )?,
        support_penalty: finite_result("support_penalty", estimate.support_penalty())?,
        estimated_supported_superiority_se: finite_result(
            "estimated_supported_superiority_mc_se",
            estimate.monte_carlo_standard_error(),
        )?,
    })
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> CliResult<ExitCode> {
    let Some(args) = Args::parse()? else {
        return Ok(ExitCode::SUCCESS);
    };

    let parallelism = match args.threads {
        Some(value) => Parallelism::threads(value)?,
        None => Parallelism::Auto,
    };
    let extraction = load(&args)?;

    let mut metrics = Vec::new();
    let mut paired = Vec::new();
    for column in &extraction.columns {
        metrics.extend(compute_column(column, &args, parallelism));
    }
    if let Some(baseline) = &args.paired_baseline {
        for column in &extraction.columns {
            if &column.name == baseline {
                continue;
            }
            paired.extend(compute_paired(
                column,
                baseline,
                &extraction,
                &args,
                parallelism,
            ));
        }
    }

    let failures = metrics.iter().filter(|result| result.is_failure()).count()
        + paired.iter().filter(|result| result.is_failure()).count();
    let json = render_json(&args, &extraction, &metrics, &paired, failures)?;
    write_atomic(&args.output, &json)
        .map_err(|error| format!("failed to write {}: {error}", args.output.display()))?;

    if failures > 0 {
        eprintln!(
            "warning: {failures} of {} results failed",
            metrics.len() + paired.len()
        );
        if args.fail_on_error {
            return Ok(ExitCode::FAILURE);
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(contents.as_bytes())?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[derive(Serialize)]
struct ThreadPolicy {
    mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    threads: Option<usize>,
}

#[derive(Serialize)]
struct RowSummary {
    read: usize,
    skipped_label: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    skipped_any_score: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retained_shared: Option<usize>,
}

#[derive(Serialize)]
struct Report<'a> {
    schema_version: u32,
    package_version: &'static str,
    parallel_feature: bool,
    input: String,
    label_col: &'a str,
    single_model_null: &'static str,
    conditional_null_calibration_upper_endpoint: f64,
    reference_prevalences: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    paired_replication_positive_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    paired_replication_negative_count: Option<usize>,
    bootstrap_replicates: usize,
    permutations: usize,
    support_order: usize,
    survival_frequency: f64,
    bootstrap_seed: u64,
    permutation_seed: u64,
    thread_policy: ThreadPolicy,
    row_mode: &'static str,
    rows: RowSummary,
    failures: usize,
    score_columns: Vec<&'a str>,
    metrics: &'a [MetricResult],
    paired: &'a [PairedResult],
}

fn render_json(
    args: &Args,
    extraction: &Extraction,
    metrics: &[MetricResult],
    paired: &[PairedResult],
    failures: usize,
) -> CliResult<String> {
    let report = Report {
        schema_version: SCHEMA_VERSION,
        package_version: CRATE_VERSION,
        parallel_feature: PARALLEL_ENABLED,
        input: args.input.display().to_string(),
        label_col: &args.label_col,
        single_model_null: "conditional permutation null with fixed scores",
        conditional_null_calibration_upper_endpoint: 1.0,
        reference_prevalences: args.prevalences.iter().map(|spec| spec.label()).collect(),
        paired_replication_positive_count: args.replication_positive_count,
        paired_replication_negative_count: args.replication_negative_count,
        bootstrap_replicates: args.bootstrap_replicates,
        permutations: args.permutations,
        support_order: args.support_order,
        survival_frequency: args.survival_frequency,
        bootstrap_seed: args.bootstrap_seed,
        permutation_seed: args.permutation_seed,
        thread_policy: ThreadPolicy {
            mode: if args.threads.is_some() {
                "fixed"
            } else {
                "auto"
            },
            threads: args.threads,
        },
        row_mode: if args.common_rows {
            "shared"
        } else {
            "per_model"
        },
        rows: RowSummary {
            read: extraction.rows_read,
            skipped_label: extraction.skipped_label,
            skipped_any_score: extraction.skipped_any_score,
            retained_shared: extraction.shared_rows,
        },
        failures,
        score_columns: extraction
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect(),
        metrics,
        paired,
    };
    let mut rendered = serde_json::to_string_pretty(&report)?;
    rendered.push('\n');
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        Args, Extraction, MetricContext, MetricFailure, MetricResult, MetricSuccess, PairedContext,
        PairedFailure, PairedResult, PrevalenceSpec, SCHEMA_VERSION, finite_result, render_json,
    };

    fn successful_metric() -> MetricResult {
        MetricResult::Success(MetricSuccess {
            context: MetricContext {
                score_col: "score_a".to_string(),
                n: 20,
                n_positive: 4,
                n_negative: 16,
                skipped_score: 1,
                prevalence_mode: "0.2".to_string(),
            },
            status: "ok",
            reference_prevalence: 0.2,
            prior_standardized_ap: 0.6,
            chance_normalized_ap: 0.5,
            replicate_mean: 0.48,
            support_penalty: 0.03,
            supported_cnap: 0.45,
            supported_cnap_se: 0.01,
            conditional_null_supported_cnap: 0.1,
            conditional_null_supported_cnap_se: 0.02,
            calibrated_supported_cnap: 0.4375,
            calibrated_supported_cnap_se: 0.03,
            permutation_p_value: 0.04,
            survival_level: 0.4,
            conditional_null_p05: 0.02,
            conditional_null_p50: 0.1,
            conditional_null_p95: 0.2,
            above_conditional_null: true,
            calibrated_supported_cnap_is_extrapolation: false,
        })
    }

    #[test]
    fn successful_rows_serialize_as_complete_json_objects() {
        let value = serde_json::to_value(successful_metric()).unwrap();
        assert_eq!(value["status"], "ok");
        assert_eq!(value["supported_cnap"], 0.45);
        assert_eq!(value["n_skipped_score"], 1);
        assert!(value.get("error").is_none());
        assert!(
            value
                .as_object()
                .unwrap()
                .values()
                .all(|value| !value.is_null())
        );
    }

    #[test]
    fn error_rows_omit_unavailable_values() {
        let metric = MetricResult::Failure(MetricFailure {
            context: MetricContext {
                score_col: "score_a".to_string(),
                n: 0,
                n_positive: 0,
                n_negative: 0,
                skipped_score: 0,
                prevalence_mode: "observed".to_string(),
            },
            status: "error",
            reference_prevalence: None,
            error: "missing positive class".to_string(),
        });
        let paired = PairedResult::Failure(PairedFailure {
            context: PairedContext {
                model_a: "score_a".to_string(),
                model_b: "score_b".to_string(),
                observed_positive_count: 4,
                observed_negative_count: 16,
                replication_positive_count: 4,
                replication_negative_count: 16,
                prevalence_mode: "observed".to_string(),
            },
            status: "error",
            reference_prevalence: None,
            error: "models use different rows".to_string(),
        });

        let metric = serde_json::to_value(metric).unwrap();
        let paired = serde_json::to_value(paired).unwrap();
        assert_eq!(metric["status"], "error");
        assert!(metric.get("reference_prevalence").is_none());
        assert!(metric.get("supported_cnap").is_none());
        assert!(paired.get("estimated_supported_superiority").is_none());
    }

    #[test]
    fn schema_eight_report_is_valid_and_modes_are_explicit() {
        let args = Args {
            input: PathBuf::from("predictions.csv"),
            output: PathBuf::from("metrics.json"),
            label_col: "label".to_string(),
            score_cols: vec!["score_a".to_string()],
            score_prefix: None,
            prevalences: vec![PrevalenceSpec::Fixed(0.2)],
            paired_baseline: None,
            replication_positive_count: None,
            replication_negative_count: None,
            bootstrap_replicates: 100,
            permutations: 50,
            support_order: 2,
            survival_frequency: 0.5,
            bootstrap_seed: 1,
            permutation_seed: 2,
            threads: None,
            common_rows: false,
            fail_on_error: false,
        };
        let extraction = Extraction {
            columns: Vec::new(),
            rows_read: 20,
            skipped_label: 1,
            shared_rows: None,
            skipped_any_score: None,
        };
        let rendered = render_json(&args, &extraction, &[successful_metric()], &[], 0).unwrap();
        let report: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(SCHEMA_VERSION, 8);
        assert_eq!(report["schema_version"], 8);
        assert_eq!(report["thread_policy"]["mode"], "auto");
        assert_eq!(report["row_mode"], "per_model");
        assert!(report["rows"].get("retained_shared").is_none());
        assert!(!rendered.contains(": null"));
    }

    #[test]
    fn non_finite_computed_values_are_explicit_errors() {
        assert!(finite_result("metric", f64::NAN).is_err());
        assert!(finite_result("metric", f64::INFINITY).is_err());
        assert_eq!(finite_result("metric", 0.5).unwrap(), 0.5);
    }
}
