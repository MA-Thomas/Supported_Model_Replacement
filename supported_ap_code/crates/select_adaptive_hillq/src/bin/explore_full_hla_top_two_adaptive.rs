//! Adaptive endpoint-local aggregation restricted to the two strongest HLAs.
//!
//! The maximum remains the anchor. Corroboration is calculated either from
//! the two HLA-specific maxima or from real candidates belonging to those two
//! HLAs. No other endpoint or probabilistic construction enters the score.

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
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.5,1,2,4,8,inf")]
    alpha_values: Vec<String>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "hla_maxima,top_two_hla_candidates")]
    offer_rosters: Vec<String>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.5,1,2,inf")]
    q_values: Vec<String>,
    #[arg(long, value_delimiter = ',', num_args = 1..)]
    c_values: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.02,0.057707996236288556,0.1665106415,0.4804497736,1.3862896863")]
    kappas: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "absolute,second_hla_plus")]
    center_modes: Vec<String>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "1")]
    center_scales: Vec<f64>,
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Serialize)]
struct Output {
    schema_version: u32,
    analysis: &'static str,
    selection_replications: usize,
    observed_only: bool,
    rows: Vec<Row>,
}

#[derive(Debug, Serialize)]
struct Row {
    rule: &'static str,
    cohort: String,
    offer_roster: String,
    alpha: String,
    q: String,
    center_mode: String,
    center_scale: f64,
    c_parameter: f64,
    kappa: f64,
    outcome: CnapOutcome,
}

struct TopTwoRosters {
    maxima: Vec<Vec<f64>>,
    candidates: Vec<Vec<f64>>,
    second_maxima: Vec<f64>,
    first_maxima: Vec<f64>,
}

fn top_two_rosters(
    score_rosters: &[Vec<f64>],
    hla_rosters: &[Vec<String>],
) -> Result<TopTwoRosters> {
    if score_rosters.len() != hla_rosters.len() {
        return Err(SelectionError::msg(
            "endpoint candidate/HLA roster mismatch",
        ));
    }
    let mut maxima_rosters = Vec::with_capacity(score_rosters.len());
    let mut candidate_rosters = Vec::with_capacity(score_rosters.len());
    let mut second_maxima = Vec::with_capacity(score_rosters.len());
    let mut first_maxima = Vec::with_capacity(score_rosters.len());
    for (scores, hlas) in score_rosters.iter().zip(hla_rosters) {
        if scores.len() != hlas.len() || scores.is_empty() {
            return Err(SelectionError::msg("invalid candidate/HLA roster"));
        }
        let mut maxima: HashMap<&str, f64> = HashMap::new();
        for (&score, hla) in scores.iter().zip(hlas) {
            maxima
                .entry(hla)
                .and_modify(|value| *value = value.max(score))
                .or_insert(score);
        }
        let mut ordered: Vec<(&str, f64)> = maxima.into_iter().collect();
        ordered.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
        if ordered.len() < 2 {
            return Err(SelectionError::msg(
                "top-two aggregation requires two distinct HLAs",
            ));
        }
        let first = ordered[0];
        let second = ordered[1];
        maxima_rosters.push(vec![first.1, second.1]);
        candidate_rosters.push(
            scores
                .iter()
                .zip(hlas)
                .filter_map(|(&score, hla)| (hla == first.0 || hla == second.0).then_some(score))
                .collect(),
        );
        second_maxima.push(second.1);
        first_maxima.push(first.1);
    }
    Ok(TopTwoRosters {
        maxima: maxima_rosters,
        candidates: candidate_rosters,
        second_maxima,
        first_maxima,
    })
}

fn q_name(q: HillOrder) -> String {
    match q {
        HillOrder::Infinity => "inf".to_string(),
        HillOrder::Finite(value) => value.to_string(),
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    if args.selection_replications == 0
        || args.c_values.is_empty()
        || args.c_values.iter().any(|value| !value.is_finite())
        || args
            .kappas
            .iter()
            .any(|&value| !value.is_finite() || value <= 0.0)
        || args.offer_rosters.iter().any(|value| {
            value != "hla_maxima" && value != "top_two_hla_candidates" && value != "all_candidates"
        })
        || args.center_modes.iter().any(|value| {
            value != "absolute"
                && value != "second_hla_plus"
                && value != "second_hla_affine"
                && value != "top_two_mean_plus"
        })
        || args.center_scales.iter().any(|value| !value.is_finite())
    {
        return Err(SelectionError::msg("invalid adaptive top-two HLA grid"));
    }
    let hills: Result<Vec<HillOrder>> = args
        .q_values
        .iter()
        .map(|value| HillOrder::parse(value))
        .collect();
    let hills = hills?;
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
        let top_two = top_two_rosters(&task.raw_candidates, &task.raw_candidate_hlas)?;

        for offer_roster in &args.offer_rosters {
            let source = match offer_roster.as_str() {
                "hla_maxima" => &top_two.maxima,
                "top_two_hla_candidates" => &top_two.candidates,
                "all_candidates" => &task.raw_candidates,
                _ => unreachable!(),
            };
            for &power in &powers {
                for &hill in &hills {
                    let (anchors, offers) = adaptive_components(source, power, hill)?;
                    for center_mode in &args.center_modes {
                        for &center_scale in &args.center_scales {
                            for &c_parameter in &args.c_values {
                                for &kappa in &args.kappas {
                                    let scores: Result<Vec<f64>> = anchors
                                        .iter()
                                        .zip(&offers)
                                        .zip(&top_two.first_maxima)
                                        .zip(&top_two.second_maxima)
                                        .map(|(((&anchor, &offer), &first), &second)| {
                                            let c = match center_mode.as_str() {
                                                "absolute" => c_parameter,
                                                "second_hla_plus" => second + c_parameter,
                                                "second_hla_affine" => {
                                                    center_scale * second + c_parameter
                                                }
                                                "top_two_mean_plus" => {
                                                    (first + second) / 2.0 + c_parameter
                                                }
                                                _ => unreachable!(),
                                            };
                                            self_gated_score(anchor, offer, c, kappa, 1e-10, 64)
                                        })
                                        .collect();
                                    let (paired, observed) = observed_gate(
                                        &scores?,
                                        &reference_scores,
                                        &labels_bool,
                                        &contract,
                                    )?;
                                    let outcome = if args.observed_only {
                                        observed
                                    } else {
                                        complete_forward_assessment(
                                            &paired, observed, &contract, seed,
                                        )?
                                    };
                                    rows.push(Row {
                                        rule: "adaptive_top_two_hla",
                                        cohort: cohort.to_string(),
                                        offer_roster: offer_roster.clone(),
                                        alpha: match power {
                                            PowerOrder::Infinity => "inf".to_string(),
                                            PowerOrder::Finite(value) => value.to_string(),
                                        },
                                        q: q_name(hill),
                                        center_mode: center_mode.clone(),
                                        center_scale,
                                        c_parameter,
                                        kappa,
                                        outcome,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let output = Output {
        schema_version: 1,
        analysis: "post_result_full_hla_adaptive_top_two_hla_rule",
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
    use super::top_two_rosters;

    #[test]
    fn keeps_only_candidates_from_the_two_strongest_hlas() {
        let scores = vec![vec![-4.0, -1.0, -2.0, -3.0, -1.5]];
        let hlas = vec![["A", "A", "B", "B", "C"].map(String::from).to_vec()];
        let result = top_two_rosters(&scores, &hlas).unwrap();
        assert_eq!(result.maxima, vec![vec![-1.0, -1.5]]);
        assert_eq!(result.candidates, vec![vec![-4.0, -1.0, -1.5]]);
        assert_eq!(result.second_maxima, vec![-1.5]);
        assert_eq!(result.first_maxima, vec![-1.0]);
    }
}
