//! Adaptive full-HLA aggregation after endpoint-local biological collapsing.
//!
//! Candidate redundancy is removed before Hill breadth is computed. Roster
//! options are one maximum per distinct HLA, the two strongest HLA maxima, or
//! one maximum per distinct short peptide. Every operation is local to one
//! long peptide and its actual HLA profile.

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
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "hla_maxima,top_two_hla_maxima,peptide_maxima")]
    collapsed_rosters: Vec<String>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "inf")]
    alpha_values: Vec<String>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.5,1,2,inf")]
    q_values: Vec<String>,
    #[arg(long, value_delimiter = ',', num_args = 1.., allow_hyphen_values = true)]
    c_values: Vec<f64>,
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
    collapsed_roster: String,
    alpha: String,
    q: String,
    c: f64,
    kappa: f64,
    outcome: CnapOutcome,
}

fn power_name(value: PowerOrder) -> String {
    match value {
        PowerOrder::Finite(x) => x.to_string(),
        PowerOrder::Infinity => "inf".to_string(),
    }
}

fn hill_name(value: HillOrder) -> String {
    match value {
        HillOrder::Finite(x) => x.to_string(),
        HillOrder::Infinity => "inf".to_string(),
    }
}

fn collapsed_roster(
    kind: &str,
    scores: &[f64],
    peptides: &[String],
    hlas: &[String],
) -> Result<Vec<f64>> {
    if scores.is_empty() || scores.len() != peptides.len() || scores.len() != hlas.len() {
        return Err(SelectionError::msg(
            "invalid candidate/peptide/HLA collapsed roster",
        ));
    }
    let key_values = match kind {
        "hla_maxima" | "top_two_hla_maxima" => scores.iter().zip(hlas),
        "peptide_maxima" => scores.iter().zip(peptides),
        _ => {
            return Err(SelectionError::msg(format!(
                "unknown collapsed roster: {kind}"
            )));
        }
    };
    let mut maxima: HashMap<&str, f64> = HashMap::new();
    for (&score, key) in key_values {
        maxima
            .entry(key)
            .and_modify(|value| *value = value.max(score))
            .or_insert(score);
    }
    let mut values: Vec<f64> = maxima.into_values().collect();
    values.sort_by(|a, b| b.total_cmp(a));
    if kind == "top_two_hla_maxima" {
        if values.len() < 2 {
            return Err(SelectionError::msg(
                "top-two collapsed roster requires two distinct HLAs",
            ));
        }
        values.truncate(2);
    }
    Ok(values)
}

fn run() -> Result<()> {
    let args = Args::parse();
    if args.selection_replications == 0
        || args.c_values.is_empty()
        || args.c_values.iter().any(|x| !x.is_finite())
        || args.kappas.iter().any(|x| !x.is_finite() || *x <= 0.0)
        || args.collapsed_rosters.iter().any(|kind| {
            !matches!(
                kind.as_str(),
                "hla_maxima" | "top_two_hla_maxima" | "peptide_maxima"
            )
        })
    {
        return Err(SelectionError::msg("invalid collapsed adaptive grid"));
    }
    let powers: Result<Vec<PowerOrder>> = args
        .alpha_values
        .iter()
        .map(|value| PowerOrder::parse(value))
        .collect();
    let powers = powers?;
    let hills: Result<Vec<HillOrder>> = args
        .q_values
        .iter()
        .map(|value| HillOrder::parse(value))
        .collect();
    let hills = hills?;
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

        for roster_kind in &args.collapsed_rosters {
            let rosters: Result<Vec<Vec<f64>>> = task
                .raw_candidates
                .iter()
                .zip(&task.raw_candidate_peptides)
                .zip(&task.raw_candidate_hlas)
                .map(|((scores, peptides), hlas)| {
                    collapsed_roster(roster_kind, scores, peptides, hlas)
                })
                .collect();
            let rosters = rosters?;
            for &power in &powers {
                for &hill in &hills {
                    let (anchors, offers) = adaptive_components(&rosters, power, hill)?;
                    for &c in &args.c_values {
                        for &kappa in &args.kappas {
                            let scores: Result<Vec<f64>> = anchors
                                .iter()
                                .zip(&offers)
                                .map(|(&anchor, &offer)| {
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
                                complete_forward_assessment(&paired, observed, &contract, seed)?
                            };
                            rows.push(Row {
                                rule: "collapsed_roster_adaptive_hill",
                                cohort: cohort.to_string(),
                                collapsed_roster: roster_kind.clone(),
                                alpha: power_name(power),
                                q: hill_name(hill),
                                c,
                                kappa,
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
        analysis: "post_result_full_hla_endpoint_local_collapsed_adaptive_rule",
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
    use super::collapsed_roster;

    #[test]
    fn collapses_to_distinct_hla_and_peptide_maxima() {
        let scores = [-1.0, -3.0, -2.0, -1.5];
        let peptides = ["P1", "P1", "P2", "P2"].map(String::from);
        let hlas = ["A", "B", "A", "B"].map(String::from);
        assert_eq!(
            collapsed_roster("top_two_hla_maxima", &scores, &peptides, &hlas).unwrap(),
            vec![-1.0, -1.5]
        );
        assert_eq!(
            collapsed_roster("peptide_maxima", &scores, &peptides, &hlas).unwrap(),
            vec![-1.0, -1.5]
        );
    }
}
