//! Post-result diagnostic for endpoint-local opportunity-centered full-HLA rules.
//!
//! This binary is deliberately separate from the frozen production selector.

use std::collections::HashMap;
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
    #[arg(long, default_value_t = 600)]
    selection_replications: usize,
    #[arg(long, default_value_t = false)]
    observed_only: bool,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "mean,median,q75,q90,q95,hla_max_median,hla_second_max")]
    center_types: Vec<String>,
    #[arg(long, value_delimiter = ',', num_args = 1.., allow_hyphen_values = true)]
    offsets: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.02,0.057707996236288556,0.1665106415,0.4")]
    kappas: Vec<f64>,
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Serialize)]
struct Output {
    schema_version: u32,
    analysis: &'static str,
    model: &'static str,
    selection_replications: usize,
    observed_only: bool,
    source_root: String,
    bundle_root: String,
    rows: Vec<Row>,
}

#[derive(Debug, Serialize)]
struct Row {
    rule: &'static str,
    cohort: String,
    alpha: &'static str,
    hill_q: &'static str,
    center_type: String,
    offset: f64,
    kappa: f64,
    minimum_endpoint_c: f64,
    median_endpoint_c: f64,
    maximum_endpoint_c: f64,
    outcome: CnapOutcome,
}

fn quantile(values: &[f64], probability: f64) -> Result<f64> {
    if values.is_empty() {
        return Err(SelectionError::msg(
            "cannot center an empty candidate roster",
        ));
    }
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    let position = (ordered.len() - 1) as f64 * probability;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    let weight = position - lower as f64;
    Ok(ordered[lower] * (1.0 - weight) + ordered[upper] * weight)
}

fn mean(values: &[f64]) -> Result<f64> {
    if values.is_empty() {
        return Err(SelectionError::msg(
            "cannot center an empty candidate roster",
        ));
    }
    Ok(values.iter().sum::<f64>() / values.len() as f64)
}

fn endpoint_centers(
    center_type: &str,
    rosters: &[Vec<f64>],
    hla_rosters: &[Vec<String>],
) -> Result<Vec<f64>> {
    if rosters.len() != hla_rosters.len() {
        return Err(SelectionError::msg(
            "endpoint candidate/HLA roster mismatch",
        ));
    }
    rosters
        .iter()
        .zip(hla_rosters)
        .map(|(scores, hlas)| match center_type {
            "mean" => mean(scores),
            "median" => quantile(scores, 0.5),
            "q75" => quantile(scores, 0.75),
            "q90" => quantile(scores, 0.90),
            "q95" => quantile(scores, 0.95),
            "hla_max_median" => {
                if scores.len() != hlas.len() {
                    return Err(SelectionError::msg("candidate/HLA alignment mismatch"));
                }
                let mut maxima: HashMap<&str, f64> = HashMap::new();
                for (&score, hla) in scores.iter().zip(hlas) {
                    maxima
                        .entry(hla)
                        .and_modify(|value| *value = value.max(score))
                        .or_insert(score);
                }
                quantile(&maxima.into_values().collect::<Vec<_>>(), 0.5)
            }
            "hla_second_max" => {
                if scores.len() != hlas.len() {
                    return Err(SelectionError::msg("candidate/HLA alignment mismatch"));
                }
                let mut maxima: HashMap<&str, f64> = HashMap::new();
                for (&score, hla) in scores.iter().zip(hlas) {
                    maxima
                        .entry(hla)
                        .and_modify(|value| *value = value.max(score))
                        .or_insert(score);
                }
                let mut ordered: Vec<f64> = maxima.into_values().collect();
                ordered.sort_by(|a, b| b.total_cmp(a));
                ordered.get(1).copied().ok_or_else(|| {
                    SelectionError::msg("second-HLA center requires two distinct HLAs")
                })
            }
            _ => Err(SelectionError::msg(format!(
                "unknown center type: {center_type}"
            ))),
        })
        .collect()
}

fn run() -> Result<()> {
    let args = Args::parse();
    if args.selection_replications == 0
        || args.offsets.is_empty()
        || args.offsets.iter().any(|value| !value.is_finite())
        || args
            .kappas
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        || args.center_types.iter().any(|value| {
            value != "mean"
                && value != "median"
                && value != "q75"
                && value != "q90"
                && value != "q95"
                && value != "hla_max_median"
                && value != "hla_second_max"
        })
    {
        return Err(SelectionError::msg(
            "invalid exploratory endpoint-center grid",
        ));
    }

    let bundle = load_bundle(&args.bundle_root.join("pr"))
        .map_err(|error| SelectionError::msg(format!("failed to load PR bundle: {error}")))?;
    let contract =
        CnapContract::from_pr_bundle_with_replications(&bundle, args.selection_replications)?;
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
        let (anchors, offers) = adaptive_components(
            &task.raw_candidates,
            PowerOrder::Infinity,
            HillOrder::Infinity,
        )?;

        for center_type in &args.center_types {
            let centers =
                endpoint_centers(center_type, &task.raw_candidates, &task.raw_candidate_hlas)?;
            for &offset in &args.offsets {
                let endpoint_cs: Vec<f64> = centers.iter().map(|center| center + offset).collect();
                let minimum_endpoint_c = endpoint_cs.iter().copied().fold(f64::INFINITY, f64::min);
                let maximum_endpoint_c = endpoint_cs
                    .iter()
                    .copied()
                    .fold(f64::NEG_INFINITY, f64::max);
                let median_endpoint_c = quantile(&endpoint_cs, 0.5)?;
                for &kappa in &args.kappas {
                    let scores: Result<Vec<f64>> = anchors
                        .iter()
                        .zip(&offers)
                        .zip(&endpoint_cs)
                        .map(|((&anchor, &offer), &c)| {
                            self_gated_score(anchor, offer, c, kappa, 1e-10, 64)
                        })
                        .collect();
                    let scores = scores?;
                    let (paired, observed) =
                        observed_gate(&scores, &reference_scores, &labels_bool, &contract)?;
                    let outcome = if args.observed_only {
                        observed
                    } else {
                        complete_forward_assessment(&paired, observed, &contract, seed)?
                    };
                    rows.push(Row {
                        rule: "endpoint_local_opportunity_centered_self_gate",
                        cohort: cohort.to_string(),
                        alpha: "inf",
                        hill_q: "inf",
                        center_type: center_type.clone(),
                        offset,
                        kappa,
                        minimum_endpoint_c,
                        median_endpoint_c,
                        maximum_endpoint_c,
                        outcome,
                    });
                }
            }
        }
    }

    let output = Output {
        schema_version: 1,
        analysis: "post_result_full_hla_endpoint_local_opportunity_centered_rule",
        model: "full_hla",
        selection_replications: args.selection_replications,
        observed_only: args.observed_only,
        source_root: args.source_root.display().to_string(),
        bundle_root: args.bundle_root.display().to_string(),
        rows,
    };
    let text = serde_json::to_string_pretty(&output)
        .map_err(|error| SelectionError::msg(format!("failed to serialize JSON: {error}")))?;
    if let Some(parent) = args.output.parent() {
        std::fs::create_dir_all(parent).map_err(|source| SelectionError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(&args.output, format!("{text}\n")).map_err(|source| SelectionError::Io {
        path: args.output.clone(),
        source,
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
    use super::{endpoint_centers, quantile};

    #[test]
    fn quantile_is_linearly_interpolated() {
        assert_eq!(quantile(&[4.0, 1.0, 3.0, 2.0], 0.5).unwrap(), 2.5);
    }

    #[test]
    fn hla_center_collapses_each_allele_to_its_maximum() {
        let scores = vec![vec![-4.0, -1.0, -3.0, -3.5]];
        let hlas = vec![vec!["A".into(), "A".into(), "B".into(), "B".into()]];
        assert_eq!(
            endpoint_centers("hla_max_median", &scores, &hlas).unwrap(),
            vec![-2.0]
        );
    }
}
