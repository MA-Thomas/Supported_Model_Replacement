use crate::{Error, SupportOrder};

pub fn support(effects: &[f64], order: SupportOrder) -> Result<f64, Error> {
    validate(effects)?;
    let k = order.get();
    if k > effects.len() {
        return Err(Error::SupportOrderExceedsList {
            order: k,
            list_length: effects.len(),
        });
    }
    let mut sorted = effects.to_vec();
    sorted.sort_unstable_by(f64::total_cmp);
    Ok(support_sorted(&sorted, k))
}

pub fn monte_carlo_standard_error(effects: &[f64], order: SupportOrder) -> Result<f64, Error> {
    validate(effects)?;
    let k = order.get();
    if k > effects.len() {
        return Err(Error::SupportOrderExceedsList {
            order: k,
            list_length: effects.len(),
        });
    }
    let mut sorted = effects.to_vec();
    sorted.sort_unstable_by(f64::total_cmp);
    Ok(mcse_sorted(&sorted, k))
}

fn support_sorted(sorted: &[f64], k: usize) -> f64 {
    if k == 1 {
        return mean(sorted);
    }
    if sorted.first() == sorted.last() {
        return sorted[0];
    }
    let b = sorted.len();
    let last = b - k;
    let mut weight = k as f64 / b as f64;
    let mut total = 0.0;
    for (index, effect) in sorted.iter().take(last + 1).enumerate() {
        total += weight * effect;
        if index < last {
            weight *= (b - index - k) as f64 / (b - index - 1) as f64;
        }
    }
    total
}

fn mcse_sorted(sorted: &[f64], k: usize) -> f64 {
    let b = sorted.len();
    if b == 1 {
        return 0.0;
    }
    if k == 1 {
        return sample_sd(sorted) / (b as f64).sqrt();
    }

    let mut weighted_prefix = 0.0;
    let mut lower_weight = (k - 1) as f64 / (b - 1) as f64;
    let mut self_weight = 1.0;
    let mut projection = Vec::with_capacity(b);
    for (index, &effect) in sorted.iter().enumerate() {
        projection.push(weighted_prefix + self_weight * effect);
        weighted_prefix += lower_weight * effect;
        if index + 1 < b {
            let numerator = b.saturating_sub(index + k) as f64;
            self_weight = if numerator == 0.0 {
                0.0
            } else {
                self_weight * numerator / (b - index - 1) as f64
            };
            if index + 2 < b {
                lower_weight = if numerator == 0.0 {
                    0.0
                } else {
                    lower_weight * numerator / (b - index - 2) as f64
                };
            }
        }
    }
    k as f64 * sample_sd(&projection) / (b as f64).sqrt()
}

pub(crate) fn mean(values: &[f64]) -> f64 {
    let mut sum = 0.0;
    let mut correction = 0.0;
    for &value in values {
        let adjusted = value - correction;
        let next = sum + adjusted;
        correction = (next - sum) - adjusted;
        sum = next;
    }
    sum / values.len() as f64
}

fn sample_sd(values: &[f64]) -> f64 {
    let average = mean(values);
    let sum = values
        .iter()
        .map(|value| {
            let deviation = value - average;
            deviation * deviation
        })
        .sum::<f64>();
    (sum / (values.len() - 1) as f64).sqrt()
}

fn validate(effects: &[f64]) -> Result<(), Error> {
    if effects.is_empty() {
        return Err(Error::EmptyEffects);
    }
    for (index, &value) in effects.iter().enumerate() {
        if !value.is_finite() {
            return Err(Error::InvalidEffect { index, value });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_k_weights_match_complete_subset_enumeration() {
        let effects = [-0.2, 0.1, 0.4, 0.9, 1.0];
        for k in 1..=effects.len() {
            let estimated = support(&effects, SupportOrder::new(k).unwrap()).unwrap();
            let mut minima = Vec::new();
            enumerate(&effects, k, 0, f64::INFINITY, &mut minima);
            let exact = mean(&minima);
            assert!((estimated - exact).abs() < 1e-14, "K={k}");
        }
    }

    fn enumerate(
        effects: &[f64],
        remaining: usize,
        start: usize,
        minimum: f64,
        out: &mut Vec<f64>,
    ) {
        if remaining == 0 {
            out.push(minimum);
            return;
        }
        for index in start..=effects.len() - remaining {
            enumerate(
                effects,
                remaining - 1,
                index + 1,
                minimum.min(effects[index]),
                out,
            );
        }
    }
}
