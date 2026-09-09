//! Post-result diagnostic for label-blind full-HLA adaptive rules.
//!
//! This binary is intentionally separate from the frozen production selector.
//! It evaluates a small family of location-normalized `alpha=inf` gates against
//! the same model-matched maximum and the same staged paired-CNAP contract.

use std::path::PathBuf;

use clap::Parser;
use directed_round_robin_organizer::input::load_bundle;
use serde::Serialize;

use select_adaptive_hillq::cnap::{
    CnapContract, CnapOutcome, aligned_maximum_reference, complete_forward_assessment,
    observed_gate,
};
use select_adaptive_hillq::config::SELECTION_COHORTS;
use select_adaptive_hillq::data::{authoritative_endpoint_table, load_task, validate_task_labels};
use select_adaptive_hillq::error::{Result, SelectionError};
use select_adaptive_hillq::numeric::{
    HillOrder, PowerOrder, adaptive_components, self_gated_score,
};

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    source_root: PathBuf,
    #[arg(long)]
    bundle_root: PathBuf,
    #[arg(long, default_value_t = 200)]
    selection_replications: usize,
    #[arg(long, value_delimiter = ' ', num_args = 1.., default_value = "inf")]
    q_values: Vec<String>,
    #[arg(long, value_delimiter = ' ', num_args = 1.., default_value = "0.38")]
    location_quantiles: Vec<f64>,
    #[arg(long, value_delimiter = ' ', num_args = 1.., default_value = "0")]
    delta_values: Vec<f64>,
    #[arg(
        long,
        value_delimiter = ' ',
        num_args = 1..,
        default_value = "0.057707996236288556"
    )]
    kappa_values: Vec<f64>,
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Serialize)]
struct Output {
    schema_version: u32,
    analysis: &'static str,
    model: &'static str,
    selection_replications: usize,
    source_root: String,
    bundle_root: String,
    rows: Vec<Row>,
}

#[derive(Debug, Serialize)]
struct Row {
    rule: &'static str,
    cohort: String,
    alpha: &'static str,
    q: String,
    location_quantile: f64,
    location: f64,
    delta: f64,
    c: f64,
    kappa: f64,
    outcome: CnapOutcome,
}

fn quantile(mut values: Vec<f64>, probability: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    let position = (values.len() - 1) as f64 * probability;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    let weight = position - lower as f64;
    values[lower] * (1.0 - weight) + values[upper] * weight
}

fn run() -> Result<()> {
    let args = Args::parse();
    if args.selection_replications == 0
        || args
            .location_quantiles
            .iter()
            .any(|&value| !(0.0..=1.0).contains(&value) || !value.is_finite())
        || args.delta_values.iter().any(|value| !value.is_finite())
        || args
            .kappa_values
            .iter()
            .any(|&value| !value.is_finite() || value <= 0.0)
    {
        return Err(SelectionError::msg("invalid exploratory rule grid"));
    }
    let bundle = load_bundle(&args.bundle_root.join("pr"))
        .map_err(|error| SelectionError::msg(format!("failed to load PR bundle: {error}")))?;
    let contract =
        CnapContract::from_pr_bundle_with_replications(&bundle, args.selection_replications)?;

    let hills: Result<Vec<HillOrder>> = args
        .q_values
        .iter()
        .map(|value| HillOrder::parse(value))
        .collect();
    let hills = hills?;
    let mut rows = Vec::new();

    for cohort in SELECTION_COHORTS {
        let (task, _) = load_task(&args.source_root, cohort, "full_hla", "pr")?;
        let authority = authoritative_endpoint_table(&args.bundle_root, cohort)?;
        let endpoint_ids = validate_task_labels(&task, &authority)?;
        let labels = task.labels();
        let labels_bool: Vec<bool> = labels.iter().map(|&label| label == 1).collect();
        let (_, reference_scores, _) =
            aligned_maximum_reference(&bundle, cohort, "full_hla", &endpoint_ids, &labels)?;
        let seed = contract.selection_seed(cohort)?;

        for &hill in &hills {
            let (anchors, offers) =
                adaptive_components(&task.raw_candidates, PowerOrder::Infinity, hill)?;
            for &location_quantile in &args.location_quantiles {
                let location = quantile(anchors.clone(), location_quantile);
                for &delta in &args.delta_values {
                    let c = location + delta;
                    for &kappa in &args.kappa_values {
                        let scores: Result<Vec<f64>> = anchors
                            .iter()
                            .zip(&offers)
                            .map(|(&anchor, &offer)| {
                                self_gated_score(anchor, offer, c, kappa, 1e-10, 64)
                            })
                            .collect();
                        let scores = scores?;
                        let (paired, observed) =
                            observed_gate(&scores, &reference_scores, &labels_bool, &contract)?;
                        let outcome =
                            complete_forward_assessment(&paired, observed, &contract, seed)?;
                        rows.push(Row {
                            rule: "max_distribution_quantile_plus_delta",
                            cohort: cohort.to_string(),
                            alpha: "inf",
                            q: if matches!(hill, HillOrder::Infinity) {
                                "inf".to_string()
                            } else {
                                "2".to_string()
                            },
                            location_quantile,
                            location,
                            delta,
                            c,
                            kappa,
                            outcome,
                        });
                    }
                }
            }
        }
    }

    let output = Output {
        schema_version: 1,
        analysis: "post_result_full_hla_quantile_gated_adaptive_rule",
        model: "full_hla",
        selection_replications: args.selection_replications,
        source_root: args.source_root.display().to_string(),
        bundle_root: args.bundle_root.display().to_string(),
        rows,
    };
    let text = serde_json::to_string_pretty(&output)
        .map_err(|error| SelectionError::msg(format!("failed to serialize JSON: {error}")))?;
    if let Some(parent) = args.output.parent() {
        std::fs::create_dir_all(parent).map_err(|error| SelectionError::Io {
            path: parent.to_path_buf(),
            source: error,
        })?;
    }
    std::fs::write(&args.output, format!("{text}\n")).map_err(|error| SelectionError::Io {
        path: args.output.clone(),
        source: error,
    })?;
    eprintln!(
        "wrote {} evaluations to {}",
        output.rows.len(),
        args.output.display()
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::quantile;

    #[test]
    fn quantile_uses_linear_order_statistic_interpolation() {
        let values = vec![4.0, 1.0, 3.0, 2.0];
        assert_eq!(quantile(values.clone(), 0.0), 1.0);
        assert_eq!(quantile(values.clone(), 0.5), 2.5);
        assert_eq!(quantile(values, 1.0), 4.0);
    }
}
