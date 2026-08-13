//! Properties of the support functional that the estimator relies on, and the
//! analytic chance level the manuscript derives in closed form.
//!
//! Subset enumeration and the independent threshold reference for average
//! precision live in `metrics.rs`.

use supported_ap::{
    BootstrapOptions, Evaluation, Parallelism, ReferencePrevalence, ReplicateCount, SupportOrder,
    SupportedApError, monte_carlo_standard_error, monte_carlo_standard_error_k, quantile,
    support_k, support_two, survival_level, survival_level_k,
};

fn assert_close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected:.12}, received {actual:.12}"
    );
}

/// `S_2` evaluated directly over all ordered pairs, independent of the
/// order-statistic weights the crate uses.
fn pairwise_support_two(effects: &[f64]) -> f64 {
    let mut total = 0.0;
    for &first in effects {
        for &second in effects {
            total += first.min(second);
        }
    }
    let n = effects.len() as f64;
    // Remove the n self-pairs, which contribute the values themselves.
    let self_pairs: f64 = effects.iter().sum();
    (total - self_pairs) / (n * (n - 1.0))
}

#[test]
fn degree_two_closed_form_matches_direct_pairwise_computation() {
    let effects: Vec<f64> = (0..400)
        .map(|index| ((index * 37 % 101) as f64 / 101.0) - 0.5)
        .collect();
    assert_close(
        support_two(&effects).unwrap(),
        pairwise_support_two(&effects),
        1e-12,
    );
    assert_close(
        support_k(&effects, SupportOrder::new(2).unwrap()).unwrap(),
        pairwise_support_two(&effects),
        1e-12,
    );
}

#[test]
fn support_is_decreasing_in_the_replication_count() {
    let effects = [0.1, 0.4, 0.2, 0.9, 0.55, 0.33, 0.7];
    let mut previous = f64::INFINITY;
    for order in 1..=effects.len() {
        let value = support_k(&effects, SupportOrder::new(order).unwrap()).unwrap();
        assert!(
            value <= previous + 1e-12,
            "S_K must not increase with K, at K={order}"
        );
        previous = value;
    }
    // The strictest criterion is the sample minimum.
    assert_close(previous, 0.1, 1e-12);
}

#[test]
fn survival_level_spans_the_sorted_effects() {
    let effects: Vec<f64> = (0..1000).map(|index| index as f64 / 999.0).collect();
    // gamma = 1 asks the level to survive every replication.
    assert_close(survival_level(&effects, 1.0).unwrap(), 0.0, 1e-12);
    // Q(1 - sqrt(gamma)) at gamma = 0.25 is the median.
    assert_close(survival_level(&effects, 0.25).unwrap(), 0.5, 2e-3);
    // A stricter frequency demands a lower level.
    assert!(survival_level(&effects, 0.5).unwrap() < survival_level(&effects, 0.25).unwrap());
    assert!(survival_level(&effects, 2.0).is_err());
    assert!(survival_level(&effects, 0.0).is_err());

    // For K = 3 and gamma = 1/8, gamma^(1/K) = 1/2, so the same median is used.
    assert_close(
        survival_level_k(&effects, 0.125, SupportOrder::new(3).unwrap()).unwrap(),
        0.5,
        2e-3,
    );
}

#[test]
fn quantile_matches_the_inverted_cdf() {
    let effects: Vec<f64> = (1..=100).map(|index| index as f64).collect();
    assert_close(quantile(&effects, 0.0).unwrap(), 1.0, 1e-12);
    assert_close(quantile(&effects, 0.5).unwrap(), 50.0, 1e-12);
    assert_close(quantile(&effects, 1.0).unwrap(), 100.0, 1e-12);
}

#[test]
fn quantile_reports_probability_errors_with_the_correct_variant() {
    assert!(matches!(
        quantile(&[1.0, 2.0], -0.1),
        Err(SupportedApError::InvalidProbability { value }) if value == -0.1
    ));
}

#[test]
fn degenerate_effects_have_zero_monte_carlo_error() {
    let effects = [0.42; 64];
    assert_close(support_two(&effects).unwrap(), 0.42, 1e-12);
    assert_close(monte_carlo_standard_error(&effects).unwrap(), 0.0, 1e-12);
}

#[test]
fn monte_carlo_error_uses_the_requested_support_order() {
    let effects = [-0.4, 0.1, 0.3, 0.8, 1.0, 1.4];
    for order in 1..=4 {
        let expected = brute_force_projection_standard_error(&effects, order);
        let actual =
            monte_carlo_standard_error_k(&effects, SupportOrder::new(order).unwrap()).unwrap();
        assert_close(actual, expected, 1e-12);
    }
    assert_close(
        monte_carlo_standard_error(&effects).unwrap(),
        monte_carlo_standard_error_k(&effects, SupportOrder::new(2).unwrap()).unwrap(),
        1e-12,
    );
}

#[test]
fn monte_carlo_error_shrinks_with_the_replicate_count() {
    let scores: Vec<f64> = (0..10).map(|index| 10.0 - index as f64).collect();
    let mut labels = [false; 10];
    labels[3] = true;
    let evaluation = Evaluation::new(&scores, &labels).unwrap();
    let prevalence = ReferencePrevalence::new(0.1).unwrap();

    let error_for = |replicates: usize| {
        evaluation
            .supported_cnap(
                prevalence,
                BootstrapOptions {
                    replicates: ReplicateCount::new(replicates).unwrap(),
                    seed: 5,
                    parallelism: Parallelism::Sequential,
                    ..BootstrapOptions::default()
                },
            )
            .unwrap()
            .monte_carlo_standard_error()
    };

    let small = error_for(500);
    let large = error_for(8_000);
    assert!(small > 0.0 && large > 0.0);
    // Sixteen times the replicates should roughly quarter the standard error.
    assert!(
        large < small * 0.6,
        "expected the standard error to fall, {small} -> {large}"
    );
}

/// `S_2` for a discrete distribution, over all ordered pairs of independent
/// draws. Two independent draws can coincide, so the self-pairs belong in the
/// average. The U-statistic of `pairwise_support_two` excludes them, which is
/// correct for `B` sampled replicates and wrong for an enumerated support.
fn expectation_of_minimum(support: &[f64]) -> f64 {
    let mut total = 0.0;
    for &first in support {
        for &second in support {
            total += first.min(second);
        }
    }
    total / (support.len() * support.len()) as f64
}

fn brute_force_projection_standard_error(effects: &[f64], order: usize) -> f64 {
    let mut projection = Vec::with_capacity(effects.len());
    for fixed in 0..effects.len() {
        if order == 1 {
            projection.push(effects[fixed]);
            continue;
        }
        let others: Vec<f64> = effects
            .iter()
            .enumerate()
            .filter_map(|(index, &value)| (index != fixed).then_some(value))
            .collect();
        let mut minima = Vec::new();
        collect_conditional_minima(&others, order - 1, 0, effects[fixed], &mut minima);
        projection.push(minima.iter().sum::<f64>() / minima.len() as f64);
    }
    let mean = projection.iter().sum::<f64>() / projection.len() as f64;
    let variance = projection
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum::<f64>()
        / (projection.len() - 1) as f64;
    order as f64 * (variance / projection.len() as f64).sqrt()
}

fn collect_conditional_minima(
    values: &[f64],
    remaining: usize,
    start: usize,
    current_minimum: f64,
    out: &mut Vec<f64>,
) {
    if remaining == 0 {
        out.push(current_minimum);
        return;
    }
    for index in start..=values.len() - remaining {
        collect_conditional_minima(
            values,
            remaining - 1,
            index + 1,
            current_minimum.min(values[index]),
            out,
        );
    }
}

/// With `n = 10`, one positive, and a reference prevalence of `0.1`, random
/// ranking places the positive at rank `j` with probability `1/10` and gives
/// `AP = 1/j`. The manuscript derives `E[S_2^{AP} | chance] = (10 - H_10)/90`,
/// the mean `(H_10 - 1)/9`, and the disagreement penalty between them.
#[test]
fn analytic_chance_level_at_ten_units_with_one_positive() {
    let harmonic: f64 = (1..=10).map(|j| 1.0 / j as f64).sum();
    let expected = (10.0 - harmonic) / 90.0;

    // The exact replication distribution: ten equally likely ranks.
    let support: Vec<f64> = (1..=10)
        .map(|rank| ((1.0 / rank as f64) - 0.1) / 0.9)
        .collect();

    assert_close(expectation_of_minimum(&support), expected, 1e-12);
    assert_close(expected, 0.078_567_019_400_352_74, 1e-12);

    // The mean of the same distribution is the upward AP bias, and the gap
    // between the two is the disagreement penalty.
    let mean = support.iter().sum::<f64>() / 10.0;
    assert_close(mean, (harmonic - 1.0) / 9.0, 1e-12);
    assert_close(mean - expected, 0.135_762_786_596_119_9, 1e-12);
}
