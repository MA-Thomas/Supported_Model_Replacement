//! Endpoint-local full-HLA aggregators that preserve candidate identity.
//!
//! These diagnostics distinguish two biological forms of breadth:
//! presentation of one short peptide by two distinct HLAs, and presentation
//! of two distinct short peptides. Every score uses only the current long
//! peptide's real candidate F values, short-peptide identities, and HLA set.

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
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0,0.5,1,2,4,8,inf")]
    alpha_values: Vec<String>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.1,0.25,0.5,0.75,1,1.5,2,2.5,3,4")]
    lifts: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.25,0.5,1,1.5,2")]
    gap_distances: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.05,0.1,0.2,0.5,1")]
    gap_weights: Vec<f64>,
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

#[derive(Debug, Clone, Copy)]
struct StructuralEvidence {
    maximum: f64,
    second_hla: f64,
    second_peptide: f64,
    same_peptide_first: f64,
    same_peptide_second: f64,
}

fn two_largest(values: impl IntoIterator<Item = f64>) -> Result<[f64; 2]> {
    let mut values: Vec<f64> = values.into_iter().collect();
    values.sort_by(|a, b| b.total_cmp(a));
    if values.len() < 2 {
        return Err(SelectionError::msg(
            "structural aggregation requires at least two distinct groups",
        ));
    }
    Ok([values[0], values[1]])
}

fn structural_evidence(
    scores: &[f64],
    peptides: &[String],
    hlas: &[String],
) -> Result<StructuralEvidence> {
    if scores.is_empty() || scores.len() != peptides.len() || scores.len() != hlas.len() {
        return Err(SelectionError::msg(
            "invalid candidate/peptide/HLA structural roster",
        ));
    }
    let mut by_hla: HashMap<&str, f64> = HashMap::new();
    let mut by_peptide: HashMap<&str, Vec<(&str, f64)>> = HashMap::new();
    for ((&score, peptide), hla) in scores.iter().zip(peptides).zip(hlas) {
        by_hla
            .entry(hla)
            .and_modify(|value| *value = value.max(score))
            .or_insert(score);
        by_peptide.entry(peptide).or_default().push((hla, score));
    }
    let hla_pair = two_largest(by_hla.into_values())?;
    let peptide_pair = two_largest(by_peptide.values().map(|entries| {
        entries
            .iter()
            .map(|(_, score)| *score)
            .fold(f64::NEG_INFINITY, f64::max)
    }))?;

    let mut same_peptide_pairs = Vec::with_capacity(by_peptide.len());
    for entries in by_peptide.into_values() {
        let mut per_hla: HashMap<&str, f64> = HashMap::new();
        for (hla, score) in entries {
            per_hla
                .entry(hla)
                .and_modify(|value| *value = value.max(score))
                .or_insert(score);
        }
        if per_hla.len() >= 2 {
            same_peptide_pairs.push(two_largest(per_hla.into_values())?);
        }
    }
    let same_peptide = same_peptide_pairs
        .into_iter()
        .max_by(|a, b| a[1].total_cmp(&b[1]).then_with(|| a[0].total_cmp(&b[0])))
        .ok_or_else(|| {
            SelectionError::msg("no short peptide is represented for two distinct HLAs")
        })?;
    Ok(StructuralEvidence {
        maximum: hla_pair[0],
        second_hla: hla_pair[1],
        second_peptide: peptide_pair[1],
        same_peptide_first: same_peptide[0],
        same_peptide_second: same_peptide[1],
    })
}

fn assess(
    scores: Vec<f64>,
    reference_scores: &[f64],
    labels: &[bool],
    contract: &CnapContract,
    seed: u64,
    observed_only: bool,
) -> Result<CnapOutcome> {
    let (paired, observed) = observed_gate(&scores, reference_scores, labels, contract)?;
    if observed_only {
        Ok(observed)
    } else {
        complete_forward_assessment(&paired, observed, contract, seed)
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    if args.selection_replications == 0
        || args.lifts.iter().any(|x| !x.is_finite() || *x <= 0.0)
        || args
            .gap_distances
            .iter()
            .chain(&args.gap_weights)
            .any(|x| !x.is_finite() || *x <= 0.0)
    {
        return Err(SelectionError::msg("invalid structural aggregation grid"));
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
        let evidence: Result<Vec<StructuralEvidence>> = task
            .raw_candidates
            .iter()
            .zip(&task.raw_candidate_peptides)
            .zip(&task.raw_candidate_hlas)
            .map(|((scores, peptides), hlas)| structural_evidence(scores, peptides, hlas))
            .collect();
        let evidence = evidence?;

        for &power in &powers {
            let scores: Result<Vec<f64>> = evidence
                .iter()
                .map(|e| log_power_mean(&[e.same_peptide_first, e.same_peptide_second], power))
                .collect();
            rows.push(Row {
                rule: "same_peptide_top_two_hla_power_mean",
                cohort: cohort.to_string(),
                parameter: match power {
                    PowerOrder::Infinity => "alpha=inf".to_string(),
                    PowerOrder::Finite(value) => format!("alpha={value}"),
                },
                outcome: assess(
                    scores?,
                    &reference_scores,
                    &labels_bool,
                    &contract,
                    seed,
                    args.observed_only,
                )?,
            });
        }

        for &lift in &args.lifts {
            for (rule, scores) in [
                (
                    "max_or_lifted_second_hla",
                    evidence
                        .iter()
                        .map(|e| e.maximum.max(e.second_hla + lift))
                        .collect(),
                ),
                (
                    "max_or_lifted_second_peptide",
                    evidence
                        .iter()
                        .map(|e| e.maximum.max(e.second_peptide + lift))
                        .collect(),
                ),
                (
                    "max_or_lifted_same_peptide_second_hla",
                    evidence
                        .iter()
                        .map(|e| e.maximum.max(e.same_peptide_second + lift))
                        .collect(),
                ),
                (
                    "max_or_lifted_same_peptide_hla_mean",
                    evidence
                        .iter()
                        .map(|e| {
                            e.maximum
                                .max((e.same_peptide_first + e.same_peptide_second) / 2.0 + lift)
                        })
                        .collect(),
                ),
            ] {
                rows.push(Row {
                    rule,
                    cohort: cohort.to_string(),
                    parameter: format!("lift={lift}"),
                    outcome: assess(
                        scores,
                        &reference_scores,
                        &labels_bool,
                        &contract,
                        seed,
                        args.observed_only,
                    )?,
                });
            }
        }

        for &distance in &args.gap_distances {
            for &weight in &args.gap_weights {
                let scores = evidence
                    .iter()
                    .map(|e| {
                        let hla_support = (distance - (e.maximum - e.second_hla)).max(0.0);
                        let peptide_support = (distance - (e.maximum - e.second_peptide)).max(0.0);
                        e.maximum + weight * hla_support.min(peptide_support)
                    })
                    .collect();
                rows.push(Row {
                    rule: "max_with_joint_hla_and_peptide_breadth",
                    cohort: cohort.to_string(),
                    parameter: format!("distance={distance};weight={weight}"),
                    outcome: assess(
                        scores,
                        &reference_scores,
                        &labels_bool,
                        &contract,
                        seed,
                        args.observed_only,
                    )?,
                });
            }
        }
    }

    let output = Output {
        schema_version: 1,
        analysis: "post_result_full_hla_endpoint_local_structural_rule",
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
    use super::structural_evidence;

    #[test]
    fn preserves_same_peptide_and_distinct_hla_structure() {
        let scores = [-1.0, -3.0, -2.0, -1.5, -4.0, -2.5];
        let peptides = ["P1", "P1", "P2", "P2", "P3", "P3"].map(String::from);
        let hlas = ["A", "B", "A", "B", "A", "B"].map(String::from);
        let e = structural_evidence(&scores, &peptides, &hlas).unwrap();
        assert_eq!(e.maximum, -1.0);
        assert_eq!(e.second_hla, -1.5);
        assert_eq!(e.second_peptide, -1.5);
        assert_eq!(e.same_peptide_first, -1.5);
        assert_eq!(e.same_peptide_second, -2.0);
    }
}
