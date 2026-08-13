use crate::{Cnap, SupportOrder, SupportedApError, SupportedCnap};

/// Estimate `S_K`, the mean minimum over all distinct subsets of `K` effects.
///
/// The implementation uses order-statistic weights and does not enumerate
/// subsets. Its cost is `O(B log B)` for `B = effects.len()`.
///
/// # Errors
///
/// Returns an error for an empty or non-finite effect sample or when `order`
/// exceeds the sample size.
pub fn support_k(effects: &[f64], order: SupportOrder) -> Result<f64, SupportedApError> {
    validate_effects(effects)?;
    let order = order.get();
    if order > effects.len() {
        return Err(SupportedApError::SupportOrderExceedsSample {
            order,
            sample_size: effects.len(),
        });
    }

    let mut sorted = effects.to_vec();
    sorted.sort_unstable_by(f64::total_cmp);
    Ok(support_sorted(&sorted, order))
}

/// Estimate the two-replication support operator from scalar effects.
///
/// # Errors
///
/// Returns an error unless at least two finite effects are supplied.
pub fn support_two(effects: &[f64]) -> Result<f64, SupportedApError> {
    if effects.len() < 2 {
        return Err(SupportedApError::TooFewEffects {
            value: effects.len(),
        });
    }
    support_k(
        effects,
        SupportOrder::new(2).expect("two is a valid support order"),
    )
}

/// The level retained by two replications with frequency `gamma`.
///
/// This is the sample analogue of `m_gamma = Q(1 - sqrt(gamma))`, the lower
/// quantile of the support distribution. `gamma` must lie in `(0, 1]`.
/// Larger `gamma` demands that the level survive more often and therefore
/// returns a smaller value.
///
/// # Errors
///
/// Returns an error for an empty or non-finite sample or unless `gamma` is
/// finite and in `(0, 1]`.
pub fn survival_level(effects: &[f64], gamma: f64) -> Result<f64, SupportedApError> {
    survival_level_k(
        effects,
        gamma,
        SupportOrder::new(2).expect("two is a valid support order"),
    )
}

/// The level retained by `K` replications with frequency `gamma`.
///
/// This is the sample analogue of
/// `m_gamma^(K) = Q(1 - gamma^(1 / K))`. `gamma` must lie in `(0, 1]`.
/// Larger `gamma` demands that the level survive more often and therefore
/// returns a smaller value.
///
/// # Errors
///
/// Returns an error for an empty or non-finite sample or unless `gamma` is
/// finite and in `(0, 1]`.
pub fn survival_level_k(
    effects: &[f64],
    gamma: f64,
    order: SupportOrder,
) -> Result<f64, SupportedApError> {
    validate_effects(effects)?;
    if !(gamma.is_finite() && gamma > 0.0 && gamma <= 1.0) {
        return Err(SupportedApError::InvalidSurvivalFrequency { value: gamma });
    }
    let mut sorted = effects.to_vec();
    sorted.sort_unstable_by(f64::total_cmp);
    Ok(survival_level_sorted(&sorted, gamma, order.get()))
}

/// A quantile of a set of effects, by the inverted-CDF rule.
///
/// `probability` must lie in `[0, 1]`. Used to summarize the permutation null,
/// whose mean alone is its least informative summary.
///
/// # Errors
///
/// Returns an error for an empty or non-finite sample or unless `probability`
/// is finite and in `[0, 1]`.
pub fn quantile(effects: &[f64], probability: f64) -> Result<f64, SupportedApError> {
    validate_effects(effects)?;
    if !(probability.is_finite() && (0.0..=1.0).contains(&probability)) {
        return Err(SupportedApError::InvalidProbability { value: probability });
    }
    let mut sorted = effects.to_vec();
    sorted.sort_unstable_by(f64::total_cmp);
    let sample_size = sorted.len();
    let rank = (sample_size as f64 * probability).ceil();
    let index = if rank <= 1.0 {
        0
    } else if rank >= sample_size as f64 {
        sample_size - 1
    } else {
        rank as usize - 1
    };
    Ok(sorted[index])
}

/// The Monte Carlo standard error of the degree-two support estimator.
///
/// `S_2` is a U-statistic with kernel `min`, so its Monte Carlo error follows
/// from the Hoeffding projection `h1(t) = E[min(t, T)]`. On a sorted array the
/// projection is one prefix sum, which makes the standard error free relative
/// to the estimate itself. This quantity describes numerical error at fixed
/// `B` and says nothing about the statistical information in the evaluation.
///
/// # Errors
///
/// Returns an error unless at least two finite effects are supplied.
pub fn monte_carlo_standard_error(effects: &[f64]) -> Result<f64, SupportedApError> {
    monte_carlo_standard_error_k(
        effects,
        SupportOrder::new(2).expect("two is a valid support order"),
    )
}

/// The first-order Monte Carlo standard error of the `S_K` estimator.
///
/// `S_K` is a U-statistic of degree `K`. The estimate uses the leave-one-out
/// Hoeffding projection and costs `O(B log B)` including the sort. It describes
/// numerical error at fixed `B`; it does not estimate sampling uncertainty in
/// the observed evaluation.
///
/// # Errors
///
/// Returns an error for an empty or non-finite sample or when `order` exceeds
/// the sample size.
pub fn monte_carlo_standard_error_k(
    effects: &[f64],
    order: SupportOrder,
) -> Result<f64, SupportedApError> {
    validate_effects(effects)?;
    let order = order.get();
    if order > effects.len() {
        return Err(SupportedApError::SupportOrderExceedsSample {
            order,
            sample_size: effects.len(),
        });
    }
    let mut sorted = effects.to_vec();
    sorted.sort_unstable_by(f64::total_cmp);
    Ok(monte_carlo_standard_error_sorted(&sorted, order))
}

fn support_sorted(sorted: &[f64], order: usize) -> f64 {
    if sorted.first() == sorted.last() {
        return sorted[0];
    }

    let sample_size = sorted.len();
    if order == 1 {
        return sorted.iter().sum::<f64>() / sample_size as f64;
    }
    if order == 2 {
        return support_two_sorted(sorted);
    }

    // C(B - i - 1, K - 1) / C(B, K), initialized at K / B and
    // advanced by a stable recurrence that avoids integer overflow.
    let last_index = sample_size - order;
    let mut weight = order as f64 / sample_size as f64;
    let mut estimate = 0.0;

    for (index, &effect) in sorted.iter().take(last_index + 1).enumerate() {
        estimate += weight * effect;
        if index < last_index {
            weight *= (sample_size - index - order) as f64 / (sample_size - index - 1) as f64;
        }
    }
    estimate
}

/// The `K = 2` weights have the closed form `2 (B - i) / [B (B - 1)]`, so the
/// sum is accumulated unweighted and scaled once. This avoids the `B` chained
/// multiplications the general recurrence would perform on the hot path.
fn support_two_sorted(sorted: &[f64]) -> f64 {
    let sample_size = sorted.len();
    debug_assert!(sample_size >= 2);
    let mut accumulator = 0.0;
    for (index, &effect) in sorted.iter().enumerate().take(sample_size - 1) {
        accumulator += (sample_size - index - 1) as f64 * effect;
    }
    2.0 * accumulator / (sample_size as f64 * (sample_size - 1) as f64)
}

fn survival_level_sorted(sorted: &[f64], gamma: f64, order: usize) -> f64 {
    let sample_size = sorted.len();
    let probability = 1.0 - gamma.powf(1.0 / order as f64);
    let rank = (sample_size as f64 * probability).ceil();
    let index = if rank <= 1.0 {
        0
    } else if rank >= sample_size as f64 {
        sample_size - 1
    } else {
        rank as usize - 1
    };
    sorted[index]
}

fn monte_carlo_standard_error_sorted(sorted: &[f64], order: usize) -> f64 {
    let sample_size = sorted.len();
    debug_assert!(sample_size >= order);
    debug_assert!(order >= 1);

    if sample_size == 1 {
        return 0.0;
    }

    if order == 1 {
        let mean = sorted.iter().sum::<f64>() / sample_size as f64;
        let variance = sorted
            .iter()
            .map(|value| (value - mean) * (value - mean))
            .sum::<f64>()
            / (sample_size - 1) as f64;
        return (variance / sample_size as f64).sqrt();
    }

    // For sorted t_(i), the leave-one-out projection is
    //
    //   h_{1,-i}(t_i)
    //     = E[min(t_i, T_2, ..., T_K) | t_i],
    //
    // where the remaining K - 1 values are sampled without replacement from
    // the other B - 1 effects. The two recurrences below are normalized
    // binomial coefficients, so no integer combinations are formed.
    let mut weighted_prefix = 0.0;
    let mut lower_weight = (order - 1) as f64 / (sample_size - 1) as f64;
    let mut self_weight = 1.0;
    let mut projection = Vec::with_capacity(sample_size);
    for (index, &effect) in sorted.iter().enumerate() {
        projection.push(weighted_prefix + self_weight * effect);
        weighted_prefix += lower_weight * effect;

        if index + 1 < sample_size {
            let numerator = sample_size.saturating_sub(index + order) as f64;
            let self_denominator = (sample_size - index - 1) as f64;
            self_weight = if numerator == 0.0 {
                0.0
            } else {
                self_weight * numerator / self_denominator
            };

            if index + 2 < sample_size {
                let lower_denominator = (sample_size - index - 2) as f64;
                lower_weight = if numerator == 0.0 {
                    0.0
                } else {
                    lower_weight * numerator / lower_denominator
                };
            }
        }
    }

    let mean = projection.iter().sum::<f64>() / sample_size as f64;
    let variance = projection
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum::<f64>()
        / (sample_size - 1) as f64;

    order as f64 * (variance / sample_size as f64).sqrt()
}

pub(crate) fn supported_cnap_from_cnaps(cnaps: &[Cnap], order: SupportOrder) -> SupportedCnap {
    debug_assert!(cnaps.len() >= 2);
    let values: Vec<f64> = cnaps.iter().map(|value| value.value()).collect();
    SupportedCnap::from_value(support_k(&values, order).expect("bootstrap CNAP values are valid"))
}

pub(crate) fn support_validated(effects: &[f64], order: SupportOrder) -> f64 {
    debug_assert!(effects.len() >= 2);
    support_k(effects, order).expect("internally generated effects are finite")
}

pub(crate) fn support_in_place(effects: &mut [f64], order: SupportOrder) -> f64 {
    debug_assert!(effects.len() >= order.get());
    debug_assert!(effects.iter().all(|effect| effect.is_finite()));
    effects.sort_unstable_by(f64::total_cmp);
    support_sorted(effects, order.get())
}

pub(crate) fn standard_error_from_cnaps(cnaps: &[Cnap], order: SupportOrder) -> f64 {
    let values: Vec<f64> = cnaps.iter().map(|value| value.value()).collect();
    standard_error_from_values(&values, order)
}

pub(crate) fn standard_error_from_values(values: &[f64], order: SupportOrder) -> f64 {
    monte_carlo_standard_error_k(values, order).expect("internally generated effects are valid")
}

pub(crate) fn survival_from_cnaps(
    cnaps: &[Cnap],
    gamma: f64,
    order: SupportOrder,
) -> Result<f64, SupportedApError> {
    let values: Vec<f64> = cnaps.iter().map(|value| value.value()).collect();
    survival_level_k(&values, gamma, order)
}

fn validate_effects(effects: &[f64]) -> Result<(), SupportedApError> {
    if effects.is_empty() {
        return Err(SupportedApError::TooFewEffects { value: 0 });
    }
    for (index, &value) in effects.iter().enumerate() {
        if !value.is_finite() {
            return Err(SupportedApError::InvalidEffect { index, value });
        }
    }
    Ok(())
}
