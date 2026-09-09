//! Self-gated full-HLA aggregation with an endpoint-intrinsic gate center.
//!
//! The center is a generalized mean of the current long peptide's real score
//! landscape: all candidates, one maximum per HLA, one maximum per short
//! peptide, or the second-HLA score for each short peptide. No cohort-level
//! distribution or external peptide enters the score.

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
    HillOrder, PowerOrder, adaptive_components, log_power_mean, self_gated_score,
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
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "raw,hla_maxima,peptide_maxima,peptide_second_hla")]
    center_rosters: Vec<String>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0,0.5,1,2,4,8,inf")]
    center_alphas: Vec<String>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "2,inf")]
    hill_qs: Vec<String>,
    #[arg(long, value_delimiter = ',', num_args = 1.., allow_hyphen_values = true)]
    offsets: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.02,0.057707996236288556,0.1665106415")]
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
    rows: Vec<Row>,
}

#[derive(Debug, Serialize)]
struct Row {
    rule: &'static str,
    cohort: String,
    center_roster: String,
    center_alpha: String,
    hill_q: String,
    offset: f64,
    kappa: f64,
    minimum_center: f64,
    median_center: f64,
    maximum_center: f64,
    outcome: CnapOutcome,
}

fn order_name(order: impl Into<OrderName>) -> String {
    match order.into() {
        OrderName::Finite(value) => value.to_string(),
        OrderName::Infinity => "inf".to_string(),
    }
}

enum OrderName {
    Finite(f64),
    Infinity,
}

impl From<PowerOrder> for OrderName {
    fn from(value: PowerOrder) -> Self {
        match value {
            PowerOrder::Finite(x) => Self::Finite(x),
            PowerOrder::Infinity => Self::Infinity,
        }
    }
}

impl From<HillOrder> for OrderName {
    fn from(value: HillOrder) -> Self {
        match value {
            HillOrder::Finite(x) => Self::Finite(x),
            HillOrder::Infinity => Self::Infinity,
        }
    }
}

fn median(values: &[f64]) -> f64 {
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    let middle = ordered.len() / 2;
    if ordered.len() % 2 == 0 {
        (ordered[middle - 1] + ordered[middle]) / 2.0
    } else {
        ordered[middle]
    }
}

fn center_roster(
    kind: &str,
    scores: &[f64],
    peptides: &[String],
    hlas: &[String],
) -> Result<Vec<f64>> {
    if scores.is_empty() || scores.len() != peptides.len() || scores.len() != hlas.len() {
        return Err(SelectionError::msg(
            "invalid candidate/peptide/HLA center roster",
        ));
    }
    match kind {
        "raw" => Ok(scores.to_vec()),
        "hla_maxima" => {
            let mut maxima: HashMap<&str, f64> = HashMap::new();
            for (&score, hla) in scores.iter().zip(hlas) {
                maxima
                    .entry(hla)
                    .and_modify(|value| *value = value.max(score))
                    .or_insert(score);
            }
            Ok(maxima.into_values().collect())
        }
        "peptide_maxima" => {
            let mut maxima: HashMap<&str, f64> = HashMap::new();
            for (&score, peptide) in scores.iter().zip(peptides) {
                maxima
                    .entry(peptide)
                    .and_modify(|value| *value = value.max(score))
                    .or_insert(score);
            }
            Ok(maxima.into_values().collect())
        }
        "peptide_second_hla" => {
            let mut grouped: HashMap<&str, HashMap<&str, f64>> = HashMap::new();
            for ((&score, peptide), hla) in scores.iter().zip(peptides).zip(hlas) {
                grouped
                    .entry(peptide)
                    .or_default()
                    .entry(hla)
                    .and_modify(|value| *value = value.max(score))
                    .or_insert(score);
            }
            let mut seconds = Vec::with_capacity(grouped.len());
            for per_hla in grouped.into_values() {
                let mut values: Vec<f64> = per_hla.into_values().collect();
                values.sort_by(|a, b| b.total_cmp(a));
                if values.len() < 2 {
                    return Err(SelectionError::msg(
                        "peptide-second-HLA center requires two distinct HLAs",
                    ));
                }
                seconds.push(values[1]);
            }
            Ok(seconds)
        }
        _ => Err(SelectionError::msg(format!(
            "unknown intrinsic center roster: {kind}"
        ))),
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    if args.selection_replications == 0
        || args.offsets.is_empty()
        || args.offsets.iter().any(|x| !x.is_finite())
        || args.kappas.iter().any(|x| !x.is_finite() || *x <= 0.0)
        || args.center_rosters.iter().any(|kind| {
            !matches!(
                kind.as_str(),
                "raw" | "hla_maxima" | "peptide_maxima" | "peptide_second_hla"
            )
        })
    {
        return Err(SelectionError::msg("invalid intrinsic-center grid"));
    }
    let center_alphas: Result<Vec<PowerOrder>> = args
        .center_alphas
        .iter()
        .map(|value| PowerOrder::parse(value))
        .collect();
    let center_alphas = center_alphas?;
    let hill_qs: Result<Vec<HillOrder>> = args
        .hill_qs
        .iter()
        .map(|value| HillOrder::parse(value))
        .collect();
    let hill_qs = hill_qs?;
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

        for center_kind in &args.center_rosters {
            let rosters: Result<Vec<Vec<f64>>> = task
                .raw_candidates
                .iter()
                .zip(&task.raw_candidate_peptides)
                .zip(&task.raw_candidate_hlas)
                .map(|((scores, peptides), hlas)| {
                    center_roster(center_kind, scores, peptides, hlas)
                })
                .collect();
            let rosters = rosters?;
            for &center_alpha in &center_alphas {
                let centers: Result<Vec<f64>> = rosters
                    .iter()
                    .map(|roster| log_power_mean(roster, center_alpha))
                    .collect();
                let centers = centers?;
                for &hill_q in &hill_qs {
                    let (anchors, offers) =
                        adaptive_components(&task.raw_candidates, PowerOrder::Infinity, hill_q)?;
                    for &offset in &args.offsets {
                        let endpoint_centers: Vec<f64> =
                            centers.iter().map(|center| center + offset).collect();
                        let minimum_center = endpoint_centers
                            .iter()
                            .copied()
                            .fold(f64::INFINITY, f64::min);
                        let maximum_center = endpoint_centers
                            .iter()
                            .copied()
                            .fold(f64::NEG_INFINITY, f64::max);
                        let median_center = median(&endpoint_centers);
                        for &kappa in &args.kappas {
                            let scores: Result<Vec<f64>> = anchors
                                .iter()
                                .zip(&offers)
                                .zip(&endpoint_centers)
                                .map(|((&anchor, &offer), &center)| {
                                    self_gated_score(anchor, offer, center, kappa, 1e-10, 64)
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
                                complete_forward_assessment(&paired, observed, &contract, seed)?
                            };
                            rows.push(Row {
                                rule: "endpoint_intrinsic_centered_self_gate",
                                cohort: cohort.to_string(),
                                center_roster: center_kind.clone(),
                                center_alpha: order_name(center_alpha),
                                hill_q: order_name(hill_q),
                                offset,
                                kappa,
                                minimum_center,
                                median_center,
                                maximum_center,
                                outcome,
                            });
                        }
                    }
                }
            }
        }
    }

    let output = Output {
        schema_version: 1,
        analysis: "post_result_full_hla_endpoint_intrinsic_center_rule",
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
    use super::center_roster;

    #[test]
    fn center_rosters_preserve_requested_structure() {
        let scores = [-1.0, -3.0, -2.0, -1.5];
        let peptides = ["P1", "P1", "P2", "P2"].map(String::from);
        let hlas = ["A", "B", "A", "B"].map(String::from);
        let mut hla = center_roster("hla_maxima", &scores, &peptides, &hlas).unwrap();
        hla.sort_by(f64::total_cmp);
        assert_eq!(hla, vec![-1.5, -1.0]);
        let mut peptide = center_roster("peptide_maxima", &scores, &peptides, &hlas).unwrap();
        peptide.sort_by(f64::total_cmp);
        assert_eq!(peptide, vec![-1.5, -1.0]);
        let mut second = center_roster("peptide_second_hla", &scores, &peptides, &hlas).unwrap();
        second.sort_by(f64::total_cmp);
        assert_eq!(second, vec![-3.0, -2.0]);
    }
}
