use supported_ap::{
    BootstrapOptions, ClassCounts, ConditionalNullCalibrationOptions, Evaluation,
    NullDistributionStorage, PairedEvaluation, Parallelism, PermutationCount, PermutationOptions,
    ReferencePrevalence, ReplicateCount, SupportedApError,
};

fn bootstrap_options(replicates: usize, seed: u64) -> BootstrapOptions {
    BootstrapOptions {
        replicates: ReplicateCount::new(replicates).unwrap(),
        seed,
        parallelism: Parallelism::Sequential,
        ..BootstrapOptions::default()
    }
}

#[test]
fn perfect_separation_is_an_exact_bootstrap_fixed_point() {
    let evaluation = Evaluation::new(&[0.9, 0.8, 0.2, 0.1], &[true, true, false, false]).unwrap();
    let prevalence = ReferencePrevalence::new(0.1).unwrap();
    let result = evaluation
        .supported_cnap(prevalence, bootstrap_options(128, 10))
        .unwrap();

    assert_eq!(result.observed().cnap().value(), 1.0);
    assert_eq!(result.supported().value(), 1.0);
    assert_eq!(result.replicate_mean().value(), 1.0);
    assert_eq!(result.support_penalty(), 0.0);
    assert!(
        result
            .replicate_cnaps()
            .iter()
            .all(|effect| effect.value() == 1.0)
    );
}

#[test]
fn bootstrap_is_reproducible_for_a_fixed_seed() {
    let evaluation = Evaluation::new(
        &[0.9, 0.7, 0.7, 0.5, 0.4, 0.1],
        &[true, false, true, false, true, false],
    )
    .unwrap();
    let prevalence = ReferencePrevalence::new(0.2).unwrap();
    let options = bootstrap_options(200, 83);

    let first = evaluation.supported_cnap(prevalence, options).unwrap();
    let second = evaluation.supported_cnap(prevalence, options).unwrap();

    assert_eq!(first, second);
}

#[test]
fn constant_scores_calibrate_to_zero_against_the_fixed_endpoint() {
    let evaluation = Evaluation::new(&[1.0; 6], &[true, false, true, false, true, false]).unwrap();
    let prevalence = ReferencePrevalence::new(0.1).unwrap();
    let options = ConditionalNullCalibrationOptions {
        bootstrap: bootstrap_options(40, 3),
        permutation: PermutationOptions {
            permutations: PermutationCount::new(20).unwrap(),
            seed: 5,
            parallelism: Parallelism::Sequential,
            storage: NullDistributionStorage::Full,
        },
    };

    let result = evaluation
        .conditionally_calibrated_supported_cnap(prevalence, options)
        .unwrap();
    assert_eq!(result.estimate().supported().value(), 0.0);
    assert_eq!(result.conditional_null().value(), 0.0);
    assert_eq!(result.calibrated().value(), 0.0);
    assert_eq!(result.p_value().value(), 1.0);
    assert!(!result.is_above_conditional_null());
}

#[test]
fn score_ties_reduce_performance_without_changing_the_upper_endpoint() {
    let tied = Evaluation::new(&[0.9, 0.5, 0.5, 0.1], &[true, true, false, false]).unwrap();
    let prevalence = ReferencePrevalence::new(0.1).unwrap();
    let result = tied
        .conditionally_calibrated_supported_cnap(
            prevalence,
            ConditionalNullCalibrationOptions {
                bootstrap: bootstrap_options(256, 2),
                permutation: PermutationOptions {
                    permutations: PermutationCount::new(40).unwrap(),
                    seed: 3,
                    parallelism: Parallelism::Sequential,
                    storage: NullDistributionStorage::SummaryOnly,
                },
            },
        )
        .unwrap();
    assert!(result.estimate().supported().value() < 1.0);
    assert!(result.calibrated().value() < 1.0);
}

#[test]
fn a_below_anchor_estimate_is_flagged_as_an_extrapolation() {
    // Scores ordered against the outcomes: worse than chance by construction.
    let evaluation = Evaluation::new(
        &[0.1, 0.2, 0.3, 0.9, 0.8, 0.7],
        &[true, true, true, false, false, false],
    )
    .unwrap();
    let prevalence = ReferencePrevalence::new(0.2).unwrap();
    let options = ConditionalNullCalibrationOptions {
        bootstrap: bootstrap_options(200, 21),
        permutation: PermutationOptions {
            permutations: PermutationCount::new(60).unwrap(),
            seed: 23,
            parallelism: Parallelism::Sequential,
            storage: NullDistributionStorage::Full,
        },
    };
    let result = evaluation
        .conditionally_calibrated_supported_cnap(prevalence, options)
        .unwrap();

    assert!(result.estimate().observed().cnap().value() < 0.0);
    assert!(!result.is_above_conditional_null());
    assert!(result.is_extrapolation());
    assert!(result.p_value().value() > 0.5);
    assert_eq!(result.conditional_null_distribution().unwrap().len(), 60);
    assert!(result.conditional_null_quantile(0.5).unwrap().is_finite());
}

#[test]
fn perfect_separation_retains_the_fixed_upper_endpoint() {
    let evaluation = Evaluation::new(&[0.9, 0.8, 0.2, 0.1], &[true, true, false, false]).unwrap();
    let prevalence = ReferencePrevalence::new(0.1).unwrap();
    let options = ConditionalNullCalibrationOptions {
        bootstrap: bootstrap_options(80, 13),
        permutation: PermutationOptions {
            permutations: PermutationCount::new(40).unwrap(),
            seed: 17,
            parallelism: Parallelism::Sequential,
            storage: NullDistributionStorage::SummaryOnly,
        },
    };
    let result = evaluation
        .conditionally_calibrated_supported_cnap(prevalence, options)
        .unwrap();

    assert!((result.calibrated().value() - 1.0).abs() < 1e-12);
    assert!(result.is_above_conditional_null());
    assert!(result.conditional_null_distribution().is_none());
    assert_eq!(
        result.conditional_null_quantile(0.5).unwrap_err(),
        SupportedApError::ConditionalNullDistributionNotStored
    );
    assert_eq!(
        result.conditional_null_quantile(0.5).unwrap_err(),
        SupportedApError::ConditionalNullDistributionNotStored
    );
    assert!((0.0..=1.0).contains(&result.p_value().value()));
}

#[test]
fn paired_identical_rankings_have_exact_zero_support() {
    let scores = [0.9, 0.7, 0.4, 0.1];
    let labels = [true, false, true, false];
    let paired = PairedEvaluation::new(&scores, &scores, &labels).unwrap();
    let prevalence = ReferencePrevalence::new(0.25).unwrap();
    let result = paired
        .estimated_supported_superiority(prevalence, bootstrap_options(128, 29))
        .unwrap();

    assert_eq!(result.observed_difference().value(), 0.0);
    assert_eq!(result.estimated_supported_superiority().value(), 0.0);
    assert_eq!(result.replicate_mean_difference().value(), 0.0);
    assert_eq!(result.support_penalty(), 0.0);
    assert!(
        result
            .replicate_differences()
            .iter()
            .all(|difference| difference.value() == 0.0)
    );
}

#[test]
fn paired_estimation_preserves_shared_resamples() {
    let model_a = [0.9, 0.8, 0.2, 0.1, 0.7, 0.3];
    let model_b = [0.9, 0.3, 0.8, 0.1, 0.2, 0.7];
    let labels = [true, true, false, false, true, false];
    let paired = PairedEvaluation::new(&model_a, &model_b, &labels).unwrap();
    let prevalence = ReferencePrevalence::new(0.2).unwrap();
    let options = bootstrap_options(256, 31);

    let first = paired
        .estimated_supported_superiority(prevalence, options)
        .unwrap();
    let second = paired
        .estimated_supported_superiority(prevalence, options)
        .unwrap();

    assert_eq!(first, second);
    assert!(first.observed_difference().value() > 0.0);
}

#[test]
fn paired_prospective_replication_counts_are_distinct_from_observed_counts() {
    let model_a = [0.95, 0.80, 0.55, 0.30, 0.10, 0.70, 0.40, 0.20];
    let model_b = [0.80, 0.95, 0.30, 0.55, 0.10, 0.40, 0.70, 0.20];
    let labels = [true, true, false, false, false, true, false, false];
    let paired = PairedEvaluation::new(&model_a, &model_b, &labels).unwrap();
    let prevalence = ReferencePrevalence::new(0.2).unwrap();
    let options = bootstrap_options(256, 31);

    let same_design = paired
        .estimated_supported_superiority(prevalence, options)
        .unwrap();
    let prospective_counts = ClassCounts::new(12, 40).unwrap();
    let prospective = paired
        .estimated_supported_superiority_with_replication_counts(
            prevalence,
            prospective_counts,
            options,
        )
        .unwrap();

    assert_eq!(
        same_design.observed_class_counts(),
        paired.model_a().class_counts()
    );
    assert_eq!(
        same_design.replication_class_counts(),
        paired.model_a().class_counts()
    );
    assert_eq!(
        prospective.observed_class_counts(),
        paired.model_a().class_counts()
    );
    assert_eq!(prospective.replication_class_counts(), prospective_counts);
    assert_eq!(
        prospective.observed_difference(),
        same_design.observed_difference()
    );
    assert_ne!(
        prospective.replicate_differences(),
        same_design.replicate_differences()
    );
}

#[test]
fn paired_resamples_reference_the_same_units_for_both_models() {
    // Model B is model A with the two top-ranked units swapped in score but
    // the same units underneath. If the paired bootstrap ever drew different
    // units for the two models, the shared difficulty would stop cancelling
    // and the replicate differences would spread far wider than they do.
    let model_a = [0.95, 0.90, 0.30, 0.20, 0.85, 0.25];
    let model_b = [0.90, 0.95, 0.20, 0.30, 0.25, 0.85];
    let labels = [true, true, false, false, true, false];
    let paired = PairedEvaluation::new(&model_a, &model_b, &labels).unwrap();
    let prevalence = ReferencePrevalence::new(0.2).unwrap();
    let result = paired
        .estimated_supported_superiority(prevalence, bootstrap_options(512, 77))
        .unwrap();

    // Every replicate difference must be reachable by re-weighting the same
    // six units, so it is bounded by the largest achievable CNAP gap.
    for difference in result.replicate_differences() {
        assert!(
            difference.value().abs() <= 1.0 + 1e-12,
            "replicate difference {difference} left the achievable range"
        );
    }

    // Independent resampling would break the exact zero of the identical case.
    let identical = PairedEvaluation::new(&model_a, &model_a, &labels).unwrap();
    let zero = identical
        .estimated_supported_superiority(prevalence, bootstrap_options(512, 77))
        .unwrap();
    assert!(
        zero.replicate_differences()
            .iter()
            .all(|difference| difference.value() == 0.0),
        "shared units must cancel exactly when the models agree"
    );
    assert_eq!(zero.monte_carlo_standard_error(), 0.0);
}

#[test]
fn support_order_three_is_stricter_than_two() {
    let evaluation = Evaluation::new(
        &[0.91, 0.74, 0.68, 0.52, 0.31, 0.08, 0.44, 0.60],
        &[true, false, true, true, false, false, false, true],
    )
    .unwrap();
    let prevalence = ReferencePrevalence::new(0.3).unwrap();

    let with_order = |order: usize| {
        evaluation
            .supported_cnap(
                prevalence,
                BootstrapOptions {
                    support_order: supported_ap::SupportOrder::new(order).unwrap(),
                    ..bootstrap_options(400, 91)
                },
            )
            .unwrap()
            .supported()
            .value()
    };

    let two = with_order(2);
    let three = with_order(3);
    assert!(
        three <= two + 1e-12,
        "a claim asked to survive more replications is corroborated no higher: {two} then {three}"
    );
}
