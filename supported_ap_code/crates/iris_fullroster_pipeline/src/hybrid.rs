//! Production endpoint-local adaptive L2 aggregation.
//!
//! The frozen rule projects one complete peptide--HLA roster onto distinct
//! peptide maxima and distinct HLA maxima. Distinct-peptide breadth supplies a
//! self-gated corroboration term; the second-best distinct HLA supplies a
//! separately bounded evidence-qualification term.

use std::collections::BTreeMap;

use anyhow::{Result, bail};

use crate::contract::AdaptiveL2Spec;

const EPSILON: f64 = 1e-12;
const FLOOR: f64 = -27.631_021_115_928_547;
// Transfer tensors are stored as f32, so an exact ln(1e-12) tensor value can
// round slightly below the f64 contract floor when promoted back to f64.
const TRANSFER_SCORE_FLOOR_TOLERANCE: f64 = 1e-6;

fn normalize_peptide(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}

fn normalize_hla(value: &str) -> String {
    let upper = value.trim().to_ascii_uppercase();
    upper
        .strip_prefix("HLA-")
        .unwrap_or(&upper)
        .replace(['*', ':'], "")
}

fn expit(value: f64) -> f64 {
    if value >= 0.0 {
        1.0 / (1.0 + (-value).exp())
    } else {
        let exponential = value.exp();
        exponential / (1.0 + exponential)
    }
}

fn neumaier_sum(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut sum = 0.0;
    let mut compensation = 0.0;
    for value in values {
        let next = sum + value;
        if sum.abs() >= value.abs() {
            compensation += (sum - next) + value;
        } else {
            compensation += (value - next) + sum;
        }
        sum = next;
    }
    sum + compensation
}

fn self_gated_score(anchor: f64, offer: f64, spec: &AdaptiveL2Spec) -> Result<f64> {
    if offer == 0.0 {
        return Ok(anchor);
    }
    let mut lower = anchor;
    let mut upper = anchor + offer;
    for _ in 0..spec.solver_max_iterations {
        if upper - lower <= spec.solver_absolute_tolerance {
            break;
        }
        let midpoint = lower + 0.5 * (upper - lower);
        let residual = midpoint
            - anchor
            - offer * expit((spec.epitope_gate_center - midpoint) / spec.epitope_gate_width);
        if residual < 0.0 {
            lower = midpoint;
        } else {
            upper = midpoint;
        }
    }
    if upper - lower > spec.solver_absolute_tolerance {
        bail!("frozen adaptive L2 self-gated solver did not converge");
    }
    Ok(lower + 0.5 * (upper - lower))
}

/// Score one complete endpoint roster under the frozen adaptive L2 contract.
///
/// Inputs are aligned candidate log-scores, short-peptide sequences, and HLA
/// identities. Empty and all-zero rosters return the declared floor. When
/// fewer than two distinct HLAs are available, the HLA increment is zero.
pub fn aggregate_frozen_adaptive_l2(
    scores: &[f64],
    peptides: &[String],
    hlas: &[String],
    spec: &AdaptiveL2Spec,
    empty_floor: f64,
) -> Result<f64> {
    spec.validate()?;
    if empty_floor.to_bits() != FLOOR.to_bits() {
        bail!("adaptive L2 requires the frozen ln(1e-12) floor");
    }
    if scores.len() != peptides.len() || scores.len() != hlas.len() {
        bail!("adaptive L2 score, peptide, and HLA rosters are not aligned");
    }
    if scores.is_empty() {
        return Ok(empty_floor);
    }

    let mut peptide_maxima = BTreeMap::<String, f64>::new();
    let mut hla_maxima = BTreeMap::<String, f64>::new();
    for ((&score, peptide), hla) in scores.iter().zip(peptides).zip(hlas) {
        if !score.is_finite() || score < FLOOR - TRANSFER_SCORE_FLOOR_TOLERANCE {
            bail!("adaptive L2 roster contains an invalid Level-1 score");
        }
        let peptide = normalize_peptide(peptide);
        let hla = normalize_hla(hla);
        if peptide.is_empty() || hla.is_empty() {
            bail!("adaptive L2 roster contains an empty peptide or HLA identity");
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
    let mut hla_maxima: Vec<f64> = hla_maxima.into_values().collect();
    peptide_maxima.sort_by(|left, right| right.total_cmp(left));
    hla_maxima.sort_by(|left, right| right.total_cmp(left));

    let anchor = peptide_maxima[0];
    let signals: Vec<f64> = peptide_maxima
        .iter()
        .map(|&score| (EPSILON * (score - FLOOR).exp_m1()).max(0.0))
        .collect();
    let total_signal = neumaier_sum(signals.iter().copied());
    if total_signal == 0.0 {
        return Ok(empty_floor);
    }
    let largest_probability = signals
        .iter()
        .map(|signal| signal / total_signal)
        .fold(f64::NEG_INFINITY, f64::max);
    let breadth = 1.0 - largest_probability;
    let accumulation_ceiling = (EPSILON + total_signal).ln();
    let offer = breadth * (accumulation_ceiling - anchor).max(0.0);
    let epitope_score = self_gated_score(anchor, offer, spec)?;

    let second_hla_gate = hla_maxima.get(1).map_or(0.0, |&second_hla| {
        expit((second_hla - spec.second_hla_threshold) / spec.second_hla_gate_width)
    });
    let score = anchor
        + (1.0 - spec.hla_weight) * (epitope_score - anchor)
        + spec.hla_weight * spec.hla_bonus * second_hla_gate;
    let upper = anchor + (1.0 - spec.hla_weight) * offer + spec.hla_weight * spec.hla_bonus;
    if !score.is_finite() || score < anchor - spec.solver_absolute_tolerance || score > upper {
        bail!("adaptive L2 score escaped its certified bounds");
    }
    Ok(score)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{AdaptiveL2Method, AdaptiveL2Spec};

    fn spec() -> AdaptiveL2Spec {
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
    fn empty_and_all_zero_rosters_return_the_floor() {
        let floor = FLOOR;
        assert_eq!(
            aggregate_frozen_adaptive_l2(&[], &[], &[], &spec(), floor).unwrap(),
            floor
        );
        let quantized_floor = (floor as f32) as f64;
        assert_eq!(
            aggregate_frozen_adaptive_l2(
                &[quantized_floor, quantized_floor],
                &["AAAA".into(), "BBBB".into()],
                &["A0101".into(), "B0702".into()],
                &spec(),
                floor,
            )
            .unwrap(),
            floor
        );
        assert_eq!(
            aggregate_frozen_adaptive_l2(
                &[floor, floor],
                &["AAAA".into(), "BBBB".into()],
                &["A0101".into(), "B0702".into()],
                &spec(),
                floor,
            )
            .unwrap(),
            floor
        );
    }

    #[test]
    fn fewer_than_two_hlas_has_no_hla_increment() {
        let scores = [-2.0, -3.0];
        let peptides = ["AAAA".into(), "BBBB".into()];
        let one_hla = ["A0101".into(), "HLA-A*01:01".into()];
        let observed =
            aggregate_frozen_adaptive_l2(&scores, &peptides, &one_hla, &spec(), FLOOR).unwrap();
        assert!(observed >= -2.0);
        assert!(observed < -2.0 + 0.88);
    }

    #[test]
    fn canonicalization_and_permutation_do_not_change_the_score() {
        let scores = [-2.0, -4.0, -6.4, -7.0];
        let peptides = ["aaaa", "BBBB", "CCCC", "AAAA"].map(String::from);
        let hlas = ["HLA-A*01:01", "B0702", "C0701", "A0101"].map(String::from);
        let forward =
            aggregate_frozen_adaptive_l2(&scores, &peptides, &hlas, &spec(), FLOOR).unwrap();
        let reverse_scores = [-7.0, -6.4, -4.0, -2.0];
        let reverse_peptides = ["AAAA", "CCCC", "BBBB", "AAAA"].map(String::from);
        let reverse_hlas = ["A0101", "C0701", "B0702", "A0101"].map(String::from);
        let reverse = aggregate_frozen_adaptive_l2(
            &reverse_scores,
            &reverse_peptides,
            &reverse_hlas,
            &spec(),
            FLOOR,
        )
        .unwrap();
        assert_eq!(forward.to_bits(), reverse.to_bits());
    }

    #[test]
    fn frozen_golden_score_is_stable() {
        let score = aggregate_frozen_adaptive_l2(
            &[-2.0, -3.0, -6.4, -7.0],
            &["AAAA", "BBBB", "CCCC", "DDDD"].map(String::from),
            &["A0101", "A0101", "B0702", "C0701"].map(String::from),
            &spec(),
            FLOOR,
        )
        .unwrap();
        assert_eq!(score.to_bits(), (-1.876_182_531_029_853_f64).to_bits());
    }
}
