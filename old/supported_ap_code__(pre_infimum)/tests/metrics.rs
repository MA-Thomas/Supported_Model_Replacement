use supported_ap::{
    BinaryLabel, Evaluation, ReferencePrevalence, SupportOrder, SupportedApError, cnap,
    prior_standardized_average_precision, support_k, support_two,
};

fn assert_close(actual: f64, expected: f64) {
    let tolerance = 1e-12_f64.max(expected.abs() * 1e-12);
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected:.16}, received {actual:.16}"
    );
}

#[test]
fn tied_scores_enter_as_one_threshold_block() {
    let scores = [0.90, 0.70, 0.70, 0.20];
    let labels = [true, true, false, false];
    let prevalence = ReferencePrevalence::new(0.5).unwrap();
    let evaluation = Evaluation::new(&scores, &labels).unwrap();

    assert_eq!(evaluation.threshold_count(), 3);
    assert_close(
        evaluation
            .prior_standardized_average_precision(prevalence)
            .value(),
        5.0 / 6.0,
    );
    assert_close(evaluation.cnap(prevalence).value(), 2.0 / 3.0);
}

#[test]
fn constant_scores_are_exactly_at_analytic_chance() {
    let scores = [1.0; 6];
    let labels = [true, false, true, false, false, true];

    for value in [0.01, 0.10, 0.50, 0.90] {
        let prevalence = ReferencePrevalence::new(value).unwrap();
        assert_close(
            prior_standardized_average_precision(&scores, &labels, prevalence)
                .unwrap()
                .value(),
            value,
        );
        assert_close(cnap(&scores, &labels, prevalence).unwrap().value(), 0.0);
    }
}

#[test]
fn perfect_ranking_has_exact_unit_cnap() {
    let scores = [0.9, 0.8, 0.3, 0.1];
    let labels = [true, true, false, false];

    for value in [0.001, 0.10, 0.50, 0.999] {
        let prevalence = ReferencePrevalence::new(value).unwrap();
        assert_close(cnap(&scores, &labels, prevalence).unwrap().value(), 1.0);
    }
}

#[test]
fn below_chance_cnap_is_not_clipped() {
    let scores = [0.4, 0.3, 0.9, 0.8];
    let labels = [true, true, false, false];
    let prevalence = ReferencePrevalence::new(0.5).unwrap();
    let effect = cnap(&scores, &labels, prevalence).unwrap().value();

    assert!(effect < 0.0);
    assert_close(effect, -1.0 / 6.0);
}

#[test]
fn bool_binary_label_and_u8_inputs_agree() {
    let scores = [0.8, 0.6, 0.2];
    let bools = [true, false, true];
    let labels = [
        BinaryLabel::Positive,
        BinaryLabel::Negative,
        BinaryLabel::Positive,
    ];
    let bytes = [1, 0, 1];
    let prevalence = ReferencePrevalence::new(0.2).unwrap();

    let from_bool = Evaluation::new(&scores, &bools).unwrap();
    let from_label = Evaluation::new(&scores, &labels).unwrap();
    let from_u8 = Evaluation::from_u8(&scores, &bytes).unwrap();

    assert_eq!(from_bool.cnap(prevalence), from_label.cnap(prevalence));
    assert_eq!(from_bool.cnap(prevalence), from_u8.cnap(prevalence));
}

#[test]
fn invalid_inputs_are_rejected_at_construction() {
    assert!(matches!(
        ReferencePrevalence::new(0.0),
        Err(SupportedApError::InvalidReferencePrevalence { .. })
    ));
    assert!(matches!(
        Evaluation::new(&[0.1, f64::NAN], &[true, false]),
        Err(SupportedApError::InvalidScore { index: 1, .. })
    ));
    assert!(matches!(
        Evaluation::new(&[0.1, 0.2], &[true, true]),
        Err(SupportedApError::MissingClass {
            class: BinaryLabel::Negative
        })
    ));
    assert!(matches!(
        Evaluation::from_u8(&[0.1, 0.2], &[0, 2]),
        Err(SupportedApError::InvalidBinaryLabel { value: 2 })
    ));
}

#[test]
fn general_support_uses_all_distinct_subsets() {
    let effects = [1.0, 2.0, 3.0];

    assert_close(
        support_k(&effects, SupportOrder::new(1).unwrap()).unwrap(),
        2.0,
    );
    assert_close(support_two(&effects).unwrap(), 4.0 / 3.0);
    assert_close(
        support_k(&effects, SupportOrder::new(3).unwrap()).unwrap(),
        1.0,
    );
}

#[test]
fn support_rejects_invalid_effects_and_orders() {
    assert!(matches!(
        support_two(&[1.0]),
        Err(SupportedApError::TooFewEffects { value: 1 })
    ));
    assert!(matches!(
        support_two(&[1.0, f64::INFINITY]),
        Err(SupportedApError::InvalidEffect { index: 1, .. })
    ));
    assert!(matches!(
        support_k(&[1.0, 2.0], SupportOrder::new(3).unwrap()),
        Err(SupportedApError::SupportOrderExceedsSample { .. })
    ));
}

#[test]
fn support_recurrence_matches_brute_force_minima() {
    let effects = [-0.4, 0.1, 0.3, 0.8, 1.0];
    for order in 1..=effects.len() {
        let mut minima = Vec::new();
        collect_subset_minima(&effects, order, 0, &mut Vec::new(), &mut minima);
        let brute_force = minima.iter().sum::<f64>() / minima.len() as f64;
        let optimized = support_k(&effects, SupportOrder::new(order).unwrap()).unwrap();
        assert_close(optimized, brute_force);
    }
}

#[test]
fn tie_block_ap_matches_an_independent_threshold_reference() {
    for sample_size in 2..=30 {
        let scores: Vec<f64> = (0..sample_size)
            .map(|index| ((index * 11 + sample_size * 3) % 7) as f64)
            .collect();
        let labels: Vec<bool> = (0..sample_size)
            .map(|index| index == 0 || (index % 4 == 0 && index + 1 < sample_size))
            .collect();
        let evaluation = Evaluation::new(&scores, &labels).unwrap();

        for prevalence in [0.01, 0.17, 0.50, 0.93] {
            let prevalence = ReferencePrevalence::new(prevalence).unwrap();
            let expected = threshold_prior_standardized_ap(&scores, &labels, prevalence.value());
            assert_close(
                evaluation
                    .prior_standardized_average_precision(prevalence)
                    .value(),
                expected,
            );
        }
    }
}

fn threshold_prior_standardized_ap(scores: &[f64], labels: &[bool], prevalence: f64) -> f64 {
    let mut thresholds = scores.to_vec();
    thresholds.sort_unstable_by(|left, right| right.total_cmp(left));
    thresholds.dedup();

    let positive_total = labels.iter().filter(|&&label| label).count() as f64;
    let negative_total = labels.len() as f64 - positive_total;
    let mut previous_recall = 0.0;
    let mut average_precision = 0.0;

    for threshold in thresholds {
        let mut true_positives = 0usize;
        let mut false_positives = 0usize;
        for (&score, &label) in scores.iter().zip(labels) {
            if score >= threshold {
                if label {
                    true_positives += 1;
                } else {
                    false_positives += 1;
                }
            }
        }

        let recall = true_positives as f64 / positive_total;
        let false_positive_rate = false_positives as f64 / negative_total;
        let numerator = prevalence * recall;
        let precision = numerator / (numerator + (1.0 - prevalence) * false_positive_rate);
        average_precision += (recall - previous_recall) * precision;
        previous_recall = recall;
    }
    average_precision
}

fn collect_subset_minima(
    values: &[f64],
    order: usize,
    start: usize,
    selected: &mut Vec<f64>,
    minima: &mut Vec<f64>,
) {
    if selected.len() == order {
        minima.push(
            selected
                .iter()
                .copied()
                .reduce(f64::min)
                .expect("subsets are nonempty"),
        );
        return;
    }

    let remaining = order - selected.len();
    for index in start..=values.len() - remaining {
        selected.push(values[index]);
        collect_subset_minima(values, order, index + 1, selected, minima);
        selected.pop();
    }
}
