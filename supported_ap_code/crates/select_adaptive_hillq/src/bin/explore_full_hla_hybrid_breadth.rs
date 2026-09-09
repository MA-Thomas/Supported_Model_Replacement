//! Endpoint-local exploration of second- and third-HLA corroboration.
//!
//! Every endpoint is projected in two deterministic ways: one maximum per
//! distinct short peptide and one maximum per distinct HLA. The
//! distinct-peptide roster supplies the established self-gated score. A fixed
//! total HLA weight is then combined with an independently gated third-HLA
//! term through one of three declared families: fixed-cap reallocation,
//! additive bonus, or conjunctive qualification. Zero third-HLA weight exactly
//! recovers the current second-HLA hybrid.

use std::collections::HashMap;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};
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
    FLOOR, HillOrder, PowerOrder, adaptive_components, expit, self_gated_score,
};

const MODEL: &str = "full_hla";
const CURRENT_RULE: &str = "convex_epitope_second_hla_hybrid";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
enum ThirdHlaFamily {
    /// Move a fixed portion of the second-HLA weight to the third HLA.
    Reallocate,
    /// Preserve the current hybrid and add an independently bounded third-HLA term.
    Additive,
    /// Require the third-HLA gate to qualify a portion of the second-HLA bonus.
    Conjunctive,
}

impl ThirdHlaFamily {
    fn rule_id(self) -> &'static str {
        match self {
            Self::Reallocate => "third_hla_fixed_cap_reallocation",
            Self::Additive => "third_hla_additive_bonus",
            Self::Conjunctive => "third_hla_conjunctive_qualification",
        }
    }

    fn formula(self) -> &'static str {
        match self {
            Self::Reallocate => "m + (1-w_hla)*(S_E-m) + (w_hla-w_3)*B*g_2 + w_3*B*g_3",
            Self::Additive => "m + (1-w_hla)*(S_E-m) + w_hla*B*g_2 + w_3*B*g_3",
            Self::Conjunctive => "m + (1-w_hla)*(S_E-m) + (w_hla-w_3)*B*g_2 + w_3*B*g_2*g_3",
        }
    }
}

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
    #[arg(long, value_delimiter = ',', num_args = 1.., allow_hyphen_values = true, default_value = "-2.2")]
    c_values: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.13")]
    kappas: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., allow_hyphen_values = true, default_value = "-6.45")]
    second_hla_thresholds: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.02")]
    second_hla_widths: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., allow_hyphen_values = true, default_value = "-8,-7.5,-7,-6.75,-6.5,-6.25,-6,-5.75,-5.5")]
    third_hla_thresholds: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.02,0.05,0.1,0.25,0.5")]
    third_hla_widths: Vec<f64>,
    /// Total convex weight reserved for all HLA corroboration branches.
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.12")]
    total_hla_weights: Vec<f64>,
    /// Final-score weight assigned to the third-HLA term.
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "0.01,0.02,0.04,0.06,0.08,0.12")]
    third_hla_weights: Vec<f64>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "reallocate,additive,conjunctive")]
    third_hla_families: Vec<ThirdHlaFamily>,
    #[arg(long, value_delimiter = ',', num_args = 1.., default_value = "1")]
    hla_bonuses: Vec<f64>,
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Serialize)]
struct Output {
    schema_version: u32,
    analysis: &'static str,
    model: &'static str,
    design: &'static str,
    source_root: String,
    bundle_root: String,
    selection_replications: usize,
    observed_only: bool,
    rows: Vec<Row>,
}

#[derive(Debug, Serialize)]
struct Row {
    rule: &'static str,
    formula: &'static str,
    third_hla_family: Option<ThirdHlaFamily>,
    cohort: String,
    c: f64,
    kappa: f64,
    second_hla_threshold: f64,
    second_hla_width: f64,
    third_hla_threshold: Option<f64>,
    third_hla_width: Option<f64>,
    hla_bonus: f64,
    epitope_weight: f64,
    total_hla_weight: f64,
    second_hla_weight: f64,
    third_hla_weight: f64,
    maximum_hla_uplift: f64,
    comparisons: Vec<Comparison>,
}

#[derive(Debug, Serialize)]
struct Comparison {
    reference_rule: &'static str,
    outcome: CnapOutcome,
}

#[derive(Debug, Clone, PartialEq)]
struct CollapsedEvidence {
    peptide_maxima: Vec<f64>,
    hla_maxima_descending: Vec<f64>,
}

impl CollapsedEvidence {
    fn anchor(&self) -> f64 {
        self.peptide_maxima[0]
    }

    fn hla_maximum(&self, order: usize) -> Result<f64> {
        if order == 0 {
            return Err(SelectionError::msg("HLA order must be one-based"));
        }
        self.hla_maxima_descending
            .get(order - 1)
            .copied()
            .ok_or_else(|| {
                SelectionError::msg(format!(
                    "hybrid rule requires at least {order} distinct normalized HLAs"
                ))
            })
    }
}

fn sort_descending(values: &mut [f64]) {
    values.sort_by(|a, b| b.total_cmp(a));
}

fn collapse_evidence(
    scores: &[f64],
    peptides: &[String],
    hlas: &[String],
) -> Result<CollapsedEvidence> {
    if scores.is_empty() || scores.len() != peptides.len() || scores.len() != hlas.len() {
        return Err(SelectionError::msg(
            "invalid candidate/peptide/HLA hybrid roster",
        ));
    }

    let mut peptide_maxima: HashMap<&str, f64> = HashMap::new();
    let mut hla_maxima: HashMap<&str, f64> = HashMap::new();
    for ((&score, peptide), hla) in scores.iter().zip(peptides).zip(hlas) {
        if !score.is_finite() || score < FLOOR - 1e-12 {
            return Err(SelectionError::msg(
                "hybrid roster contains an invalid Level-1 score",
            ));
        }
        if peptide.is_empty() || hla.is_empty() {
            return Err(SelectionError::msg(
                "hybrid roster contains an empty peptide or normalized HLA identity",
            ));
        }
        peptide_maxima
            .entry(peptide)
            .and_modify(|value| *value = value.max(score))
            .or_insert(score);
        hla_maxima
            .entry(hla)
            .and_modify(|value| *value = value.max(score))
            .or_insert(score);
    }

    let mut peptide_maxima: Vec<f64> = peptide_maxima.into_values().collect();
    let mut hla_maxima_descending: Vec<f64> = hla_maxima.into_values().collect();
    sort_descending(&mut peptide_maxima);
    sort_descending(&mut hla_maxima_descending);
    Ok(CollapsedEvidence {
        peptide_maxima,
        hla_maxima_descending,
    })
}

fn epitope_branch_scores(evidence: &[CollapsedEvidence], c: f64, kappa: f64) -> Result<Vec<f64>> {
    let rosters: Vec<Vec<f64>> = evidence
        .iter()
        .map(|item| item.peptide_maxima.clone())
        .collect();
    let (anchors, offers) =
        adaptive_components(&rosters, PowerOrder::Infinity, HillOrder::Infinity)?;
    anchors
        .iter()
        .zip(&offers)
        .map(|(&anchor, &offer)| self_gated_score(anchor, offer, c, kappa, 1e-10, 64))
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct HybridSpec {
    second_hla_threshold: f64,
    second_hla_width: f64,
    third_hla_threshold: f64,
    third_hla_width: f64,
    hla_bonus: f64,
    total_hla_weight: f64,
    third_hla_weight: f64,
    family: ThirdHlaFamily,
}

impl HybridSpec {
    fn second_hla_weight(self) -> f64 {
        match self.family {
            ThirdHlaFamily::Additive => self.total_hla_weight,
            ThirdHlaFamily::Reallocate | ThirdHlaFamily::Conjunctive => {
                self.total_hla_weight - self.third_hla_weight
            }
        }
    }

    fn epitope_weight(self) -> f64 {
        1.0 - self.total_hla_weight
    }

    fn maximum_hla_uplift(self) -> f64 {
        let weight = match self.family {
            ThirdHlaFamily::Additive => self.total_hla_weight + self.third_hla_weight,
            ThirdHlaFamily::Reallocate | ThirdHlaFamily::Conjunctive => self.total_hla_weight,
        };
        weight * self.hla_bonus
    }
}

fn hla_gate(evidence: &CollapsedEvidence, order: usize, threshold: f64, width: f64) -> Result<f64> {
    let maximum = evidence.hla_maximum(order)?;
    Ok(expit((maximum - threshold) / width))
}

fn constrained_hybrid_score(
    evidence: &CollapsedEvidence,
    epitope_score: f64,
    spec: HybridSpec,
) -> Result<f64> {
    let second_gate = hla_gate(
        evidence,
        2,
        spec.second_hla_threshold,
        spec.second_hla_width,
    )?;
    let third_gate = if spec.third_hla_weight == 0.0 {
        0.0
    } else {
        hla_gate(evidence, 3, spec.third_hla_threshold, spec.third_hla_width)?
    };
    let anchor = evidence.anchor();
    let shared = anchor
        + spec.epitope_weight() * (epitope_score - anchor)
        + spec.second_hla_weight() * spec.hla_bonus * second_gate;
    let third_term = match spec.family {
        ThirdHlaFamily::Reallocate | ThirdHlaFamily::Additive => {
            spec.third_hla_weight * spec.hla_bonus * third_gate
        }
        ThirdHlaFamily::Conjunctive => {
            spec.third_hla_weight * spec.hla_bonus * second_gate * third_gate
        }
    };
    Ok(shared + third_term)
}

fn score_endpoints(
    evidence: &[CollapsedEvidence],
    epitope_scores: &[f64],
    spec: HybridSpec,
) -> Result<Vec<f64>> {
    if evidence.len() != epitope_scores.len() {
        return Err(SelectionError::msg(
            "collapsed evidence and epitope scores are not aligned",
        ));
    }
    evidence
        .iter()
        .zip(epitope_scores)
        .map(|(item, &epitope_score)| constrained_hybrid_score(item, epitope_score, spec))
        .collect()
}

fn assess(
    scores: &[f64],
    reference_scores: &[f64],
    labels: &[bool],
    contract: &CnapContract,
    seed: u64,
    observed_only: bool,
) -> Result<CnapOutcome> {
    let (paired, observed) = observed_gate(scores, reference_scores, labels, contract)?;
    if observed_only {
        Ok(observed)
    } else {
        complete_forward_assessment(&paired, observed, contract, seed)
    }
}

fn validate_args(args: &Args) -> Result<()> {
    let invalid = args.selection_replications == 0
        || args.c_values.iter().any(|x| !x.is_finite())
        || args
            .second_hla_thresholds
            .iter()
            .chain(&args.third_hla_thresholds)
            .any(|x| !x.is_finite())
        || args
            .kappas
            .iter()
            .chain(&args.second_hla_widths)
            .chain(&args.third_hla_widths)
            .any(|x| !x.is_finite() || *x <= 0.0)
        || args
            .total_hla_weights
            .iter()
            .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
        || args
            .third_hla_weights
            .iter()
            .any(|x| !x.is_finite() || *x < 0.0)
        || args.hla_bonuses.iter().any(|x| !x.is_finite() || *x <= 0.0)
        || args.third_hla_weights.iter().any(|third| {
            args.total_hla_weights.iter().any(|total| {
                third > total
                    && args.third_hla_families.iter().any(|family| {
                        matches!(
                            family,
                            ThirdHlaFamily::Reallocate | ThirdHlaFamily::Conjunctive
                        )
                    })
            })
        });
    if invalid {
        return Err(SelectionError::msg(
            "invalid constrained second-/third-HLA grid",
        ));
    }
    Ok(())
}

fn run() -> Result<()> {
    let args = Args::parse();
    validate_args(&args)?;
    let bundle = load_bundle(&args.bundle_root.join("pr"))
        .map_err(|error| SelectionError::msg(format!("failed to load PR bundle: {error}")))?;
    let contract =
        CnapContract::from_pr_bundle_with_replications(&bundle, args.selection_replications)?;
    let mut rows = Vec::new();

    for cohort in SELECTION_COHORTS {
        let (task, _) = load_task(&args.source_root, cohort, MODEL, "pr")?;
        let authority = authoritative_endpoint_table(&args.bundle_root, cohort)?;
        let endpoint_ids = validate_task_labels(&task, &authority)?;
        let labels = task.labels();
        let labels_bool: Vec<bool> = labels.iter().map(|&label| label == 1).collect();
        let (_, maximum_scores, _) =
            aligned_maximum_reference(&bundle, cohort, MODEL, &endpoint_ids, &labels)?;
        let seed = contract.selection_seed(cohort)?;
        let evidence: Result<Vec<CollapsedEvidence>> = task
            .raw_candidates
            .iter()
            .zip(&task.raw_candidate_peptides)
            .zip(&task.raw_candidate_hlas)
            .map(|((scores, peptides), hlas)| collapse_evidence(scores, peptides, hlas))
            .collect();
        let evidence = evidence?;

        for &c in &args.c_values {
            for &kappa in &args.kappas {
                let epitope_scores = epitope_branch_scores(&evidence, c, kappa)?;
                for &second_hla_threshold in &args.second_hla_thresholds {
                    for &second_hla_width in &args.second_hla_widths {
                        for &hla_bonus in &args.hla_bonuses {
                            for &total_hla_weight in &args.total_hla_weights {
                                let baseline_spec = HybridSpec {
                                    second_hla_threshold,
                                    second_hla_width,
                                    third_hla_threshold: 0.0,
                                    third_hla_width: 1.0,
                                    hla_bonus,
                                    total_hla_weight,
                                    third_hla_weight: 0.0,
                                    family: ThirdHlaFamily::Reallocate,
                                };
                                let baseline_scores =
                                    score_endpoints(&evidence, &epitope_scores, baseline_spec)?;
                                rows.push(Row {
                                    rule: CURRENT_RULE,
                                    formula: "m + (1-w_hla)*(S_E-m) + w_hla*B*g_2",
                                    third_hla_family: None,
                                    cohort: cohort.to_string(),
                                    c,
                                    kappa,
                                    second_hla_threshold,
                                    second_hla_width,
                                    third_hla_threshold: None,
                                    third_hla_width: None,
                                    hla_bonus,
                                    epitope_weight: baseline_spec.epitope_weight(),
                                    total_hla_weight,
                                    second_hla_weight: total_hla_weight,
                                    third_hla_weight: 0.0,
                                    maximum_hla_uplift: total_hla_weight * hla_bonus,
                                    comparisons: vec![Comparison {
                                        reference_rule: "full_hla__max",
                                        outcome: assess(
                                            &baseline_scores,
                                            &maximum_scores,
                                            &labels_bool,
                                            &contract,
                                            seed,
                                            args.observed_only,
                                        )?,
                                    }],
                                });

                                for &third_hla_threshold in &args.third_hla_thresholds {
                                    for &third_hla_width in &args.third_hla_widths {
                                        for &family in &args.third_hla_families {
                                            for &third_hla_weight in &args.third_hla_weights {
                                                if third_hla_weight == 0.0 {
                                                    continue;
                                                }
                                                if third_hla_weight > total_hla_weight
                                                    && !matches!(family, ThirdHlaFamily::Additive)
                                                {
                                                    continue;
                                                }
                                                let spec = HybridSpec {
                                                    second_hla_threshold,
                                                    second_hla_width,
                                                    third_hla_threshold,
                                                    third_hla_width,
                                                    hla_bonus,
                                                    total_hla_weight,
                                                    third_hla_weight,
                                                    family,
                                                };
                                                let scores = score_endpoints(
                                                    &evidence,
                                                    &epitope_scores,
                                                    spec,
                                                )?;
                                                rows.push(Row {
                                                    rule: family.rule_id(),
                                                    formula: family.formula(),
                                                    third_hla_family: Some(family),
                                                    cohort: cohort.to_string(),
                                                    c,
                                                    kappa,
                                                    second_hla_threshold,
                                                    second_hla_width,
                                                    third_hla_threshold: Some(third_hla_threshold),
                                                    third_hla_width: Some(third_hla_width),
                                                    hla_bonus,
                                                    epitope_weight: spec.epitope_weight(),
                                                    total_hla_weight,
                                                    second_hla_weight: spec.second_hla_weight(),
                                                    third_hla_weight,
                                                    maximum_hla_uplift: spec.maximum_hla_uplift(),
                                                    comparisons: vec![
                                                        Comparison {
                                                            reference_rule: "full_hla__max",
                                                            outcome: assess(
                                                                &scores,
                                                                &maximum_scores,
                                                                &labels_bool,
                                                                &contract,
                                                                seed,
                                                                args.observed_only,
                                                            )?,
                                                        },
                                                        Comparison {
                                                            reference_rule: CURRENT_RULE,
                                                            outcome: assess(
                                                                &scores,
                                                                &baseline_scores,
                                                                &labels_bool,
                                                                &contract,
                                                                seed,
                                                                args.observed_only,
                                                            )?,
                                                        },
                                                    ],
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let output = Output {
        schema_version: 3,
        analysis: "post_result_broad_third_hla_exploration",
        model: MODEL,
        design: "independent third-HLA threshold/width across reallocation, additive, and conjunctive families",
        source_root: args.source_root.display().to_string(),
        bundle_root: args.bundle_root.display().to_string(),
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
        "wrote {} hybrid evaluations to {}",
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
    use iris_fullroster_pipeline::contract::{AdaptiveL2Method, AdaptiveL2Spec};
    use iris_fullroster_pipeline::hybrid::aggregate_frozen_adaptive_l2;

    use super::{
        CollapsedEvidence, HybridSpec, ThirdHlaFamily, collapse_evidence, constrained_hybrid_score,
        hla_gate, score_endpoints,
    };
    use select_adaptive_hillq::numeric::FLOOR;

    fn evidence() -> CollapsedEvidence {
        let scores = [-1.0, -3.0, -2.0, -1.5, -2.5];
        let peptides = ["P1", "P1", "P2", "P2", "P3"].map(String::from);
        let hlas = ["A", "B", "A", "B", "C"].map(String::from);
        collapse_evidence(&scores, &peptides, &hlas).unwrap()
    }

    fn production_spec() -> AdaptiveL2Spec {
        AdaptiveL2Spec {
            method: AdaptiveL2Method::EndpointLocalEpitopeSecondHlaHybrid,
            epitope_gate_center: -2.2,
            epitope_gate_width: 0.13,
            second_hla_threshold: -6.45,
            second_hla_gate_width: 0.02,
            hla_bonus: 1.0,
            hla_weight: 0.12,
            solver_absolute_tolerance: 1e-10,
            solver_max_iterations: 64,
        }
    }

    #[test]
    fn production_frozen_hybrid_matches_the_exploratory_equation() {
        let scores = [-1.0, -3.0, -2.0, -1.5, -2.5];
        let peptides = ["P1", "P1", "P2", "P2", "P3"].map(String::from);
        let hlas = ["A", "B", "A", "B", "C"].map(String::from);
        let collapsed = collapse_evidence(&scores, &peptides, &hlas).unwrap();
        let epitope_score =
            super::epitope_branch_scores(std::slice::from_ref(&collapsed), -2.2, 0.13).unwrap()[0];
        let exploratory = constrained_hybrid_score(
            &collapsed,
            epitope_score,
            HybridSpec {
                second_hla_threshold: -6.45,
                second_hla_width: 0.02,
                third_hla_threshold: 0.0,
                third_hla_width: 1.0,
                hla_bonus: 1.0,
                total_hla_weight: 0.12,
                third_hla_weight: 0.0,
                family: ThirdHlaFamily::Reallocate,
            },
        )
        .unwrap();
        let production =
            aggregate_frozen_adaptive_l2(&scores, &peptides, &hlas, &production_spec(), FLOOR)
                .unwrap();
        assert_eq!(production.to_bits(), exploratory.to_bits());
    }

    fn spec(family: ThirdHlaFamily, third_hla_weight: f64) -> HybridSpec {
        HybridSpec {
            second_hla_threshold: -2.0,
            second_hla_width: 0.5,
            third_hla_threshold: -2.0,
            third_hla_width: 0.5,
            hla_bonus: 1.0,
            total_hla_weight: 0.12,
            third_hla_weight,
            family,
        }
    }

    #[test]
    fn collapses_and_sorts_both_projections() {
        let evidence = evidence();
        assert_eq!(evidence.peptide_maxima, vec![-1.0, -1.5, -2.5]);
        assert_eq!(evidence.hla_maxima_descending, vec![-1.0, -1.5, -2.5]);
        assert_eq!(evidence.anchor(), -1.0);
        assert_eq!(evidence.hla_maximum(2).unwrap(), -1.5);
        assert_eq!(evidence.hla_maximum(3).unwrap(), -2.5);
    }

    #[test]
    fn collapse_is_permutation_invariant() {
        let original = evidence();
        let scores = [-2.5, -1.5, -2.0, -3.0, -1.0];
        let peptides = ["P3", "P2", "P2", "P1", "P1"].map(String::from);
        let hlas = ["C", "B", "A", "B", "A"].map(String::from);
        let permuted = collapse_evidence(&scores, &peptides, &hlas).unwrap();
        assert_eq!(original, permuted);
    }

    #[test]
    fn rejects_invalid_scores_and_identities() {
        let peptide = [String::from("P")];
        let hla = [String::from("A")];
        assert!(collapse_evidence(&[f64::NAN], &peptide, &hla).is_err());
        assert!(collapse_evidence(&[FLOOR - 1.0], &peptide, &hla).is_err());
        assert!(collapse_evidence(&[-1.0], &[String::new()], &hla).is_err());
    }

    #[test]
    fn reports_an_unavailable_third_hla() {
        let scores = [-1.0, -2.0];
        let peptides = ["P1", "P2"].map(String::from);
        let hlas = ["A", "B"].map(String::from);
        let evidence = collapse_evidence(&scores, &peptides, &hlas).unwrap();
        assert!(hla_gate(&evidence, 3, -2.0, 0.5).is_err());
    }

    #[test]
    fn rejects_misaligned_endpoint_scores() {
        assert!(
            score_endpoints(&[evidence()], &[], spec(ThirdHlaFamily::Reallocate, 0.0)).is_err()
        );
    }

    #[test]
    fn zero_third_weight_recovers_the_current_hybrid_in_every_family() {
        let evidence = evidence();
        let epitope_score = -0.8;
        for family in [
            ThirdHlaFamily::Reallocate,
            ThirdHlaFamily::Additive,
            ThirdHlaFamily::Conjunctive,
        ] {
            let spec = spec(family, 0.0);
            let actual = constrained_hybrid_score(&evidence, epitope_score, spec).unwrap();
            let second_gate = hla_gate(
                &evidence,
                2,
                spec.second_hla_threshold,
                spec.second_hla_width,
            )
            .unwrap();
            let expected = (1.0 - spec.total_hla_weight) * epitope_score
                + spec.total_hla_weight * (evidence.anchor() + spec.hla_bonus * second_gate);
            assert!((actual - expected).abs() < 1e-14);
        }
    }

    #[test]
    fn third_hla_reallocation_preserves_the_hla_uplift_cap() {
        let evidence = evidence();
        let epitope_score = -0.8;
        let spec = spec(ThirdHlaFamily::Reallocate, 0.04);
        let score = constrained_hybrid_score(&evidence, epitope_score, spec).unwrap();
        let non_hla_part =
            evidence.anchor() + spec.epitope_weight() * (epitope_score - evidence.anchor());
        let hla_uplift = score - non_hla_part;
        assert!(hla_uplift >= 0.0);
        assert!(hla_uplift <= spec.maximum_hla_uplift());
    }

    #[test]
    fn additive_family_has_the_declared_larger_cap() {
        let spec = spec(ThirdHlaFamily::Additive, 0.04);
        assert!((spec.maximum_hla_uplift() - 0.16).abs() < 1e-14);
        assert!((spec.second_hla_weight() - 0.12).abs() < 1e-14);
    }

    #[test]
    fn conjunctive_family_never_exceeds_the_reallocation_score() {
        let evidence = evidence();
        let epitope_score = -0.8;
        let reallocate = constrained_hybrid_score(
            &evidence,
            epitope_score,
            spec(ThirdHlaFamily::Reallocate, 0.04),
        )
        .unwrap();
        let conjunctive = constrained_hybrid_score(
            &evidence,
            epitope_score,
            spec(ThirdHlaFamily::Conjunctive, 0.04),
        )
        .unwrap();
        assert!(conjunctive <= reallocate);
    }
}
