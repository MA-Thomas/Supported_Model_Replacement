//! CLI entry point: self-gated power-anchor Hill-q L2 parameter selection.
//!
//! The joint-grid sweep is parallelized with rayon; ranking and both policy
//! selections are deterministic across thread counts.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use clap::Parser;
use directed_round_robin_organizer::input::{LoadedBundle, load_bundle};
use rayon::prelude::*;

use select_adaptive_hillq::cluster::{PrecomputedCnap, load_complete_cache};
use select_adaptive_hillq::cnap::{
    CnapContract, CnapOutcome, aligned_maximum_reference, complete_forward_assessment,
    observed_gate, ranking_signature,
};
use select_adaptive_hillq::config::{
    BRANCHES, DEFAULT_SOURCE, MODELS, N_GROUPS, SELECTION_COHORTS,
};
use select_adaptive_hillq::data::{
    Audit, AuthEndpoint, Task, authoritative_endpoint_table, load_task, validate_label_contract,
    validate_task_labels,
};
use select_adaptive_hillq::error::{Result, SelectionError};
use select_adaptive_hillq::grid::{
    AggregationOrder, BaseGrid, GridSpec, aggregation_orders, default_q_tokens, make_grid,
    parse_alpha_values, parse_q_values,
};
use select_adaptive_hillq::manifest::sha256_file;
use select_adaptive_hillq::numeric::{
    FLOOR, adaptive_components, ap_auc, max_of, self_gated_score,
};
use select_adaptive_hillq::output::{fmt_f64, write_csv, write_csv_gz_iter, write_text};
use select_adaptive_hillq::select::{
    JointRanking, LeaveOneOutRow, SelectedCnapEvidence, SelectedRow, SelectionPolicy,
    leave_one_cohort_out, select_group, select_group_eligible, select_group_for_indices_eligible,
};

const SCHEMA_VERSION: u32 = 5;

#[derive(Parser, Debug)]
#[command(
    name = "select-adaptive-hillq",
    about = "Self-gated power-anchor Hill-q L2 aggregation parameter selection",
    allow_negative_numbers = true
)]
struct Args {
    #[arg(long, default_value = DEFAULT_SOURCE)]
    source_root: PathBuf,
    #[arg(
        long,
        default_value = "IRIS_scripts/prebuilt_bundles/full_roster_fixed_l2"
    )]
    bundle_root: PathBuf,
    #[arg(
        long,
        default_value = "IRIS_scripts/analysis_reports/adaptive_power_hillq_selection"
    )]
    output: PathBuf,
    /// Explicit power-mean orders. There is deliberately no production default.
    #[arg(long, value_delimiter = ' ', num_args = 1.., required = true)]
    alpha_values: Vec<String>,
    /// Frozen Hill-order list (space-separated).
    #[arg(long, value_delimiter = ' ', num_args = 1..)]
    q_values: Vec<String>,
    #[arg(long, default_value_t = -12.0)]
    c_min: f64,
    #[arg(long, default_value_t = 2.0)]
    c_max: f64,
    #[arg(long, default_value_t = 0.05)]
    c_step: f64,
    #[arg(long, default_value_t = 1e-4)]
    kappa_min: f64,
    #[arg(long, default_value_t = 4.0)]
    kappa_max: f64,
    #[arg(long, default_value_t = 113)]
    kappa_points: usize,
    #[arg(long, default_value_t = 1e-10)]
    solver_absolute_tolerance: f64,
    #[arg(long, default_value_t = 64)]
    solver_max_iterations: usize,
    #[arg(long, default_value_t = 0.01)]
    near_optimal_rank_tolerance: f64,
    /// rayon worker threads (default: all cores).
    #[arg(long)]
    threads: Option<usize>,
    /// Separate finite challenge roster used for parameter selection only.
    #[arg(long, default_value_t = 200)]
    selection_replications: usize,
    /// Immutable cluster plan supplying precomputed paired-CNAP outcomes.
    #[arg(long, requires = "cnap_results")]
    cnap_plan: Option<PathBuf>,
    /// Complete audited result shards for --cnap-plan.
    #[arg(long, requires = "cnap_plan")]
    cnap_results: Option<PathBuf>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn bool_str(b: bool) -> String {
    if b { "True".into() } else { "False".into() }
}

/// Metadata for one joint-grid index.
struct JointMeta {
    aggregation_order: usize,
    alpha_order: usize,
    alpha_id: String,
    alpha_str: String,
    q_order: usize,
    q_id: String,
    q_str: String,
    grid_index: usize,
    c: f64,
    kappa: f64,
}

fn joint_meta(
    j: usize,
    base_len: usize,
    orders: &[AggregationOrder],
    base: &BaseGrid,
) -> JointMeta {
    let aggregation_order = j / base_len;
    let grid_index = j % base_len;
    let order = orders[aggregation_order];
    JointMeta {
        aggregation_order,
        alpha_order: order.alpha_order,
        alpha_id: order.power.id(),
        alpha_str: fmt_f64(order.power.value()),
        q_order: order.q_order,
        q_id: order.hill.id(),
        q_str: fmt_f64(order.hill.value()),
        grid_index,
        c: base.c[grid_index],
        kappa: base.kappa[grid_index],
    }
}

const SELECTED_HEADER: [&str; 64] = [
    "selection_policy",
    "model",
    "branch",
    "selection_metric",
    "aggregation_order",
    "alpha_order",
    "alpha_id",
    "alpha",
    "q_order",
    "q_id",
    "q",
    "joint_grid_index",
    "grid_index",
    "c",
    "kappa",
    "gate_half_score_F",
    "mean_fractional_rank",
    "worst_cohort_fractional_rank",
    "mean_metric_regret",
    "near_optimal_rank_tolerance",
    "near_optimal_joint_tuples",
    "near_optimal_alpha_count",
    "near_optimal_alpha_ids",
    "near_optimal_q_count",
    "near_optimal_q_ids",
    "q_is_zero_or_infinity",
    "alpha_is_zero_or_infinity",
    "c_at_boundary",
    "kappa_at_boundary",
    "pdac_metric",
    "pdac_rank",
    "pdac_fractional_rank",
    "pdac_regret",
    "covid_spike_metric",
    "covid_spike_rank",
    "covid_spike_fractional_rank",
    "covid_spike_regret",
    "covid_nonspike_metric",
    "covid_nonspike_rank",
    "covid_nonspike_fractional_rank",
    "covid_nonspike_regret",
    "reference_system_id",
    "pdac_cnap_observed_gate_supported",
    "pdac_cnap_staged_supported",
    "pdac_observed_retained_difference",
    "pdac_limiting_prevalence",
    "pdac_supported_magnitude",
    "pdac_survival_subset_fraction",
    "pdac_mean_retained_effect",
    "covid_spike_cnap_observed_gate_supported",
    "covid_spike_cnap_staged_supported",
    "covid_spike_observed_retained_difference",
    "covid_spike_limiting_prevalence",
    "covid_spike_supported_magnitude",
    "covid_spike_survival_subset_fraction",
    "covid_spike_mean_retained_effect",
    "covid_nonspike_cnap_observed_gate_supported",
    "covid_nonspike_cnap_staged_supported",
    "covid_nonspike_observed_retained_difference",
    "covid_nonspike_limiting_prevalence",
    "covid_nonspike_supported_magnitude",
    "covid_nonspike_survival_subset_fraction",
    "covid_nonspike_mean_retained_effect",
    "system_id",
];

fn system_id(row: &SelectedRow) -> String {
    match row.selection_policy.as_str() {
        "pdac_only" => format!("{}__self_gated_power_hillq_pdac_selected", row.model),
        "all_contexts_equal_weight" => {
            format!(
                "{}__self_gated_power_hillq_all_contexts_selected",
                row.model
            )
        }
        _ => format!("{}__self_gated_power_hillq_selected", row.model),
    }
}

fn selected_fields(row: &SelectedRow) -> Vec<String> {
    let mut f = vec![
        row.selection_policy.clone(),
        row.model.clone(),
        row.branch.clone(),
        row.selection_metric.clone(),
        row.aggregation_order.to_string(),
        row.alpha_order.to_string(),
        row.power.id(),
        fmt_f64(row.power.value()),
        row.q_order.to_string(),
        row.hill.id(),
        fmt_f64(row.hill.value()),
        row.joint_grid_index.to_string(),
        row.grid_index.to_string(),
        fmt_f64(row.c),
        fmt_f64(row.kappa),
        fmt_f64(row.gate_half_score_f),
        fmt_f64(row.mean_fractional_rank),
        fmt_f64(row.worst_cohort_fractional_rank),
        fmt_f64(row.mean_metric_regret),
        fmt_f64(row.near_optimal_rank_tolerance),
        row.near_optimal_joint_tuples.to_string(),
        row.near_optimal_alpha_count.to_string(),
        row.near_optimal_alpha_ids.clone(),
        row.near_optimal_q_count.to_string(),
        row.near_optimal_q_ids.clone(),
        bool_str(row.q_is_zero_or_infinity),
        bool_str(row.power.value() == 0.0 || row.power.value().is_infinite()),
        bool_str(row.c_at_boundary),
        bool_str(row.kappa_at_boundary),
    ];
    for k in 0..3 {
        f.push(fmt_f64(row.cohort_metric[k]));
        f.push(fmt_f64(row.cohort_rank[k]));
        f.push(fmt_f64(row.cohort_fractional_rank[k]));
        f.push(fmt_f64(row.cohort_regret[k]));
    }
    if let Some(evidence) = &row.cnap_evidence {
        f.push(evidence.reference_system_id.clone());
        for outcome in &evidence.cohort_outcomes {
            f.push(bool_str(outcome.observed_gate_supported));
            f.push(bool_str(outcome.staged_supported));
            f.push(fmt_f64(outcome.observed_retained_difference));
            f.push(fmt_f64(outcome.limiting_prevalence));
            f.push(outcome.supported_magnitude.map(fmt_f64).unwrap_or_default());
            f.push(
                outcome
                    .survival_subset_fraction
                    .map(fmt_f64)
                    .unwrap_or_default(),
            );
            f.push(
                outcome
                    .mean_retained_effect
                    .map(fmt_f64)
                    .unwrap_or_default(),
            );
        }
    } else {
        f.extend(std::iter::repeat_n(String::new(), 22));
    }
    f.push(system_id(row));
    f
}

const AUDIT_HEADER: [&str; 14] = [
    "task_id",
    "source_variant_column",
    "n_endpoints",
    "n_positive",
    "n_patients",
    "n_unique_candidates",
    "n_scoreable_candidates",
    "n_zero_signal_candidates",
    "n_exact_duplicates_removed",
    "max_reconstruction_max_abs_error",
    "logsumexp_reconstruction_max_abs_error",
    "reference_system_id",
    "reference_score_vector_hash",
    "bundle_max_reconstruction_max_abs_error",
];

fn audit_fields(a: &Audit) -> Vec<String> {
    vec![
        a.task_id.clone(),
        a.source_variant_column.clone(),
        a.n_endpoints.to_string(),
        a.n_positive.to_string(),
        a.n_patients.to_string(),
        a.n_unique_candidates.to_string(),
        a.n_scoreable_candidates.to_string(),
        a.n_zero_signal_candidates.to_string(),
        a.n_exact_duplicates_removed.to_string(),
        fmt_f64(a.max_reconstruction_max_abs_error),
        fmt_f64(a.logsumexp_reconstruction_max_abs_error),
        a.reference_system_id.clone().unwrap_or_default(),
        a.reference_score_vector_hash.clone().unwrap_or_default(),
        a.bundle_max_reconstruction_max_abs_error
            .map(fmt_f64)
            .unwrap_or_default(),
    ]
}

/// AP/AUROC over the whole base grid for one cohort at one Hill order,
/// parallelized across grid points with rayon.
fn sweep_grid(
    labels: &[i8],
    anchors: &[f64],
    corroboration_offer: &[f64],
    base: &BaseGrid,
    solver_absolute_tolerance: f64,
    solver_max_iterations: usize,
) -> Result<(Vec<f64>, Vec<f64>)> {
    let base_len = base.len();
    let metrics: Result<Vec<(f64, f64)>> = (0..base_len)
        .into_par_iter()
        .map(|b| {
            let c = base.c[b];
            let kappa = base.kappa[b];
            let scores: Result<Vec<f64>> = anchors
                .iter()
                .zip(corroboration_offer.iter())
                .map(|(&anchor, &offer)| {
                    self_gated_score(
                        anchor,
                        offer,
                        c,
                        kappa,
                        solver_absolute_tolerance,
                        solver_max_iterations,
                    )
                })
                .collect();
            ap_auc(labels, &scores?)
        })
        .collect();
    Ok(metrics?.into_iter().unzip())
}

struct PrSweep {
    outcomes: Vec<CnapOutcome>,
    empirical_ap: Vec<f64>,
    auroc: Vec<f64>,
    unique_rankings_evaluated: usize,
}

#[allow(clippy::too_many_arguments)]
fn sweep_pr_cnap(
    model: &str,
    cohort: &str,
    labels_i8: &[i8],
    labels_bool: &[bool],
    anchors: &[f64],
    corroboration_offer: &[f64],
    maximum_reference: &[f64],
    base: &BaseGrid,
    contract: &CnapContract,
    selection_seed: u64,
    cache: &mut HashMap<[u8; 32], CnapOutcome>,
    precomputed: Option<&PrecomputedCnap>,
    solver_absolute_tolerance: f64,
    solver_max_iterations: usize,
) -> Result<PrSweep> {
    struct Diagnostic {
        signature: [u8; 32],
        ap: f64,
        auc: f64,
    }

    let diagnostics: Result<Vec<Diagnostic>> = (0..base.len())
        .into_par_iter()
        .map(|b| {
            let scores: Result<Vec<f64>> = anchors
                .iter()
                .zip(corroboration_offer)
                .map(|(&anchor, &offer)| {
                    self_gated_score(
                        anchor,
                        offer,
                        base.c[b],
                        base.kappa[b],
                        solver_absolute_tolerance,
                        solver_max_iterations,
                    )
                })
                .collect();
            let scores = scores?;
            let (ap, auc) = ap_auc(labels_i8, &scores)?;
            Ok(Diagnostic {
                signature: ranking_signature(&scores),
                ap,
                auc,
            })
        })
        .collect();
    let diagnostics = diagnostics?;

    let mut representatives: HashMap<[u8; 32], usize> = HashMap::new();
    for (b, diagnostic) in diagnostics.iter().enumerate() {
        if !cache.contains_key(&diagnostic.signature) {
            representatives.entry(diagnostic.signature).or_insert(b);
        }
    }
    let representatives: Vec<([u8; 32], usize)> = representatives.into_iter().collect();
    let evaluated: Result<Vec<([u8; 32], CnapOutcome)>> = if let Some(precomputed) = precomputed {
        representatives
            .par_iter()
            .map(|&(signature, _)| Ok((signature, precomputed.outcome(model, cohort, &signature)?)))
            .collect()
    } else {
        representatives
            .par_iter()
            .map(|&(signature, b)| {
                let scores: Result<Vec<f64>> = anchors
                    .iter()
                    .zip(corroboration_offer)
                    .map(|(&anchor, &offer)| {
                        self_gated_score(
                            anchor,
                            offer,
                            base.c[b],
                            base.kappa[b],
                            solver_absolute_tolerance,
                            solver_max_iterations,
                        )
                    })
                    .collect();
                let scores = scores?;
                let (paired, observed) =
                    observed_gate(&scores, maximum_reference, labels_bool, contract)?;
                let completed =
                    complete_forward_assessment(&paired, observed, contract, selection_seed)?;
                Ok((signature, completed))
            })
            .collect()
    };
    for (signature, outcome) in evaluated? {
        cache.insert(signature, outcome);
    }
    let outcomes = diagnostics
        .iter()
        .map(|diagnostic| {
            cache
                .get(&diagnostic.signature)
                .cloned()
                .ok_or_else(|| SelectionError::msg("missing cached paired-CNAP result"))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(PrSweep {
        outcomes,
        empirical_ap: diagnostics.iter().map(|row| row.ap).collect(),
        auroc: diagnostics.iter().map(|row| row.auc).collect(),
        unique_rankings_evaluated: representatives.len(),
    })
}

struct Loaded {
    task: Task,
    endpoint_ids: Vec<String>,
}

struct GroupOutput {
    selections: Vec<SelectedRow>,
    leave_one_out: Vec<LeaveOneOutRow>,
    audits: Vec<Audit>,
}

#[allow(clippy::too_many_arguments)]
fn process_group(
    model: &str,
    branch: &str,
    source_root: &Path,
    orders: &[AggregationOrder],
    base: &BaseGrid,
    authorities: &BTreeMap<String, Vec<AuthEndpoint>>,
    surfaces_dir: &Path,
    loaded: &mut BTreeMap<(String, String, String), Loaded>,
    source_files: &mut Vec<PathBuf>,
    solver_absolute_tolerance: f64,
    solver_max_iterations: usize,
    near_optimal_rank_tolerance: f64,
    pr_bundle: &LoadedBundle,
    cnap_contract: &CnapContract,
    precomputed_cnap: Option<&PrecomputedCnap>,
) -> Result<GroupOutput> {
    let base_len = base.len();
    let joint_len = base_len * orders.len();

    // per-cohort selection-metric vectors over the joint grid, plus full
    // (ap, auc) surfaces for the metric table.
    let mut cohort_values: Vec<Vec<f64>> = Vec::with_capacity(3);
    let mut cohort_ap: Vec<Vec<f64>> = Vec::with_capacity(3);
    let mut cohort_auc: Vec<Vec<f64>> = Vec::with_capacity(3);
    let mut cohort_cnap: Vec<Vec<CnapOutcome>> = Vec::with_capacity(3);
    let mut reference_system_ids: Vec<String> = Vec::with_capacity(3);
    let mut reference_hashes: Vec<String> = Vec::with_capacity(3);
    let mut selection_seeds: Vec<u64> = Vec::with_capacity(3);
    let mut audits: Vec<Audit> = Vec::with_capacity(3);

    for cohort in SELECTION_COHORTS {
        let (task, mut audit) = load_task(source_root, cohort, model, branch)?;
        let authority = authorities
            .get(cohort)
            .ok_or_else(|| SelectionError::msg(format!("missing authority for {cohort}")))?;
        let endpoint_ids = validate_task_labels(&task, authority)?;
        for path in &task.source_files {
            source_files.push(path.clone());
        }
        let labels = task.labels();
        let labels_bool: Vec<bool> = labels.iter().map(|&label| label == 1).collect();

        let pr_reference = if branch == "pr" {
            let (reference_id, reference_scores, reference_hash) =
                aligned_maximum_reference(pr_bundle, cohort, model, &endpoint_ids, &labels)?;
            let reconstructed_max: Vec<f64> = task
                .raw_candidates
                .iter()
                .map(|roster| {
                    if roster.is_empty() {
                        FLOOR
                    } else {
                        max_of(roster)
                    }
                })
                .collect();
            let bundle_error = reconstructed_max
                .iter()
                .zip(&reference_scores)
                .map(|(&left, &right)| (left - right).abs())
                .fold(0.0_f64, f64::max);
            if bundle_error > 3e-5 {
                return Err(SelectionError::msg(format!(
                    "model-matched bundle maximum reconstruction failed for {model}/{cohort}: {bundle_error}"
                )));
            }
            let seed = cnap_contract.selection_seed(cohort)?;
            audit.reference_system_id = Some(reference_id.clone());
            audit.reference_score_vector_hash = Some(reference_hash.clone());
            audit.bundle_max_reconstruction_max_abs_error = Some(bundle_error);
            reference_system_ids.push(reference_id);
            reference_hashes.push(reference_hash);
            selection_seeds.push(seed);
            Some((reference_scores, seed))
        } else {
            None
        };

        let mut ap = Vec::with_capacity(joint_len);
        let mut auc = Vec::with_capacity(joint_len);
        let mut cnap = Vec::with_capacity(joint_len);
        let mut cnap_cache: HashMap<[u8; 32], CnapOutcome> = HashMap::new();
        let mut unique_rankings = 0usize;
        for order in orders {
            let (anchors, corroboration_offer) =
                adaptive_components(&task.raw_candidates, order.power, order.hill)?;
            if let Some((reference_scores, seed)) = &pr_reference {
                let mut block = sweep_pr_cnap(
                    model,
                    cohort,
                    &labels,
                    &labels_bool,
                    &anchors,
                    &corroboration_offer,
                    reference_scores,
                    base,
                    cnap_contract,
                    *seed,
                    &mut cnap_cache,
                    precomputed_cnap,
                    solver_absolute_tolerance,
                    solver_max_iterations,
                )?;
                unique_rankings += block.unique_rankings_evaluated;
                ap.append(&mut block.empirical_ap);
                auc.append(&mut block.auroc);
                cnap.append(&mut block.outcomes);
            } else {
                let (mut ap_block, mut auc_block) = sweep_grid(
                    &labels,
                    &anchors,
                    &corroboration_offer,
                    base,
                    solver_absolute_tolerance,
                    solver_max_iterations,
                )?;
                ap.append(&mut ap_block);
                auc.append(&mut auc_block);
            }
        }
        let selected: Vec<f64> = if branch == "pr" {
            cnap.iter().map(CnapOutcome::ranking_magnitude).collect()
        } else {
            auc.clone()
        };
        cohort_values.push(selected);
        cohort_ap.push(ap);
        cohort_auc.push(auc);
        if branch == "pr" {
            cohort_cnap.push(cnap);
        }
        audits.push(audit.clone());
        loaded.insert(
            (model.to_string(), branch.to_string(), cohort.to_string()),
            Loaded { task, endpoint_ids },
        );
        if branch == "pr" {
            eprintln!(
                "evaluated {model}/{branch}/{cohort}: {joint_len} joint grid points, {unique_rankings} unique paired-CNAP rankings"
            );
        } else {
            eprintln!("evaluated {model}/{branch}/{cohort}: {joint_len} joint grid points");
        }
    }

    if branch == "pr" {
        let pending = select_adaptive_hillq::cnap::pending_grid_indices(&cohort_cnap);
        if !pending.is_empty() {
            return Err(SelectionError::PendingCnapSelection(Box::new(
                serde_json::json!({
                    "schema_version": 1, "status": "pending_numerical_resolution",
                    "model": model, "branch": branch, "pending_joint_grid_indices": pending,
                    "cohorts": SELECTION_COHORTS, "cohort_outcomes": cohort_cnap,
                    "selection_finalized": false,
                    "reason": "Eligibility remains unresolved; no candidate is excluded on that basis",
                    "refinement_multiplier": select_adaptive_hillq::cnap::SEARCH_REFINEMENT_MULTIPLIER,
                }),
            )));
        }
    }

    let cohort_array: [Vec<f64>; 3] = [
        cohort_values[0].clone(),
        cohort_values[1].clone(),
        cohort_values[2].clone(),
    ];
    let ranking = JointRanking::build(cohort_array);
    let mut selections = if branch == "pr" {
        let pdac_eligible: Vec<bool> = (0..joint_len)
            .map(|j| cohort_cnap[0][j].staged_supported)
            .collect();
        let all_eligible: Vec<bool> = (0..joint_len)
            .map(|j| (0..3).all(|k| cohort_cnap[k][j].staged_supported))
            .collect();
        let pdac_survival: Vec<f64> = (0..joint_len)
            .map(|j| cohort_cnap[0][j].ranking_survival())
            .collect();
        let all_survival: Vec<f64> = (0..joint_len)
            .map(|j| {
                (0..3)
                    .map(|k| cohort_cnap[k][j].ranking_survival())
                    .fold(f64::INFINITY, f64::min)
            })
            .collect();
        vec![
            select_group_eligible(
                SelectionPolicy::PdacOnly,
                model,
                branch,
                &ranking,
                base,
                orders,
                near_optimal_rank_tolerance,
                &pdac_eligible,
                &pdac_survival,
            )?,
            select_group_eligible(
                SelectionPolicy::AllContextsEqualWeight,
                model,
                branch,
                &ranking,
                base,
                orders,
                near_optimal_rank_tolerance,
                &all_eligible,
                &all_survival,
            )?,
        ]
    } else {
        vec![
            select_group(
                SelectionPolicy::PdacOnly,
                model,
                branch,
                &ranking,
                base,
                orders,
                near_optimal_rank_tolerance,
            ),
            select_group(
                SelectionPolicy::AllContextsEqualWeight,
                model,
                branch,
                &ranking,
                base,
                orders,
                near_optimal_rank_tolerance,
            ),
        ]
    };
    let cnap_metadata = if branch == "pr" {
        let reference_system_id = reference_system_ids
            .first()
            .cloned()
            .ok_or_else(|| SelectionError::msg("missing PR maximum reference identity"))?;
        if reference_system_ids
            .iter()
            .any(|value| value != &reference_system_id)
        {
            return Err(SelectionError::msg(
                "model-matched maximum reference identity differs across cohorts",
            ));
        }
        let reference_score_vector_hashes: [String; 3] = reference_hashes
            .clone()
            .try_into()
            .map_err(|_| SelectionError::msg("expected three PR reference score hashes"))?;
        let selection_seed_array: [u64; 3] = selection_seeds
            .clone()
            .try_into()
            .map_err(|_| SelectionError::msg("expected three PR selection seeds"))?;
        Some((
            reference_system_id,
            reference_score_vector_hashes,
            selection_seed_array,
        ))
    } else {
        None
    };
    let mut leave_one_out = if branch == "pr" {
        let mut rows = Vec::new();
        for (held_out, held_out_cohort) in SELECTION_COHORTS.iter().enumerate() {
            let training: Vec<usize> = (0..SELECTION_COHORTS.len())
                .filter(|&k| k != held_out)
                .collect();
            let eligible: Vec<bool> = (0..joint_len)
                .map(|j| training.iter().all(|&k| cohort_cnap[k][j].staged_supported))
                .collect();
            let survival: Vec<f64> = (0..joint_len)
                .map(|j| {
                    training
                        .iter()
                        .map(|&k| cohort_cnap[k][j].ranking_survival())
                        .fold(f64::INFINITY, f64::min)
                })
                .collect();
            let selected = select_group_for_indices_eligible(
                "leave_one_cohort_out",
                &training,
                model,
                branch,
                &ranking,
                base,
                orders,
                near_optimal_rank_tolerance,
                &eligible,
                &survival,
            )?;
            let j = selected.joint_grid_index;
            rows.push(LeaveOneOutRow {
                held_out_cohort: (*held_out_cohort).to_string(),
                held_out_metric: ranking.metric[held_out][j],
                held_out_fractional_rank: ranking.fractional[held_out][j],
                held_out_regret: ranking.regret[held_out][j],
                selected,
            });
        }
        rows
    } else {
        leave_one_cohort_out(
            model,
            branch,
            &ranking,
            base,
            orders,
            near_optimal_rank_tolerance,
        )
    };
    if let Some((reference_system_id, reference_score_vector_hashes, seed_array)) = cnap_metadata {
        let attach = |selected: &mut SelectedRow| {
            let j = selected.joint_grid_index;
            selected.cnap_evidence = Some(SelectedCnapEvidence {
                reference_system_id: reference_system_id.clone(),
                reference_score_vector_hashes: reference_score_vector_hashes.clone(),
                selection_seeds: seed_array,
                cohort_outcomes: std::array::from_fn(|k| cohort_cnap[k][j].clone()),
            });
        };
        for selected in &mut selections {
            attach(selected);
        }
        for row in &mut leave_one_out {
            attach(&mut row.selected);
        }
    }

    // --- per-group surfaces ---
    let group_dir = surfaces_dir.join(format!("{branch}__{model}"));
    std::fs::create_dir_all(&group_dir).map_err(|e| SelectionError::Io {
        path: group_dir.clone(),
        source: e,
    })?;

    if branch == "pr" {
        let evidence = serde_json::json!({"schema_version": 1, "cohorts": SELECTION_COHORTS,
            "outcomes": cohort_cnap, "limiting_prevalence_semantics": "best sampled witness",
            "refinement_multiplier": select_adaptive_hillq::cnap::SEARCH_REFINEMENT_MULTIPLIER});
        write_text(
            &group_dir.join("numerical_evidence.json"),
            &serde_json::to_string_pretty(&evidence)
                .map_err(|error| SelectionError::msg(error.to_string()))?,
        )?;
    }

    write_group_metrics(
        &group_dir,
        model,
        branch,
        orders,
        base,
        &ranking,
        &cohort_ap,
        &cohort_auc,
        &cohort_cnap,
    )?;
    write_group_rankings(&group_dir, model, branch, orders, base, &ranking)?;
    let selected_rows: Vec<Vec<String>> = selections.iter().map(selected_fields).collect();
    write_csv(
        &group_dir.join("selected_parameters.csv"),
        &SELECTED_HEADER,
        &selected_rows,
    )?;
    let audit_rows: Vec<Vec<String>> = audits.iter().map(audit_fields).collect();
    write_csv(
        &group_dir.join("baseline_reproduction.csv"),
        &AUDIT_HEADER,
        &audit_rows,
    )?;

    Ok(GroupOutput {
        selections,
        leave_one_out,
        audits,
    })
}

#[allow(clippy::too_many_arguments)]
fn write_group_metrics(
    group_dir: &Path,
    model: &str,
    branch: &str,
    orders: &[AggregationOrder],
    base: &BaseGrid,
    ranking: &JointRanking,
    cohort_ap: &[Vec<f64>],
    cohort_auc: &[Vec<f64>],
    cohort_cnap: &[Vec<CnapOutcome>],
) -> Result<()> {
    let base_len = base.len();
    let joint_len = ranking.count;
    let header = [
        "model",
        "branch",
        "cohort",
        "joint_grid_index",
        "aggregation_order",
        "alpha_order",
        "alpha_id",
        "alpha",
        "q_order",
        "q_id",
        "q",
        "grid_index",
        "c",
        "kappa",
        "average_precision",
        "auroc",
        "selection_metric_rank",
        "selection_metric_fractional_rank",
        "cnap_observed_gate_supported",
        "cnap_staged_supported",
        "observed_retained_cnap_difference",
        "limiting_prevalence",
        "supported_magnitude",
        "survival_subset_fraction",
        "mean_retained_effect",
    ];
    let rows = SELECTION_COHORTS
        .iter()
        .enumerate()
        .flat_map(|(k, cohort)| {
            (0..joint_len).map(move |j| {
                let meta = joint_meta(j, base_len, orders, base);
                let mut row = vec![
                    model.to_string(),
                    branch.to_string(),
                    cohort.to_string(),
                    j.to_string(),
                    meta.aggregation_order.to_string(),
                    meta.alpha_order.to_string(),
                    meta.alpha_id,
                    meta.alpha_str,
                    meta.q_order.to_string(),
                    meta.q_id,
                    meta.q_str,
                    meta.grid_index.to_string(),
                    fmt_f64(meta.c),
                    fmt_f64(meta.kappa),
                    fmt_f64(cohort_ap[k][j]),
                    fmt_f64(cohort_auc[k][j]),
                    fmt_f64(ranking.raw_rank[k][j]),
                    fmt_f64(ranking.fractional[k][j]),
                ];
                if branch == "pr" {
                    let outcome = &cohort_cnap[k][j];
                    row.extend([
                        bool_str(outcome.observed_gate_supported),
                        bool_str(outcome.staged_supported),
                        fmt_f64(outcome.observed_retained_difference),
                        fmt_f64(outcome.limiting_prevalence),
                        outcome.supported_magnitude.map(fmt_f64).unwrap_or_default(),
                        outcome
                            .survival_subset_fraction
                            .map(fmt_f64)
                            .unwrap_or_default(),
                        outcome
                            .mean_retained_effect
                            .map(fmt_f64)
                            .unwrap_or_default(),
                    ]);
                } else {
                    row.extend(std::iter::repeat_n(String::new(), 7));
                }
                row
            })
        });
    write_csv_gz_iter(&group_dir.join("cohort_grid_metrics.csv.gz"), &header, rows)
}

fn write_group_rankings(
    group_dir: &Path,
    model: &str,
    branch: &str,
    orders: &[AggregationOrder],
    base: &BaseGrid,
    ranking: &JointRanking,
) -> Result<()> {
    let base_len = base.len();
    let header = [
        "model",
        "branch",
        "joint_grid_index",
        "aggregation_order",
        "alpha_order",
        "alpha_id",
        "alpha",
        "q_order",
        "q_id",
        "q",
        "grid_index",
        "c",
        "kappa",
        "pdac_fractional_rank",
        "pdac_regret",
        "covid_spike_fractional_rank",
        "covid_spike_regret",
        "covid_nonspike_fractional_rank",
        "covid_nonspike_regret",
        "mean_fractional_rank",
        "worst_cohort_fractional_rank",
        "mean_metric_regret",
    ];
    let rows = (0..ranking.count).map(|j| {
        let meta = joint_meta(j, base_len, orders, base);
        vec![
            model.to_string(),
            branch.to_string(),
            j.to_string(),
            meta.aggregation_order.to_string(),
            meta.alpha_order.to_string(),
            meta.alpha_id,
            meta.alpha_str,
            meta.q_order.to_string(),
            meta.q_id,
            meta.q_str,
            meta.grid_index.to_string(),
            fmt_f64(meta.c),
            fmt_f64(meta.kappa),
            fmt_f64(ranking.fractional[0][j]),
            fmt_f64(ranking.regret[0][j]),
            fmt_f64(ranking.fractional[1][j]),
            fmt_f64(ranking.regret[1][j]),
            fmt_f64(ranking.fractional[2][j]),
            fmt_f64(ranking.regret[2][j]),
            fmt_f64(ranking.mean_fractional_rank[j]),
            fmt_f64(ranking.worst_cohort_fractional_rank[j]),
            fmt_f64(ranking.mean_metric_regret[j]),
        ]
    });
    write_csv_gz_iter(&group_dir.join("aggregate_rankings.csv.gz"), &header, rows)
}

fn run() -> Result<()> {
    let mut args = Args::parse();
    if args.q_values.is_empty() {
        args.q_values = default_q_tokens();
    }
    if !args.solver_absolute_tolerance.is_finite()
        || args.solver_absolute_tolerance <= 0.0
        || args.solver_max_iterations == 0
        || !args.near_optimal_rank_tolerance.is_finite()
        || args.near_optimal_rank_tolerance < 0.0
        || args.selection_replications == 0
    {
        return Err(SelectionError::msg(
            "invalid solver or near-optimal tolerance arguments",
        ));
    }
    if let Some(threads) = args.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .map_err(|e| SelectionError::msg(format!("failed to build rayon pool: {e}")))?;
    }

    let source_root = std::fs::canonicalize(&args.source_root).unwrap_or(args.source_root.clone());
    if !source_root.join("manifest.json").is_file() {
        return Err(SelectionError::msg(format!(
            "source root must be a Rust transfer package with manifest.json: {}",
            source_root.display()
        )));
    }
    iris_fullroster_pipeline::transfer::validate_transfer_package(&source_root).map_err(
        |error| {
            SelectionError::msg(format!(
                "Rust transfer package validation failed for {}: {error:#}",
                source_root.display()
            ))
        },
    )?;
    let bundle_root = std::fs::canonicalize(&args.bundle_root).unwrap_or(args.bundle_root.clone());
    let output = args.output.clone();
    if output.exists() {
        return Err(SelectionError::msg(format!(
            "output already exists: {}",
            output.display()
        )));
    }
    let powers = parse_alpha_values(&args.alpha_values)?;
    let hills = parse_q_values(&args.q_values)?;
    let orders = aggregation_orders(&powers, &hills);
    let spec = GridSpec {
        c_min: args.c_min,
        c_max: args.c_max,
        c_step: args.c_step,
        kappa_min: args.kappa_min,
        kappa_max: args.kappa_max,
        kappa_points: args.kappa_points,
    };
    let base = make_grid(&spec)?;
    let label_contracts = validate_label_contract(&bundle_root)?;
    let pr_bundle = load_bundle(&bundle_root.join("pr")).map_err(|error| {
        SelectionError::msg(format!("failed to load authoritative PR bundle: {error}"))
    })?;
    let cnap_contract =
        CnapContract::from_pr_bundle_with_replications(&pr_bundle, args.selection_replications)?;
    let precomputed_cnap = match (&args.cnap_plan, &args.cnap_results) {
        (Some(plan), Some(results)) => Some(load_complete_cache(
            &source_root,
            &bundle_root,
            plan,
            results,
            &powers,
            &hills,
            &spec,
            args.solver_absolute_tolerance,
            args.solver_max_iterations,
            args.selection_replications,
        )?),
        (None, None) => None,
        _ => {
            return Err(SelectionError::msg(
                "--cnap-plan and --cnap-results must be supplied together",
            ));
        }
    };

    let mut authorities: BTreeMap<String, Vec<AuthEndpoint>> = BTreeMap::new();
    for cohort in SELECTION_COHORTS {
        authorities.insert(
            cohort.to_string(),
            authoritative_endpoint_table(&bundle_root, cohort)?,
        );
    }

    // staging directory in output.parent, atomically renamed at the end.
    let parent = output
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&parent).map_err(|e| SelectionError::Io {
        path: parent.clone(),
        source: e,
    })?;
    let staging = parent.join(format!(
        ".{}.tmp-{}",
        output.file_name().and_then(|s| s.to_str()).unwrap_or("out"),
        std::process::id()
    ));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).ok();
    }
    std::fs::create_dir_all(staging.join("surfaces")).map_err(|e| SelectionError::Io {
        path: staging.clone(),
        source: e,
    })?;

    let result = build_outputs(
        &source_root,
        &bundle_root,
        &staging,
        &orders,
        &base,
        &spec,
        &authorities,
        &label_contracts,
        &pr_bundle,
        &cnap_contract,
        precomputed_cnap.as_ref(),
        &args,
    );
    match result {
        Ok(summary_line) => {
            std::fs::rename(&staging, &output).map_err(|e| SelectionError::Io {
                path: output.clone(),
                source: e,
            })?;
            println!("{summary_line}");
            Ok(())
        }
        Err(SelectionError::PendingCnapSelection(evidence)) => {
            write_text(
                &staging.join("pending_selection.json"),
                &serde_json::to_string_pretty(&evidence)
                    .map_err(|error| SelectionError::msg(error.to_string()))?,
            )?;
            std::fs::rename(&staging, &output).map_err(|source| SelectionError::Io {
                path: output.clone(),
                source,
            })?;
            Err(SelectionError::PendingCnapSelection(evidence))
        }
        Err(err) => {
            std::fs::remove_dir_all(&staging).ok();
            Err(err)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_outputs(
    source_root: &Path,
    bundle_root: &Path,
    staging: &Path,
    orders: &[AggregationOrder],
    base: &BaseGrid,
    spec: &GridSpec,
    authorities: &BTreeMap<String, Vec<AuthEndpoint>>,
    label_contracts: &serde_json::Value,
    pr_bundle: &LoadedBundle,
    cnap_contract: &CnapContract,
    precomputed_cnap: Option<&PrecomputedCnap>,
    args: &Args,
) -> Result<String> {
    // Base, alpha, Hill, and complete joint grids.
    let grid_rows: Vec<Vec<String>> = (0..base.len())
        .map(|i| vec![i.to_string(), fmt_f64(base.c[i]), fmt_f64(base.kappa[i])])
        .collect();
    write_csv(
        &staging.join("parameter_grid.csv"),
        &["grid_index", "c", "kappa"],
        &grid_rows,
    )?;
    let alpha_rows: Vec<Vec<String>> = orders
        .iter()
        .filter(|order| order.q_order == 0)
        .map(|order| {
            vec![
                order.alpha_order.to_string(),
                order.power.id(),
                fmt_f64(order.power.value()),
            ]
        })
        .collect();
    write_csv(
        &staging.join("alpha_grid.csv"),
        &["alpha_order", "alpha_id", "alpha"],
        &alpha_rows,
    )?;
    let q_rows: Vec<Vec<String>> = orders
        .iter()
        .filter(|order| order.alpha_order == 0)
        .map(|order| {
            vec![
                order.q_order.to_string(),
                order.hill.id(),
                fmt_f64(order.hill.value()),
            ]
        })
        .collect();
    write_csv(
        &staging.join("q_grid.csv"),
        &["q_order", "q_id", "q"],
        &q_rows,
    )?;
    let joint_rows = (0..base.len() * orders.len()).map(|j| {
        let meta = joint_meta(j, base.len(), orders, base);
        vec![
            j.to_string(),
            meta.aggregation_order.to_string(),
            meta.alpha_order.to_string(),
            meta.alpha_id,
            meta.alpha_str,
            meta.q_order.to_string(),
            meta.q_id,
            meta.q_str,
            meta.grid_index.to_string(),
            fmt_f64(meta.c),
            fmt_f64(meta.kappa),
        ]
    });
    // The authoritative grid is modest enough to retain uncompressed and is
    // the join key for all surface, selection, and diagnostic tables.
    let joint_rows: Vec<Vec<String>> = joint_rows.collect();
    write_csv(
        &staging.join("joint_parameter_grid.csv"),
        &[
            "joint_grid_index",
            "aggregation_order",
            "alpha_order",
            "alpha_id",
            "alpha",
            "q_order",
            "q_id",
            "q",
            "grid_index",
            "c",
            "kappa",
        ],
        &joint_rows,
    )?;

    let surfaces_dir = staging.join("surfaces");
    let mut loaded: BTreeMap<(String, String, String), Loaded> = BTreeMap::new();
    let mut source_files: Vec<PathBuf> = Vec::new();
    let transfer_manifest = source_root.join("manifest.json");
    if transfer_manifest.is_file() {
        source_files.push(transfer_manifest);
    }
    let mut selections: Vec<SelectedRow> = Vec::new();
    let mut leave_one_out: Vec<LeaveOneOutRow> = Vec::new();
    let mut all_audits: Vec<Audit> = Vec::new();

    for model in MODELS {
        for branch in BRANCHES {
            let group = process_group(
                model,
                branch,
                source_root,
                orders,
                base,
                authorities,
                &surfaces_dir,
                &mut loaded,
                &mut source_files,
                args.solver_absolute_tolerance,
                args.solver_max_iterations,
                args.near_optimal_rank_tolerance,
                pr_bundle,
                cnap_contract,
                precomputed_cnap,
            )?;
            selections.extend(group.selections);
            leave_one_out.extend(group.leave_one_out);
            all_audits.extend(group.audits);
        }
    }

    if selections.len() != 2 * N_GROUPS {
        return Err(SelectionError::msg(
            "selection did not produce both policy rows for every group",
        ));
    }
    selections.sort_by(|a, b| {
        a.selection_policy
            .cmp(&b.selection_policy)
            .then_with(|| a.branch.cmp(&b.branch))
            .then_with(|| a.model.cmp(&b.model))
    });

    if let Some(bad) = selections
        .iter()
        .find(|r| r.c_at_boundary || r.kappa_at_boundary)
    {
        return Err(SelectionError::msg(format!(
            "a selected regime lies on the c or kappa grid boundary; enlarge the grid: {}/{} c={} kappa={}",
            bad.model, bad.branch, bad.c, bad.kappa
        )));
    }

    write_selected_parameters(&staging.join("selected_parameters.csv"), &selections)?;
    let pdac: Vec<SelectedRow> = selections
        .iter()
        .filter(|row| row.selection_policy == "pdac_only")
        .cloned()
        .collect();
    let all_contexts: Vec<SelectedRow> = selections
        .iter()
        .filter(|row| row.selection_policy == "all_contexts_equal_weight")
        .cloned()
        .collect();
    write_selected_parameters(&staging.join("selected_parameters_pdac_only.csv"), &pdac)?;
    write_selected_parameters(
        &staging.join("selected_parameters_all_contexts.csv"),
        &all_contexts,
    )?;
    write_leave_one_out(&staging.join("leave_one_cohort_out.csv"), &leave_one_out)?;

    // top-level baseline reproduction, sorted by task_id
    all_audits.sort_by(|a, b| a.task_id.cmp(&b.task_id));
    let audit_rows: Vec<Vec<String>> = all_audits.iter().map(audit_fields).collect();
    write_csv(
        &staging.join("baseline_reproduction.csv"),
        &AUDIT_HEADER,
        &audit_rows,
    )?;

    // selected endpoint scores (recomputed at the selected order/gate)
    write_selected_endpoint_scores(
        &staging.join("selected_endpoint_scores.csv"),
        &selections,
        &loaded,
        pr_bundle,
        args.solver_absolute_tolerance,
        args.solver_max_iterations,
    )?;

    source_files.extend(write_secondary_view_outputs(
        staging,
        source_root,
        &selections,
        &loaded,
        args.solver_absolute_tolerance,
        args.solver_max_iterations,
    )?);

    // manifest + readme
    let mut source_files_unique: Vec<PathBuf> = source_files.clone();
    source_files_unique.sort();
    source_files_unique.dedup();
    write_manifest(
        staging,
        source_root,
        bundle_root,
        orders,
        base,
        spec,
        label_contracts,
        &selections,
        &source_files_unique,
        cnap_contract,
        args,
    )?;
    write_readme(
        staging,
        orders,
        base,
        &selections,
        args.selection_replications,
    )?;

    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "output": args.output.display().to_string(),
        "selection_policies": ["pdac_only", "all_contexts_equal_weight"],
        "selected_parameter_rows": selections.len(),
        "joint_grid_points_per_group": base.len() * orders.len(),
    }))
    .unwrap_or_default())
}

fn write_selected_parameters(path: &Path, selections: &[SelectedRow]) -> Result<()> {
    let rows: Vec<Vec<String>> = selections.iter().map(selected_fields).collect();
    write_csv(path, &SELECTED_HEADER, &rows)
}

fn write_leave_one_out(path: &Path, rows: &[LeaveOneOutRow]) -> Result<()> {
    let header = [
        "model",
        "branch",
        "held_out_cohort",
        "alpha_order",
        "alpha_id",
        "alpha",
        "q_order",
        "q_id",
        "q",
        "joint_grid_index",
        "grid_index",
        "c",
        "kappa",
        "training_mean_fractional_rank",
        "training_worst_cohort_fractional_rank",
        "training_mean_metric_regret",
        "held_out_metric",
        "held_out_fractional_rank",
        "held_out_regret",
        "reference_system_id",
        "held_out_cnap_observed_gate_supported",
        "held_out_cnap_staged_supported",
        "held_out_observed_retained_difference",
        "held_out_limiting_prevalence",
        "held_out_supported_magnitude",
        "held_out_survival_subset_fraction",
    ];
    let mut sorted = rows.to_vec();
    sorted.sort_by(|a, b| {
        a.selected
            .branch
            .cmp(&b.selected.branch)
            .then_with(|| a.selected.model.cmp(&b.selected.model))
            .then_with(|| a.held_out_cohort.cmp(&b.held_out_cohort))
    });
    let output: Vec<Vec<String>> = sorted
        .iter()
        .map(|row| {
            let selected = &row.selected;
            let mut fields = vec![
                selected.model.clone(),
                selected.branch.clone(),
                row.held_out_cohort.clone(),
                selected.alpha_order.to_string(),
                selected.power.id(),
                fmt_f64(selected.power.value()),
                selected.q_order.to_string(),
                selected.hill.id(),
                fmt_f64(selected.hill.value()),
                selected.joint_grid_index.to_string(),
                selected.grid_index.to_string(),
                fmt_f64(selected.c),
                fmt_f64(selected.kappa),
                fmt_f64(selected.mean_fractional_rank),
                fmt_f64(selected.worst_cohort_fractional_rank),
                fmt_f64(selected.mean_metric_regret),
                fmt_f64(row.held_out_metric),
                fmt_f64(row.held_out_fractional_rank),
                fmt_f64(row.held_out_regret),
            ];
            if let Some(evidence) = &selected.cnap_evidence {
                let held_out = SELECTION_COHORTS
                    .iter()
                    .position(|cohort| *cohort == row.held_out_cohort)
                    .expect("held-out cohort is canonical");
                let outcome = &evidence.cohort_outcomes[held_out];
                fields.extend([
                    evidence.reference_system_id.clone(),
                    bool_str(outcome.observed_gate_supported),
                    bool_str(outcome.staged_supported),
                    fmt_f64(outcome.observed_retained_difference),
                    fmt_f64(outcome.limiting_prevalence),
                    outcome.supported_magnitude.map(fmt_f64).unwrap_or_default(),
                    outcome
                        .survival_subset_fraction
                        .map(fmt_f64)
                        .unwrap_or_default(),
                ]);
            } else {
                fields.extend(std::iter::repeat_n(String::new(), 7));
            }
            fields
        })
        .collect();
    write_csv(path, &header, &output)
}

fn write_selected_endpoint_scores(
    path: &Path,
    selections: &[SelectedRow],
    loaded: &BTreeMap<(String, String, String), Loaded>,
    pr_bundle: &LoadedBundle,
    solver_absolute_tolerance: f64,
    solver_max_iterations: usize,
) -> Result<()> {
    let header = [
        "selection_policy",
        "system_id",
        "model",
        "branch",
        "cohort",
        "endpoint_id",
        "patient_id",
        "mutation",
        "long_peptide",
        "label",
        "alpha_id",
        "alpha",
        "q_id",
        "q",
        "c",
        "kappa",
        "self_gated_power_hillq_score",
        "reference_system_id",
        "model_matched_max_score",
    ];
    let mut rows: Vec<Vec<String>> = Vec::new();
    for sel in selections {
        for cohort in SELECTION_COHORTS {
            let key = (sel.model.clone(), sel.branch.clone(), cohort.to_string());
            let entry = loaded
                .get(&key)
                .ok_or_else(|| SelectionError::msg(format!("missing cached task for {key:?}")))?;
            let (anchors, offers) =
                adaptive_components(&entry.task.raw_candidates, sel.power, sel.hill)?;
            let reference = if sel.branch == "pr" {
                let labels = entry.task.labels();
                let (reference_id, scores, _) = aligned_maximum_reference(
                    pr_bundle,
                    cohort,
                    &sel.model,
                    &entry.endpoint_ids,
                    &labels,
                )?;
                Some((reference_id, scores))
            } else {
                None
            };
            for (i, endpoint) in entry.task.endpoints.iter().enumerate() {
                let score = self_gated_score(
                    anchors[i],
                    offers[i],
                    sel.c,
                    sel.kappa,
                    solver_absolute_tolerance,
                    solver_max_iterations,
                )?;
                let mut row = vec![
                    sel.selection_policy.clone(),
                    system_id(sel),
                    sel.model.clone(),
                    sel.branch.clone(),
                    cohort.to_string(),
                    entry.endpoint_ids[i].clone(),
                    endpoint.patient_id.clone(),
                    endpoint.mutation.clone(),
                    endpoint.long_peptide.clone(),
                    endpoint.label.to_string(),
                    sel.power.id(),
                    fmt_f64(sel.power.value()),
                    sel.hill.id(),
                    fmt_f64(sel.hill.value()),
                    fmt_f64(sel.c),
                    fmt_f64(sel.kappa),
                    fmt_f64(score),
                ];
                if let Some((reference_id, reference_scores)) = &reference {
                    row.push(reference_id.clone());
                    row.push(fmt_f64(reference_scores[i]));
                } else {
                    row.push(String::new());
                    row.push(String::new());
                }
                rows.push(row);
            }
        }
    }
    write_csv(path, &header, &rows)
}

fn write_secondary_view_outputs(
    staging: &Path,
    source_root: &Path,
    selections: &[SelectedRow],
    primary_loaded: &BTreeMap<(String, String, String), Loaded>,
    solver_absolute_tolerance: f64,
    solver_max_iterations: usize,
) -> Result<Vec<PathBuf>> {
    let manifest_path = source_root.join("manifest.json");
    let manifest_text =
        std::fs::read_to_string(&manifest_path).map_err(|source| SelectionError::Io {
            path: manifest_path.clone(),
            source,
        })?;
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_text).map_err(|source| SelectionError::Json {
            path: manifest_path.clone(),
            source,
        })?;
    let jobs = manifest["jobs"]
        .as_array()
        .ok_or_else(|| SelectionError::msg("transfer manifest jobs is not an array"))?;
    let primary_input_views: BTreeMap<String, String> = jobs
        .iter()
        .filter(|job| job["view_role"].as_str() == Some("primary"))
        .filter_map(|job| {
            Some((
                job["cohort"].as_str()?.to_owned(),
                job["input_view_id"].as_str()?.to_owned(),
            ))
        })
        .collect();
    let mut views = BTreeMap::<String, (String, String)>::new();
    for job in jobs {
        if job["view_role"].as_str() != Some("secondary") {
            continue;
        }
        if job["selection_eligible"].as_bool() != Some(false)
            || job["bundle_eligible"].as_bool() != Some(false)
        {
            return Err(SelectionError::msg(
                "secondary transfer job is unexpectedly selection- or bundle-eligible",
            ));
        }
        let view_id = job["cohort"]
            .as_str()
            .ok_or_else(|| SelectionError::msg("secondary job lacks cohort/view identity"))?;
        let parent = job["parent_cohort"]
            .as_str()
            .ok_or_else(|| SelectionError::msg("secondary job lacks parent cohort"))?;
        let parent_view = primary_input_views.get(parent).ok_or_else(|| {
            SelectionError::msg(format!(
                "secondary view {view_id} lacks a primary parent job"
            ))
        })?;
        let identity = (parent.to_owned(), parent_view.clone());
        if let Some(previous) = views.insert(view_id.to_owned(), identity.clone())
            && previous != identity
        {
            return Err(SelectionError::msg(format!(
                "secondary view {view_id} has conflicting parents"
            )));
        }
    }

    let mut source_files = Vec::new();
    for (view_id, (parent_cohort, parent_view_id)) in views {
        let output = staging.join("secondary_views").join(&view_id);
        std::fs::create_dir_all(&output).map_err(|source| SelectionError::Io {
            path: output.clone(),
            source,
        })?;
        let mut tasks = BTreeMap::<(String, String), Task>::new();
        for model in MODELS {
            for branch in BRANCHES {
                let transfer_view = format!("secondary/{view_id}");
                let (task, _) = load_task(source_root, &transfer_view, model, branch)?;
                source_files.extend(task.source_files.iter().cloned());
                tasks.insert((model.to_owned(), branch.to_owned()), task);
            }
        }

        let mut endpoint_rows = Vec::new();
        let mut metric_rows = Vec::new();
        for selection in selections {
            let task = &tasks[&(selection.model.clone(), selection.branch.clone())];
            let (anchors, offers) =
                adaptive_components(&task.raw_candidates, selection.power, selection.hill)?;
            let mut scores = Vec::with_capacity(task.endpoints.len());
            for (index, endpoint) in task.endpoints.iter().enumerate() {
                let score = self_gated_score(
                    anchors[index],
                    offers[index],
                    selection.c,
                    selection.kappa,
                    solver_absolute_tolerance,
                    solver_max_iterations,
                )?;
                scores.push(score);
                endpoint_rows.push(vec![
                    view_id.clone(),
                    parent_view_id.clone(),
                    selection.selection_policy.clone(),
                    system_id(selection),
                    selection.model.clone(),
                    selection.branch.clone(),
                    endpoint.patient_id.clone(),
                    endpoint.mutation.clone(),
                    endpoint.long_peptide.clone(),
                    endpoint.label.to_string(),
                    selection.power.id(),
                    fmt_f64(selection.power.value()),
                    selection.hill.id(),
                    fmt_f64(selection.hill.value()),
                    fmt_f64(selection.c),
                    fmt_f64(selection.kappa),
                    fmt_f64(score),
                ]);
            }
            let labels = task.labels();
            let (view_ap, view_auc) = ap_auc(&labels, &scores)?;
            let primary_key = (
                selection.model.clone(),
                selection.branch.clone(),
                parent_cohort.clone(),
            );
            let primary = primary_loaded.get(&primary_key).ok_or_else(|| {
                SelectionError::msg(format!("missing primary task for {primary_key:?}"))
            })?;
            let (primary_anchors, primary_offers) = adaptive_components(
                &primary.task.raw_candidates,
                selection.power,
                selection.hill,
            )?;
            let primary_scores = primary_anchors
                .iter()
                .zip(&primary_offers)
                .map(|(&anchor, &offer)| {
                    self_gated_score(
                        anchor,
                        offer,
                        selection.c,
                        selection.kappa,
                        solver_absolute_tolerance,
                        solver_max_iterations,
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            let (primary_ap, primary_auc) = ap_auc(&primary.task.labels(), &primary_scores)?;
            let metric = if selection.branch == "pr" {
                (view_ap, primary_ap)
            } else {
                (view_auc, primary_auc)
            };
            let candidate_count: usize = task.raw_candidates.iter().map(Vec::len).sum();
            let completed_zeros = task
                .raw_candidates
                .iter()
                .flatten()
                .filter(|&&value| value <= FLOOR)
                .count();
            metric_rows.push(vec![
                view_id.clone(),
                parent_view_id.clone(),
                selection.selection_policy.clone(),
                system_id(selection),
                selection.model.clone(),
                selection.branch.clone(),
                task.endpoints.len().to_string(),
                labels
                    .iter()
                    .filter(|&&label| label == 1)
                    .count()
                    .to_string(),
                candidate_count.to_string(),
                completed_zeros.to_string(),
                fmt_f64(completed_zeros as f64 / candidate_count as f64),
                fmt_f64(view_ap),
                fmt_f64(view_auc),
                fmt_f64(metric.0),
                fmt_f64(metric.1),
                fmt_f64(metric.0 - metric.1),
                "true".to_owned(),
            ]);
        }
        write_csv(
            &output.join("selected_endpoint_scores.csv"),
            &[
                "view_id",
                "parent_view_id",
                "selection_policy",
                "system_id",
                "model",
                "branch",
                "patient_id",
                "mutation",
                "long_peptide",
                "label",
                "alpha_id",
                "alpha",
                "q_id",
                "q",
                "c",
                "kappa",
                "self_gated_power_hillq_score",
            ],
            &endpoint_rows,
        )?;
        write_csv(
            &output.join("metrics.csv"),
            &[
                "view_id",
                "parent_view_id",
                "selection_policy",
                "system_id",
                "model",
                "branch",
                "n_endpoints",
                "n_positive",
                "n_candidate_contributions",
                "n_completed_zero_contributions",
                "completed_zero_fraction",
                "empirical_average_precision",
                "empirical_auroc",
                "branch_metric",
                "parent_branch_metric",
                "delta_from_parent",
                "conditional_post_selection",
            ],
            &metric_rows,
        )?;
        let view_manifest = serde_json::json!({
            "schema_version": 1,
            "view_id": view_id,
            "parent_view_id": parent_view_id,
            "parent_cohort_path": parent_cohort,
            "role": "secondary",
            "selection_eligible": false,
            "bundle_eligible": false,
            "parameter_source": "parameters selected exclusively from primary transfer views",
            "interpretation": "conditional post-selection diagnostic; not an independent selection cohort or tournament vote",
            "transfer_manifest_sha256": sha256_file(&manifest_path)?,
            "selected_endpoint_scores_sha256": sha256_file(&output.join("selected_endpoint_scores.csv"))?,
            "metrics_sha256": sha256_file(&output.join("metrics.csv"))?,
        });
        write_text(
            &output.join("manifest.json"),
            &(serde_json::to_string_pretty(&view_manifest).map_err(|error| {
                SelectionError::msg(format!("cannot serialize secondary-view manifest: {error}"))
            })? + "\n"),
        )?;
    }
    Ok(source_files)
}

#[allow(clippy::too_many_arguments)]
fn write_manifest(
    staging: &Path,
    source_root: &Path,
    bundle_root: &Path,
    orders: &[AggregationOrder],
    base: &BaseGrid,
    spec: &GridSpec,
    label_contracts: &serde_json::Value,
    selections: &[SelectedRow],
    source_files: &[PathBuf],
    cnap_contract: &CnapContract,
    args: &Args,
) -> Result<()> {
    let source_hashes: serde_json::Map<String, serde_json::Value> = source_files
        .iter()
        .map(|p| Ok((p.display().to_string(), serde_json::json!(sha256_file(p)?))))
        .collect::<Result<_>>()?;

    let mut base_bundle_files: Vec<PathBuf> = Vec::new();
    for branch in BRANCHES {
        for name in [
            "bundle_manifest.json",
            "systems.json",
            "tournament_spec.json",
        ] {
            base_bundle_files.push(bundle_root.join(branch).join(name));
        }
    }
    for cohort in SELECTION_COHORTS {
        for name in [
            "endpoints.csv",
            "endpoint_identities.csv",
            "scores.csv",
            "source_provenance.json",
        ] {
            base_bundle_files.push(
                bundle_root
                    .join("pr")
                    .join("evaluations")
                    .join(cohort)
                    .join(name),
            );
        }
    }
    base_bundle_files.sort();
    let bundle_hashes: serde_json::Map<String, serde_json::Value> = base_bundle_files
        .iter()
        .map(|p| Ok((p.display().to_string(), serde_json::json!(sha256_file(p)?))))
        .collect::<Result<_>>()?;
    let mut base_bundle_content_hashes = serde_json::Map::new();
    for branch in BRANCHES {
        let path = bundle_root.join(branch).join("bundle_manifest.json");
        let text = std::fs::read_to_string(&path).map_err(|source| SelectionError::Io {
            path: path.clone(),
            source,
        })?;
        let manifest: serde_json::Value =
            serde_json::from_str(&text).map_err(|source| SelectionError::Json {
                path: path.clone(),
                source,
            })?;
        let content_hash = manifest["bundle_content_hash"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                SelectionError::msg(format!(
                    "base bundle manifest lacks bundle_content_hash: {}",
                    path.display()
                ))
            })?;
        base_bundle_content_hashes.insert(branch.to_string(), serde_json::json!(content_hash));
    }
    let transfer_manifest_sha256 = sha256_file(&source_root.join("manifest.json"))?;

    // hash the generated report files
    let mut generated: Vec<PathBuf> = Vec::new();
    collect_files(staging, &mut generated)?;
    generated.sort();
    let generated_hashes: serde_json::Map<String, serde_json::Value> = generated
        .iter()
        .map(|p| {
            let rel = p.strip_prefix(staging).unwrap_or(p).display().to_string();
            Ok((rel, serde_json::json!(sha256_file(p)?)))
        })
        .collect::<Result<_>>()?;
    let executable = std::env::current_exe().map_err(|source| SelectionError::Io {
        path: PathBuf::from("<current-executable>"),
        source,
    })?;
    let executable_hash = sha256_file(&executable)?;

    let selected_params: Vec<serde_json::Value> = selections
        .iter()
        .map(|r| {
            let paired_cnap = r.cnap_evidence.as_ref().map(|evidence| {
                serde_json::json!({
                    "reference_system_id": evidence.reference_system_id,
                    "reference_score_vector_hashes": SELECTION_COHORTS.iter().enumerate()
                        .map(|(k, cohort)| (cohort.to_string(), serde_json::json!(evidence.reference_score_vector_hashes[k])))
                        .collect::<serde_json::Map<_, _>>(),
                    "selection_seeds": SELECTION_COHORTS.iter().enumerate()
                        .map(|(k, cohort)| (cohort.to_string(), serde_json::json!(evidence.selection_seeds[k])))
                        .collect::<serde_json::Map<_, _>>(),
                    "cohort_outcomes": SELECTION_COHORTS.iter().enumerate()
                        .map(|(k, cohort)| {
                            let outcome = &evidence.cohort_outcomes[k];
                            (cohort.to_string(), serde_json::json!({
                                "observed_retained_difference": outcome.observed_retained_difference,
                                "limiting_prevalence": outcome.limiting_prevalence,
                                "observed_gate_supported": outcome.observed_gate_supported,
                                "staged_supported": outcome.staged_supported,
                                "supported_magnitude": outcome.supported_magnitude,
                                "survival_subset_fraction": outcome.survival_subset_fraction,
                                "mean_retained_effect": outcome.mean_retained_effect,
                                "adaptive_empirical_ap": outcome.adaptive_empirical_ap,
                                "maximum_empirical_ap": outcome.maximum_empirical_ap,
                            }))
                        })
                        .collect::<serde_json::Map<_, _>>(),
                })
            });
            serde_json::json!({
                "model": r.model,
                "branch": r.branch,
                "selection_policy": r.selection_policy,
                "selection_metric": r.selection_metric,
                "alpha_id": r.power.id(),
                "alpha": r.power.json(),
                "q_id": r.hill.id(),
                "q": r.hill.json(),
                "c": r.c,
                "kappa": r.kappa,
                "gate_half_score_F": r.gate_half_score_f,
                "mean_fractional_rank": r.mean_fractional_rank,
                "worst_cohort_fractional_rank": r.worst_cohort_fractional_rank,
                "mean_metric_regret": r.mean_metric_regret,
                "system_id": system_id(r),
                "paired_cnap_vs_model_matched_max": paired_cnap,
            })
        })
        .collect();

    let manifest = serde_json::json!({
        "analysis": "component-metric-specific self-gated power-anchor Hill-q L2 parameter selection",
        "schema_version": SCHEMA_VERSION,
        "status": "post_hoc_model_development",
        "implementation": "rust_native",
        "source_root": source_root.display().to_string(),
        "base_bundle_root": bundle_root.display().to_string(),
        "transfer_manifest_sha256": transfer_manifest_sha256,
        "base_bundle_content_hashes": base_bundle_content_hashes,
        "formula": {
            "score": "S solves S = A_alpha + C_alpha_q * sigmoid((c-S)/kappa)",
            "A_alpha": "log generalized power mean of exp(z); alpha=0 is mean(z), alpha=1 is logmeanexp(z), alpha=infinity is max(z)",
            "signal": "F_i = max(exp(z_i)-epsilon, 0)",
            "accumulation_ceiling": "log(epsilon + sum(F_i))",
            "D_q": "1 - 1/N_q",
            "C_alpha_q": "D_q * max(accumulation_ceiling - A_alpha, 0)",
            "N_q": "Hill number of order q on p_i=F_i/sum(F); q=0 counts positive-signal candidates",
            "empty_roster_score": FLOOR,
            "missing_observation_policy": "fail closed; omission floors are not accepted",
            "floor_definition": "ln(1e-12)",
            "all_zero_roster_score": FLOOR,
            "candidate_roster": "complete 9-12-mer roster after exact duplicate removal by endpoint+nmer+normalized-HLA; overlapping and cross-HLA tuples are distinct contributions without an independence claim",
            "certified_interval": "A_alpha <= S <= A_alpha + C_alpha_q <= accumulation_ceiling",
            "solver": {
                "method": "bisection",
                "absolute_tolerance": args.solver_absolute_tolerance,
                "maximum_iterations": args.solver_max_iterations,
            },
        },
        "selection": {
            "cohorts": SELECTION_COHORTS,
            "policies": {
                "pdac_only": "select from PDAC fractional rank and PDAC regret only; COVID cohorts are transport diagnostics",
                "all_contexts_equal_weight": "select using equal-weight PDAC, COVID SPIKE, and COVID NONSPIKE objectives",
            },
            "parameter_scope": "alpha, q, c, and kappa are jointly selected for each component-model and metric branch under each policy, then fixed across cohorts",
            "alpha_values": orders.iter().filter(|o| o.q_order == 0).map(|o| o.power.json()).collect::<Vec<_>>(),
            "q_values": orders.iter().filter(|o| o.alpha_order == 0).map(|o| o.hill.json()).collect::<Vec<_>>(),
            "within_group_primary": "PR: staged-supported replacement of the model-matched maximum in every policy cohort, then minimum policy-specific worst-cohort fractional rank of supported magnitude; ROC: minimum policy-specific worst-cohort AUROC fractional rank",
            "within_group_tie_breaks": [
                "minimum policy-specific mean fractional rank",
                "minimum policy-specific mean metric regret",
                "largest minimum literal-survival subset fraction among exact evidence ties",
                "largest kappa among exact-performance ties",
                "ascending c",
                "declared alpha-q grid order",
            ],
            "near_optimal_worst_rank_tolerance": args.near_optimal_rank_tolerance,
            "leave_one_cohort_out_status": "reported transport diagnostic; a production transport threshold must be preregistered after complete-F diagnosis and before grid selection",
            "pr_metric": "authoritative V17 staged paired CNAP, power-anchor Hill-q forward over the same component model's fixed maximum",
            "pr_selection_computational_design": {
                "replications": args.selection_replications,
                "computational_order": cnap_contract.computational_order(),
                "relationship_to_tournament": "all PR bundle settings are inherited except the explicitly declared finite selection replication roster; the downstream replacement tournament remains unchanged",
            },
            "pr_cluster_execution": args.cnap_plan.as_ref().map(|plan| serde_json::json!({
                "plan": plan.display().to_string(),
                "results": args.cnap_results.as_ref().map(|path| path.display().to_string()),
                "audited_before_reduction": true,
            })),
            "pr_reference": "model-matched fixed maximum from the validated directed-round-robin PR bundle",
            "pr_observed_gate_pruning": "exact: a triple failing the mandatory forward observed gate cannot receive a forward staged-supported verdict",
            "pr_ranking_cache": "exact reuse by canonical adaptive score-ranking and exact-tie signature within each cohort",
            "pr_no_supported_parameter_policy": "fail closed rather than select an unsupported triple",
            "roc_metric": "AUROC with half credit for tied positive-negative pairs",
        },
        "grid": {
            "kind": "one explicitly declared joint alpha-q-c-kappa Cartesian grid",
            "alpha_points": orders.iter().filter(|o| o.q_order == 0).count(),
            "q_points": orders.iter().filter(|o| o.alpha_order == 0).count(),
            "c_min": base.c_min(),
            "c_max": base.c_max(),
            "c_step": spec.c_step,
            "kappa_min": base.kappa_min(),
            "kappa_max": base.kappa_max(),
            "kappa_points": spec.kappa_points,
            "c_kappa_points": base.len(),
            "joint_grid_points_per_group": base.len() * orders.len(),
            "threads": rayon::current_num_threads(),
        },
        "label_contracts": label_contracts,
        "selected_parameters": selected_params,
        "source_files": source_hashes,
        "software_versions": {
            "rustc": option_env!("RUSTC_VERSION").unwrap_or("unknown"),
            "port": env!("CARGO_PKG_VERSION"),
        },
        "executable": {
            "path": executable.display().to_string(),
            "sha256": executable_hash,
        },
        "base_bundle_files": bundle_hashes,
        "generated_files": generated_hashes,
    });
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| SelectionError::Json {
        path: staging.join("selection_manifest.json"),
        source: e,
    })? + "\n";
    write_text(&staging.join("selection_manifest.json"), &text)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).map_err(|e| SelectionError::Io {
        path: dir.to_path_buf(),
        source: e,
    })? {
        let entry = entry.map_err(|e| SelectionError::Io {
            path: dir.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

fn write_readme(
    staging: &Path,
    orders: &[AggregationOrder],
    base: &BaseGrid,
    selections: &[SelectedRow],
    selection_replications: usize,
) -> Result<()> {
    let mut table = String::from(
        "policy                     branch  model            alpha  q_id  c        kappa    worst_fractional_rank\n",
    );
    for r in selections {
        table.push_str(&format!(
            "{:<26} {:<7} {:<16} {:<6} {:<5} {:<8} {:<8} {}\n",
            r.selection_policy,
            r.branch,
            r.model,
            r.power.id(),
            r.hill.id(),
            fmt_f64(r.c),
            fmt_f64(r.kappa),
            fmt_f64(r.worst_cohort_fractional_rank),
        ));
    }
    let readme = format!(
        "# Self-gated power-anchor Hill-q L2 selection\n\n\
This is a post-hoc model-development selection performed before a subsequent\n\
directed round robin. It evaluated **{}** declared `(alpha, q)` pairs crossed\n\
with **{}** `(c, kappa)` pairs, for **{}** tuples per component/metric group.\n\n\
The complete joint grid was selected independently under two policies: PDAC-only\n\
and equal-weight PDAC, COVID SPIKE, and COVID NONSPIKE. The aggregation solves\n\
the authoritative implicit self-gated equation by bisection. Every candidate\n\
must have a complete-F tensor score; omission floors fail closed.\n\n\
For PR, every tuple is assessed directionally against the same component\n\
model's fixed maximum using the complete staged paired-CNAP procedure. The\n\
parameter-selection challenge uses a separately declared finite roster of\n\
**{}** replications; the downstream replacement tournament remains at its\n\
validated bundle contract. A policy selects only among tuples\n\
with a staged-supported forward verdict in every policy cohort; absence of such\n\
a tuple fails closed. Among eligible tuples, worst-cohort fractional rank is\n\
primary, followed by mean fractional rank and mean metric regret. ROC uses the\n\
same maximin ordering on AUROC ranks.\n\n\
## Selected component- and metric-specific parameters\n\n```text\n{}```\n\n\
The compressed full surfaces are retained under `surfaces/`. Because the same\n\
cohorts supplied selection, later tournament evidence using these\n\
scores is conditional post-selection evidence. COVID results for PDAC-only are\n\
directional transport evidence rather than selection inputs.\n",
        orders.len(),
        base.len(),
        base.len() * orders.len(),
        selection_replications,
        table,
    );
    write_text(&staging.join("README.md"), &readme)
}

#[cfg(test)]
mod tests {
    use super::*;
    use select_adaptive_hillq::numeric::{HillOrder, PowerOrder};

    fn selected(policy: &str) -> SelectedRow {
        SelectedRow {
            selection_policy: policy.to_string(),
            model: "full_hla".to_string(),
            branch: "pr".to_string(),
            selection_metric: "average_precision".to_string(),
            aggregation_order: 0,
            alpha_order: 0,
            power: PowerOrder::Finite(0.0),
            q_order: 0,
            hill: HillOrder::Finite(2.0),
            joint_grid_index: 1,
            grid_index: 1,
            c: -1.0,
            kappa: 0.5,
            gate_half_score_f: (-1.0_f64).exp(),
            mean_fractional_rank: 0.1,
            worst_cohort_fractional_rank: 0.2,
            mean_metric_regret: 0.01,
            near_optimal_rank_tolerance: 0.01,
            near_optimal_joint_tuples: 2,
            near_optimal_alpha_count: 1,
            near_optimal_alpha_ids: "a0".to_string(),
            near_optimal_q_count: 1,
            near_optimal_q_ids: "q2".to_string(),
            q_is_zero_or_infinity: false,
            c_at_boundary: false,
            kappa_at_boundary: false,
            cohort_metric: [0.8, 0.7, 0.6],
            cohort_rank: [1.0, 2.0, 3.0],
            cohort_fractional_rank: [0.0, 0.1, 0.2],
            cohort_regret: [0.0, 0.1, 0.2],
            cnap_evidence: None,
        }
    }

    #[test]
    fn selected_csv_contract_and_policy_ids_are_stable() {
        let pdac = selected("pdac_only");
        let all = selected("all_contexts_equal_weight");
        assert_eq!(selected_fields(&pdac).len(), SELECTED_HEADER.len());
        assert_eq!(selected_fields(&all).len(), SELECTED_HEADER.len());
        assert_eq!(
            system_id(&pdac),
            "full_hla__self_gated_power_hillq_pdac_selected"
        );
        assert_eq!(
            system_id(&all),
            "full_hla__self_gated_power_hillq_all_contexts_selected"
        );
    }

    #[test]
    fn ranking_signature_uses_order_and_exact_tie_blocks_only() {
        let first = ranking_signature(&[3.0, 2.0, 2.0, 1.0]);
        let monotone = ranking_signature(&[30.0, 20.0, 20.0, 10.0]);
        let broken_tie = ranking_signature(&[30.0, 21.0, 20.0, 10.0]);
        assert_eq!(first, monotone);
        assert_ne!(first, broken_tie);
    }
}
