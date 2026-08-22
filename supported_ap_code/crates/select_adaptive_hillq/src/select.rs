//! Joint-grid ranking and per-group selection policies.
//!
//! Every `(q, c, kappa)` triple is ranked jointly within each cohort. Selection
//! is then performed independently for each component/metric group under two
//! declared policies: PDAC-only and equal-weight PDAC/SPIKE/NONSPIKE.

use std::cmp::Ordering;

use crate::cnap::CnapOutcome;
use crate::config::{SELECTION_COHORTS, selection_metric};
use crate::error::{Result, SelectionError};
use crate::grid::BaseGrid;
use crate::numeric::{HillOrder, fractional_rank_desc, max_of, rankdata_average};

fn cmp_f64(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

fn is_close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-8 + 1e-5 * b.abs()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionPolicy {
    PdacOnly,
    AllContextsEqualWeight,
}

impl SelectionPolicy {
    pub fn id(self) -> &'static str {
        match self {
            Self::PdacOnly => "pdac_only",
            Self::AllContextsEqualWeight => "all_contexts_equal_weight",
        }
    }

    pub fn cohort_indices(self) -> &'static [usize] {
        match self {
            Self::PdacOnly => &[0],
            Self::AllContextsEqualWeight => &[0, 1, 2],
        }
    }
}

/// Per-cohort ranks and regrets over the complete joint grid for one group.
pub struct JointRanking {
    pub count: usize,
    pub fractional: [Vec<f64>; 3],
    pub raw_rank: [Vec<f64>; 3],
    pub regret: [Vec<f64>; 3],
    pub metric: [Vec<f64>; 3],
    /// Convenience columns for the all-context equal-weight policy.
    pub mean_fractional_rank: Vec<f64>,
    pub worst_cohort_fractional_rank: Vec<f64>,
    pub mean_metric_regret: Vec<f64>,
}

impl JointRanking {
    pub fn build(cohort_values: [Vec<f64>; 3]) -> Self {
        let count = cohort_values[0].len();
        assert!(cohort_values.iter().all(|v| v.len() == count));
        let fractional: Vec<Vec<f64>> = cohort_values
            .iter()
            .map(|v| fractional_rank_desc(v))
            .collect();
        let raw_rank: Vec<Vec<f64>> = cohort_values
            .iter()
            .map(|v| rankdata_average(&v.iter().map(|&x| -x).collect::<Vec<_>>()))
            .collect();
        let regret: Vec<Vec<f64>> = cohort_values
            .iter()
            .map(|v| {
                let maximum = max_of(v);
                v.iter().map(|&x| maximum - x).collect()
            })
            .collect();

        let objective = objectives(&fractional, &regret, &[0, 1, 2], count);
        let to_array = |values: Vec<Vec<f64>>| -> [Vec<f64>; 3] {
            values.try_into().expect("exactly three cohorts")
        };
        Self {
            count,
            fractional: to_array(fractional),
            raw_rank: to_array(raw_rank),
            regret: to_array(regret),
            metric: cohort_values,
            mean_fractional_rank: objective.0,
            worst_cohort_fractional_rank: objective.1,
            mean_metric_regret: objective.2,
        }
    }
}

fn objectives(
    fractional: &[Vec<f64>],
    regret: &[Vec<f64>],
    cohorts: &[usize],
    count: usize,
) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    assert!(!cohorts.is_empty());
    let mut mean_rank = vec![0.0; count];
    let mut worst_rank = vec![0.0; count];
    let mut mean_regret = vec![0.0; count];
    for j in 0..count {
        worst_rank[j] = f64::NEG_INFINITY;
        for &k in cohorts {
            mean_rank[j] += fractional[k][j];
            worst_rank[j] = worst_rank[j].max(fractional[k][j]);
            mean_regret[j] += regret[k][j];
        }
        let n = cohorts.len() as f64;
        mean_rank[j] /= n;
        mean_regret[j] /= n;
    }
    (mean_rank, worst_rank, mean_regret)
}

type SelectedIndexAndObjectives = (usize, Vec<f64>, Vec<f64>, Vec<f64>);

fn selected_index(
    ranking: &JointRanking,
    base: &BaseGrid,
    cohorts: &[usize],
    eligible: &[bool],
    survival_tie_break: &[f64],
) -> Option<SelectedIndexAndObjectives> {
    assert_eq!(eligible.len(), ranking.count);
    assert_eq!(survival_tie_break.len(), ranking.count);
    let (mean_rank, worst_rank, mean_regret) =
        objectives(&ranking.fractional, &ranking.regret, cohorts, ranking.count);
    let base_len = base.len();
    let mut best = eligible.iter().position(|&value| value)?;
    for j in 0..ranking.count {
        if !eligible[j] || j == best {
            continue;
        }
        let b = j % base_len;
        let best_b = best % base_len;
        let order = cmp_f64(mean_rank[j], mean_rank[best])
            .then_with(|| cmp_f64(worst_rank[j], worst_rank[best]))
            .then_with(|| cmp_f64(mean_regret[j], mean_regret[best]))
            .then_with(|| cmp_f64(-survival_tie_break[j], -survival_tie_break[best]))
            .then_with(|| cmp_f64(-base.kappa[b], -base.kappa[best_b]))
            .then_with(|| cmp_f64(base.c[b], base.c[best_b]))
            .then_with(|| (j / base_len).cmp(&(best / base_len)));
        if order == Ordering::Less {
            best = j;
        }
    }
    Some((best, mean_rank, worst_rank, mean_regret))
}

#[derive(Debug, Clone)]
pub struct SelectedCnapEvidence {
    pub reference_system_id: String,
    pub reference_score_vector_hashes: [String; 3],
    pub selection_seeds: [u64; 3],
    pub cohort_outcomes: [CnapOutcome; 3],
}

#[derive(Debug, Clone)]
pub struct SelectedRow {
    pub selection_policy: String,
    pub model: String,
    pub branch: String,
    pub selection_metric: String,
    pub q_order: usize,
    pub hill: HillOrder,
    pub joint_grid_index: usize,
    pub grid_index: usize,
    pub c: f64,
    pub kappa: f64,
    pub gate_half_score_f: f64,
    pub mean_fractional_rank: f64,
    pub worst_cohort_fractional_rank: f64,
    pub mean_metric_regret: f64,
    pub near_optimal_rank_tolerance: f64,
    pub near_optimal_joint_triples: usize,
    pub near_optimal_q_count: usize,
    pub near_optimal_q_ids: String,
    pub q_is_zero_or_infinity: bool,
    pub c_at_boundary: bool,
    pub kappa_at_boundary: bool,
    pub cohort_metric: [f64; 3],
    pub cohort_rank: [f64; 3],
    pub cohort_fractional_rank: [f64; 3],
    pub cohort_regret: [f64; 3],
    pub cnap_evidence: Option<SelectedCnapEvidence>,
}

#[allow(clippy::too_many_arguments)]
pub fn select_group(
    policy: SelectionPolicy,
    model: &str,
    branch: &str,
    ranking: &JointRanking,
    base: &BaseGrid,
    orders: &[HillOrder],
    near_optimal_rank_tolerance: f64,
) -> SelectedRow {
    let eligible = vec![true; ranking.count];
    let survival = vec![0.0; ranking.count];
    select_group_for_cohorts(
        policy.id(),
        policy.cohort_indices(),
        model,
        branch,
        ranking,
        base,
        orders,
        near_optimal_rank_tolerance,
        &eligible,
        &survival,
    )
    .expect("an unconstrained nonempty grid always has a selected row")
}

/// Select only among triples that have already passed the declared staged
/// evidence eligibility rule. This is used by paired-CNAP selection; the ROC
/// scalar path continues to use `select_group`.
#[allow(clippy::too_many_arguments)]
pub fn select_group_eligible(
    policy: SelectionPolicy,
    model: &str,
    branch: &str,
    ranking: &JointRanking,
    base: &BaseGrid,
    orders: &[HillOrder],
    near_optimal_rank_tolerance: f64,
    eligible: &[bool],
    survival_tie_break: &[f64],
) -> Result<SelectedRow> {
    select_group_for_cohorts(
        policy.id(),
        policy.cohort_indices(),
        model,
        branch,
        ranking,
        base,
        orders,
        near_optimal_rank_tolerance,
        eligible,
        survival_tie_break,
    )
    .ok_or_else(|| {
        SelectionError::msg(format!(
            "no staged-supported Hill-q parameter exists for {model}/{branch}/{}",
            policy.id()
        ))
    })
}

/// Evidence-constrained selection over an explicitly declared cohort subset,
/// used by leave-one-context-out diagnostics.
#[allow(clippy::too_many_arguments)]
pub fn select_group_for_indices_eligible(
    policy_id: &str,
    cohorts: &[usize],
    model: &str,
    branch: &str,
    ranking: &JointRanking,
    base: &BaseGrid,
    orders: &[HillOrder],
    near_optimal_rank_tolerance: f64,
    eligible: &[bool],
    survival_tie_break: &[f64],
) -> Result<SelectedRow> {
    select_group_for_cohorts(
        policy_id,
        cohorts,
        model,
        branch,
        ranking,
        base,
        orders,
        near_optimal_rank_tolerance,
        eligible,
        survival_tie_break,
    )
    .ok_or_else(|| {
        SelectionError::msg(format!(
            "no staged-supported Hill-q parameter exists for {model}/{branch}/{policy_id}"
        ))
    })
}

#[allow(clippy::too_many_arguments)]
fn select_group_for_cohorts(
    policy_id: &str,
    cohorts: &[usize],
    model: &str,
    branch: &str,
    ranking: &JointRanking,
    base: &BaseGrid,
    orders: &[HillOrder],
    near_optimal_rank_tolerance: f64,
    eligible: &[bool],
    survival_tie_break: &[f64],
) -> Option<SelectedRow> {
    let (selected, mean_rank, worst_rank, mean_regret) =
        selected_index(ranking, base, cohorts, eligible, survival_tie_break)?;
    let base_len = base.len();
    let q_order = selected / base_len;
    let grid_index = selected % base_len;
    let hill = orders[q_order];
    let cutoff = mean_rank[selected] + near_optimal_rank_tolerance;
    let near: Vec<usize> = (0..ranking.count)
        .filter(|&j| eligible[j] && mean_rank[j] <= cutoff)
        .collect();
    let mut near_q_orders: Vec<usize> = near.iter().map(|&j| j / base_len).collect();
    near_q_orders.sort_unstable();
    near_q_orders.dedup();
    let near_q_ids = near_q_orders
        .iter()
        .map(|&q| orders[q].id())
        .collect::<Vec<_>>()
        .join("|");
    let c = base.c[grid_index];
    let kappa = base.kappa[grid_index];
    Some(SelectedRow {
        selection_policy: policy_id.to_string(),
        model: model.to_string(),
        branch: branch.to_string(),
        selection_metric: selection_metric(branch).to_string(),
        q_order,
        hill,
        joint_grid_index: selected,
        grid_index,
        c,
        kappa,
        gate_half_score_f: c.exp(),
        mean_fractional_rank: mean_rank[selected],
        worst_cohort_fractional_rank: worst_rank[selected],
        mean_metric_regret: mean_regret[selected],
        near_optimal_rank_tolerance,
        near_optimal_joint_triples: near.len(),
        near_optimal_q_count: near_q_orders.len(),
        near_optimal_q_ids: near_q_ids,
        q_is_zero_or_infinity: hill.value() == 0.0 || hill.value().is_infinite(),
        c_at_boundary: c == base.c_min() || c == base.c_max(),
        kappa_at_boundary: is_close(kappa, base.kappa_min()) || is_close(kappa, base.kappa_max()),
        cohort_metric: std::array::from_fn(|k| ranking.metric[k][selected]),
        cohort_rank: std::array::from_fn(|k| ranking.raw_rank[k][selected]),
        cohort_fractional_rank: std::array::from_fn(|k| ranking.fractional[k][selected]),
        cohort_regret: std::array::from_fn(|k| ranking.regret[k][selected]),
        cnap_evidence: None,
    })
}

#[derive(Debug, Clone)]
pub struct LeaveOneOutRow {
    pub held_out_cohort: String,
    pub selected: SelectedRow,
    pub held_out_metric: f64,
    pub held_out_fractional_rank: f64,
    pub held_out_regret: f64,
}

pub fn leave_one_cohort_out(
    model: &str,
    branch: &str,
    ranking: &JointRanking,
    base: &BaseGrid,
    orders: &[HillOrder],
    near_optimal_rank_tolerance: f64,
) -> Vec<LeaveOneOutRow> {
    let eligible = vec![true; ranking.count];
    let survival = vec![0.0; ranking.count];
    (0..SELECTION_COHORTS.len())
        .map(|held_out| {
            let cohorts: Vec<usize> = (0..SELECTION_COHORTS.len())
                .filter(|&k| k != held_out)
                .collect();
            let selected = select_group_for_cohorts(
                "leave_one_cohort_out",
                &cohorts,
                model,
                branch,
                ranking,
                base,
                orders,
                near_optimal_rank_tolerance,
                &eligible,
                &survival,
            )
            .expect("an unconstrained nonempty grid always has a selected row");
            let j = selected.joint_grid_index;
            LeaveOneOutRow {
                held_out_cohort: SELECTION_COHORTS[held_out].to_string(),
                held_out_metric: ranking.metric[held_out][j],
                held_out_fractional_rank: ranking.fractional[held_out][j],
                held_out_regret: ranking.regret[held_out][j],
                selected,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_grid() -> BaseGrid {
        BaseGrid {
            c_values: vec![-1.0],
            kappa_values: vec![0.25, 0.5],
            c: vec![-1.0, -1.0, -1.0, -1.0],
            kappa: vec![0.25, 0.5, 0.25, 0.5],
        }
    }

    #[test]
    fn policies_can_select_different_joint_triples() {
        let ranking = JointRanking::build([
            vec![1.0, 0.9, 0.1, 0.0],
            vec![0.0, 0.1, 1.0, 0.9],
            vec![0.0, 0.1, 1.0, 0.9],
        ]);
        let grid = tiny_grid();
        let orders = [HillOrder::Finite(1.0)];
        let pdac = select_group(
            SelectionPolicy::PdacOnly,
            "full_hla",
            "pr",
            &ranking,
            &grid,
            &orders,
            0.01,
        );
        let all = select_group(
            SelectionPolicy::AllContextsEqualWeight,
            "full_hla",
            "pr",
            &ranking,
            &grid,
            &orders,
            0.01,
        );
        assert_eq!(pdac.joint_grid_index, 0);
        assert_eq!(all.joint_grid_index, 2);
    }

    #[test]
    fn exact_ties_prefer_larger_kappa_then_declared_q_order() {
        let ranking = JointRanking::build([vec![1.0; 8], vec![1.0; 8], vec![1.0; 8]]);
        let grid = tiny_grid();
        let orders = [HillOrder::Finite(0.0), HillOrder::Infinity];
        let selected = select_group(
            SelectionPolicy::AllContextsEqualWeight,
            "full_hla",
            "roc",
            &ranking,
            &grid,
            &orders,
            0.01,
        );
        assert_eq!(selected.q_order, 0);
        assert_eq!(selected.grid_index, 1);
    }

    #[test]
    fn evidence_eligibility_overrides_a_better_unsupported_metric() {
        let ranking = JointRanking::build([
            vec![1.0, 0.8, 0.7, 0.6],
            vec![1.0, 0.8, 0.7, 0.6],
            vec![1.0, 0.8, 0.7, 0.6],
        ]);
        let selected = select_group_eligible(
            SelectionPolicy::PdacOnly,
            "full_hla",
            "pr",
            &ranking,
            &tiny_grid(),
            &[HillOrder::Finite(1.0)],
            0.01,
            &[false, true, false, false],
            &[0.0, 0.9, 0.0, 0.0],
        )
        .unwrap();
        assert_eq!(selected.joint_grid_index, 1);
    }

    #[test]
    fn evidence_selection_fails_closed_without_an_eligible_triple() {
        let ranking = JointRanking::build([vec![1.0; 4], vec![1.0; 4], vec![1.0; 4]]);
        let result = select_group_eligible(
            SelectionPolicy::AllContextsEqualWeight,
            "full_hla",
            "pr",
            &ranking,
            &tiny_grid(),
            &[HillOrder::Finite(1.0)],
            0.01,
            &[false; 4],
            &[0.0; 4],
        );
        assert!(result.is_err());
    }
}
