use std::cmp::Ordering;

use anyhow::{Result, bail};

use crate::contract::{L2Method, L2Spec};

pub fn log_score(q: f64, p_pos: f64, p_neg: f64, pi: f64, epsilon: f64) -> f32 {
    let floor = epsilon.ln();
    if q <= 0.0 || p_pos <= 0.0 || p_neg <= 0.0 || pi <= 0.0 {
        return floor as f32;
    }
    let log_f = q.ln() + p_pos.ln() + p_neg.ln() + pi.ln();
    if log_f >= floor {
        (log_f + (floor - log_f).exp().ln_1p()) as f32
    } else {
        (floor + (log_f - floor).exp().ln_1p()) as f32
    }
}

pub fn aggregate(scores: &[f32], spec: &L2Spec, empty_floor: f32) -> Result<f32> {
    if scores.is_empty() {
        return Ok(empty_floor);
    }
    match spec.method {
        L2Method::Max => Ok(scores.iter().copied().fold(f32::NEG_INFINITY, f32::max)),
        L2Method::Mean => {
            Ok((scores.iter().map(|&x| x as f64).sum::<f64>() / scores.len() as f64) as f32)
        }
        L2Method::Median => {
            let mut values = scores.to_vec();
            values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            let mid = values.len() / 2;
            if values.len() % 2 == 1 {
                Ok(values[mid])
            } else {
                Ok(((values[mid - 1] as f64 + values[mid] as f64) / 2.0) as f32)
            }
        }
        L2Method::Logsumexp => Ok(logsumexp(scores)),
        L2Method::Logmeanexp => Ok(logsumexp(scores) - (scores.len() as f32).ln()),
        L2Method::TopKMean => top_mean(scores, spec.k.unwrap_or(0)),
        L2Method::TopFractionMean => {
            let fraction = spec.fraction.unwrap_or(f64::NAN);
            top_mean(
                scores,
                ((fraction * scores.len() as f64).ceil() as usize).max(1),
            )
        }
        L2Method::TopKLogsumexp => top_lse(scores, spec.k.unwrap_or(0)),
    }
}

fn top_mean(scores: &[f32], k: usize) -> Result<f32> {
    if k == 0 {
        bail!("top-k must be positive");
    }
    let mut values = scores.to_vec();
    values.sort_by(|a, b| b.partial_cmp(a).unwrap_or(Ordering::Equal));
    values.truncate(k.min(values.len()));
    Ok((values.iter().map(|&x| x as f64).sum::<f64>() / values.len() as f64) as f32)
}

fn top_lse(scores: &[f32], k: usize) -> Result<f32> {
    if k == 0 {
        bail!("top-k must be positive");
    }
    let mut values = scores.to_vec();
    values.sort_by(|a, b| b.partial_cmp(a).unwrap_or(Ordering::Equal));
    values.truncate(k.min(values.len()));
    Ok(logsumexp(&values))
}

pub fn logsumexp(scores: &[f32]) -> f32 {
    if scores.is_empty() {
        return f32::NEG_INFINITY;
    }
    let maximum = scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let sum = scores
        .iter()
        .map(|&score| ((score - maximum) as f64).exp())
        .sum::<f64>();
    (maximum as f64 + sum.ln()) as f32
}

pub fn roc_auc(scores: &[f32], labels: &[u8]) -> f64 {
    let n_positive = labels.iter().filter(|&&label| label == 1).count();
    let n_negative = labels.len().saturating_sub(n_positive);
    if scores.len() != labels.len() || n_positive == 0 || n_negative == 0 {
        return f64::NAN;
    }
    let mut pairs: Vec<_> = scores.iter().copied().zip(labels.iter().copied()).collect();
    pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
    let mut negatives_before = 0usize;
    let mut concordant = 0.0;
    let mut i = 0;
    while i < pairs.len() {
        let mut j = i + 1;
        while j < pairs.len() && pairs[j].0 == pairs[i].0 {
            j += 1;
        }
        let positives = pairs[i..j].iter().filter(|row| row.1 == 1).count();
        let negatives = j - i - positives;
        concordant += positives as f64 * (negatives_before as f64 + 0.5 * negatives as f64);
        negatives_before += negatives;
        i = j;
    }
    concordant / (n_positive * n_negative) as f64
}

pub fn average_precision(scores: &[f32], labels: &[u8]) -> f64 {
    let n_positive = labels.iter().filter(|&&label| label == 1).count();
    if scores.len() != labels.len() || n_positive == 0 {
        return f64::NAN;
    }
    let mut pairs: Vec<_> = scores.iter().copied().zip(labels.iter().copied()).collect();
    pairs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
    let mut cumulative_positive = 0usize;
    let mut ap = 0.0;
    let mut i = 0;
    while i < pairs.len() {
        let mut j = i + 1;
        while j < pairs.len() && pairs[j].0 == pairs[i].0 {
            j += 1;
        }
        let group_positive = pairs[i..j].iter().filter(|row| row.1 == 1).count();
        cumulative_positive += group_positive;
        ap += (group_positive as f64 / n_positive as f64) * (cumulative_positive as f64 / j as f64);
        i = j;
    }
    ap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appended_floors_preserve_max_and_change_mean() {
        let floor = -27.631021;
        let max = L2Spec {
            method: L2Method::Max,
            k: None,
            fraction: None,
        };
        let mean = L2Spec {
            method: L2Method::Mean,
            k: None,
            fraction: None,
        };
        let scoreable = [1.0, 2.0];
        let full = [1.0, 2.0, floor, floor];
        assert_eq!(
            aggregate(&scoreable, &max, floor).unwrap(),
            aggregate(&full, &max, floor).unwrap()
        );
        assert!(
            aggregate(&full, &mean, floor).unwrap() < aggregate(&scoreable, &mean, floor).unwrap()
        );
    }
}
