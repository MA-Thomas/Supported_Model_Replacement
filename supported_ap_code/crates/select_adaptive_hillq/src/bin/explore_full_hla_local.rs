//! Post-result diagnostic for endpoint-local, HLA-aware full-HLA adaptive rules.
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
use select_adaptive_hillq::numeric::{HillOrder, PowerOrder, adaptive_components, expit};

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    source_root: PathBuf,
    #[arg(long)]
    bundle_root: PathBuf,
    #[arg(long, default_value_t = 600)]
    selection_replications: usize,
    #[arg(long, value_delimiter = ' ', num_args = 1.., default_value = "all winner")]
    offer_types: Vec<String>,
    #[arg(long, value_delimiter = ' ', num_args = 1.., default_value = "1.75 2 2.25 2.5")]
    hla_gap_thresholds: Vec<f64>,
    #[arg(long, value_delimiter = ' ', num_args = 1.., default_value = "0.02 0.05 0.1 0.2")]
    gate_widths: Vec<f64>,
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
    hill_q: &'static str,
    offer_type: String,
    hla_gap_threshold: f64,
    gate_width: f64,
    outcome: CnapOutcome,
}

fn hla_gap_and_winner_rosters(
    rosters: &[Vec<f64>],
    hla_rosters: &[Vec<String>],
) -> Result<(Vec<f64>, Vec<Vec<f64>>)> {
    if rosters.len() != hla_rosters.len() {
        return Err(SelectionError::msg(
            "endpoint candidate/HLA roster mismatch",
        ));
    }
    let mut gaps = Vec::with_capacity(rosters.len());
    let mut winners = Vec::with_capacity(rosters.len());
    for (scores, hlas) in rosters.iter().zip(hla_rosters) {
        if scores.len() != hlas.len() {
            return Err(SelectionError::msg("candidate/HLA alignment mismatch"));
        }
        let mut by_hla: HashMap<&str, Vec<f64>> = HashMap::new();
        for (&score, hla) in scores.iter().zip(hlas) {
            by_hla.entry(hla).or_default().push(score);
        }
        let mut maxima: Vec<(&str, f64)> = by_hla
            .iter()
            .map(|(&hla, values)| {
                (
                    hla,
                    values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                )
            })
            .collect();
        maxima.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
        if maxima.len() < 2 {
            gaps.push(0.0);
            winners.push(scores.clone());
        } else {
            gaps.push(maxima[0].1 - maxima[1].1);
            winners.push(by_hla[maxima[0].0].clone());
        }
    }
    Ok((gaps, winners))
}

fn run() -> Result<()> {
    let args = Args::parse();
    if args.selection_replications == 0
        || args
            .hla_gap_thresholds
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        || args
            .gate_widths
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        || args
            .offer_types
            .iter()
            .any(|value| value != "all" && value != "winner")
    {
        return Err(SelectionError::msg("invalid exploratory local-rule grid"));
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

        let (anchors, all_offers) = adaptive_components(
            &task.raw_candidates,
            PowerOrder::Infinity,
            HillOrder::Infinity,
        )?;
        let (hla_gaps, winner_rosters) =
            hla_gap_and_winner_rosters(&task.raw_candidates, &task.raw_candidate_hlas)?;
        let (_, winner_offers) =
            adaptive_components(&winner_rosters, PowerOrder::Infinity, HillOrder::Infinity)?;

        for offer_type in &args.offer_types {
            let offers = if offer_type == "all" {
                &all_offers
            } else {
                &winner_offers
            };
            for &threshold in &args.hla_gap_thresholds {
                for &gate_width in &args.gate_widths {
                    let scores: Vec<f64> = anchors
                        .iter()
                        .zip(offers)
                        .zip(&hla_gaps)
                        .map(|((&anchor, &offer), &gap)| {
                            anchor + offer * expit((gap - threshold) / gate_width)
                        })
                        .collect();
                    let (paired, observed) =
                        observed_gate(&scores, &reference_scores, &labels_bool, &contract)?;
                    let outcome = complete_forward_assessment(&paired, observed, &contract, seed)?;
                    rows.push(Row {
                        rule: "endpoint_local_hla_dominance_gated_corroboration",
                        cohort: cohort.to_string(),
                        alpha: "inf",
                        hill_q: "inf",
                        offer_type: offer_type.clone(),
                        hla_gap_threshold: threshold,
                        gate_width,
                        outcome,
                    });
                }
            }
        }
    }

    let output = Output {
        schema_version: 1,
        analysis: "post_result_full_hla_endpoint_local_hla_dominance_rule",
        model: "full_hla",
        selection_replications: args.selection_replications,
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
    use super::hla_gap_and_winner_rosters;

    #[test]
    fn hla_gap_uses_allele_specific_maxima() {
        let scores = vec![vec![-4.0, -1.0, -3.0, -3.5]];
        let hlas = vec![vec!["A".into(), "A".into(), "B".into(), "B".into()]];
        let (gaps, winners) = hla_gap_and_winner_rosters(&scores, &hlas).unwrap();
        assert_eq!(gaps, vec![2.0]);
        assert_eq!(winners, vec![vec![-4.0, -1.0]]);
    }
}
