//! Endpoint-local diagnostic based on the two strongest HLA-specific maxima.
//!
//! This is deliberately separate from the frozen production selector. Every
//! score is a deterministic function of one endpoint's real candidate scores
//! and HLA profile; no cohort-level statistic enters the aggregation.

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
use select_adaptive_hillq::numeric::{PowerOrder, log_power_mean};

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
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0,0.25,0.5,1,2,4,8,inf")]
    alpha_values: Vec<String>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.05,0.1,0.2,0.3,0.4,0.5")]
    second_hla_weights: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.005,0.01,0.02,0.05,0.1,0.2,0.5")]
    agreement_bonuses: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.1,0.2,0.5,1,2")]
    agreement_scales: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.25,0.5,1,2,4")]
    hinge_distances: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.02,0.05,0.1,0.2,0.5,1")]
    hinge_weights: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "-8,-7.5,-7,-6.5,-6,-5.5,-5,-4.5,-4,-3.5,-3,-2.5,-2,-1.5,-1,-0.5,0", allow_hyphen_values = true)]
    second_hla_thresholds: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.02,0.1,0.4,1")]
    second_hla_widths: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.005,0.01,0.02,0.05,0.1,0.2,0.5,1")]
    second_hla_bonuses: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0")]
    length_threshold_slopes: Vec<f64>,
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
    rows: Vec<Row>,
}

#[derive(Debug, Serialize)]
struct Row {
    rule: &'static str,
    cohort: String,
    parameter: String,
    outcome: CnapOutcome,
}

fn top_two_hla_maxima(scores: &[f64], hlas: &[String]) -> Result<[f64; 2]> {
    if scores.len() != hlas.len() || scores.is_empty() {
        return Err(SelectionError::msg(
            "invalid candidate/HLA roster for top-two aggregation",
        ));
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
    if ordered.len() < 2 {
        return Err(SelectionError::msg(
            "top-two HLA aggregation requires at least two distinct HLAs",
        ));
    }
    Ok([ordered[0], ordered[1]])
}

fn run() -> Result<()> {
    let args = Args::parse();
    if args.selection_replications == 0
        || args
            .second_hla_weights
            .iter()
            .any(|&value| !value.is_finite() || !(0.0..=1.0).contains(&value))
        || args
            .agreement_bonuses
            .iter()
            .chain(&args.agreement_scales)
            .chain(&args.hinge_distances)
            .chain(&args.hinge_weights)
            .chain(&args.second_hla_widths)
            .chain(&args.second_hla_bonuses)
            .any(|&value| !value.is_finite() || value <= 0.0)
        || args
            .second_hla_thresholds
            .iter()
            .any(|value| !value.is_finite())
        || args
            .length_threshold_slopes
            .iter()
            .any(|&value| !value.is_finite() || value < 0.0)
    {
        return Err(SelectionError::msg("invalid top-two HLA grid"));
    }
    let powers: Result<Vec<PowerOrder>> = args
        .alpha_values
        .iter()
        .map(|value| PowerOrder::parse(value))
        .collect();
    let powers = powers?;

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
        let top_two: Result<Vec<[f64; 2]>> = task
            .raw_candidates
            .iter()
            .zip(&task.raw_candidate_hlas)
            .map(|(scores, hlas)| top_two_hla_maxima(scores, hlas))
            .collect();
        let top_two = top_two?;

        for &power in &powers {
            let scores: Result<Vec<f64>> = top_two
                .iter()
                .map(|pair| log_power_mean(pair, power))
                .collect();
            let (paired, observed) =
                observed_gate(&scores?, &reference_scores, &labels_bool, &contract)?;
            let outcome = if args.observed_only {
                observed
            } else {
                complete_forward_assessment(&paired, observed, &contract, seed)?
            };
            rows.push(Row {
                rule: "top_two_hla_power_mean",
                cohort: cohort.to_string(),
                parameter: match power {
                    PowerOrder::Infinity => "alpha=inf".to_string(),
                    PowerOrder::Finite(value) => format!("alpha={value}"),
                },
                outcome,
            });
        }

        for &weight in &args.second_hla_weights {
            let scores: Vec<f64> = top_two
                .iter()
                .map(|pair| (1.0 - weight) * pair[0] + weight * pair[1])
                .collect();
            let (paired, observed) =
                observed_gate(&scores, &reference_scores, &labels_bool, &contract)?;
            let outcome = if args.observed_only {
                observed
            } else {
                complete_forward_assessment(&paired, observed, &contract, seed)?
            };
            rows.push(Row {
                rule: "top_two_hla_log_score_weighted_mean",
                cohort: cohort.to_string(),
                parameter: format!("second_hla_weight={weight}"),
                outcome,
            });
        }

        for &bonus in &args.agreement_bonuses {
            for &scale in &args.agreement_scales {
                let scores: Vec<f64> = top_two
                    .iter()
                    .map(|pair| pair[0] + bonus * (-(pair[0] - pair[1]) / scale).exp())
                    .collect();
                let (paired, observed) =
                    observed_gate(&scores, &reference_scores, &labels_bool, &contract)?;
                let outcome = if args.observed_only {
                    observed
                } else {
                    complete_forward_assessment(&paired, observed, &contract, seed)?
                };
                rows.push(Row {
                    rule: "top_two_hla_exponential_agreement_bonus",
                    cohort: cohort.to_string(),
                    parameter: format!("bonus={bonus};scale={scale}"),
                    outcome,
                });
            }
        }

        for &distance in &args.hinge_distances {
            for &weight in &args.hinge_weights {
                let scores: Vec<f64> = top_two
                    .iter()
                    .map(|pair| pair[0] + weight * (distance - (pair[0] - pair[1])).max(0.0))
                    .collect();
                let (paired, observed) =
                    observed_gate(&scores, &reference_scores, &labels_bool, &contract)?;
                let outcome = if args.observed_only {
                    observed
                } else {
                    complete_forward_assessment(&paired, observed, &contract, seed)?
                };
                rows.push(Row {
                    rule: "top_two_hla_hinge_agreement_bonus",
                    cohort: cohort.to_string(),
                    parameter: format!("distance={distance};weight={weight}"),
                    outcome,
                });
            }
        }

        for &threshold in &args.second_hla_thresholds {
            for &width in &args.second_hla_widths {
                for &bonus in &args.second_hla_bonuses {
                    let scores: Vec<f64> = top_two
                        .iter()
                        .map(|pair| {
                            pair[0]
                                + bonus
                                    * select_adaptive_hillq::numeric::expit(
                                        (pair[1] - threshold) / width,
                                    )
                        })
                        .collect();
                    let (paired, observed) =
                        observed_gate(&scores, &reference_scores, &labels_bool, &contract)?;
                    let outcome = if args.observed_only {
                        observed
                    } else {
                        complete_forward_assessment(&paired, observed, &contract, seed)?
                    };
                    rows.push(Row {
                        rule: "second_hla_thresholded_bounded_bonus",
                        cohort: cohort.to_string(),
                        parameter: format!("threshold={threshold};width={width};bonus={bonus}"),
                        outcome,
                    });
                }
            }
        }

        for &base_threshold in &args.second_hla_thresholds {
            for &slope in &args.length_threshold_slopes {
                for &width in &args.second_hla_widths {
                    for &bonus in &args.second_hla_bonuses {
                        let scores: Vec<f64> = top_two
                            .iter()
                            .zip(&task.endpoints)
                            .map(|(pair, endpoint)| {
                                let length_excess =
                                    endpoint.long_peptide.chars().count().saturating_sub(15) as f64;
                                let threshold = base_threshold + slope * length_excess;
                                pair[0]
                                    + bonus
                                        * select_adaptive_hillq::numeric::expit(
                                            (pair[1] - threshold) / width,
                                        )
                            })
                            .collect();
                        let (paired, observed) =
                            observed_gate(&scores, &reference_scores, &labels_bool, &contract)?;
                        let outcome = if args.observed_only {
                            observed
                        } else {
                            complete_forward_assessment(&paired, observed, &contract, seed)?
                        };
                        rows.push(Row {
                            rule: "length_adjusted_second_hla_bounded_bonus",
                            cohort: cohort.to_string(),
                            parameter: format!(
                                "base_threshold={base_threshold};slope={slope};width={width};bonus={bonus}"
                            ),
                            outcome,
                        });
                    }
                }
            }
        }
    }

    let output = Output {
        schema_version: 1,
        analysis: "post_result_full_hla_top_two_hla_endpoint_local_rule",
        model: "full_hla",
        selection_replications: args.selection_replications,
        observed_only: args.observed_only,
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
    use super::top_two_hla_maxima;

    #[test]
    fn top_two_are_maxima_from_distinct_hlas() {
        let scores = [-4.0, -1.0, -2.0, -3.0, -1.5];
        let hlas = ["A", "A", "B", "B", "C"].map(String::from);
        assert_eq!(top_two_hla_maxima(&scores, &hlas).unwrap(), [-1.0, -1.5]);
    }
}
