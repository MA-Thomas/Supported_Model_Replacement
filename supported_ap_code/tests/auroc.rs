use supported_ap::{
    AuRocBreakdown, AuRocOptimizationOptions, AuRocPolicy, ComputationalReplicationCount,
    ConcentrationFactor, ConcentrationSearchOptions, Execution, MagnitudeThreshold,
    NestedAuRocAssessment, NestedRowResampling, PairedEvaluation, ProjectedAuRocAssessment,
    ProjectedResampling, ReferenceAssessment, ResamplingUnit, SupportOrder, SurvivalFloor,
    SurvivalRequirement, estimate_nested_auroc_breakdown, estimate_projected_auroc_breakdown,
    paired_auroc_difference, worst_case_auroc_difference,
};

fn policy(delta: f64, floor: f64, gamma: f64) -> AuRocPolicy {
    AuRocPolicy::new(
        MagnitudeThreshold::new(delta).unwrap(),
        SurvivalFloor::new(floor).unwrap(),
        SurvivalRequirement::new(gamma).unwrap(),
    )
}

fn projected_assessment(
    evaluation: &PairedEvaluation,
    execution: Execution,
) -> ProjectedAuRocAssessment {
    ProjectedAuRocAssessment {
        resampling: ProjectedResampling::new(
            ComputationalReplicationCount::new(10).unwrap(),
            SupportOrder::new(2).unwrap(),
            evaluation.class_counts(),
            20260802,
            execution,
            ResamplingUnit::IndependentObservation,
        )
        .unwrap(),
        optimization: AuRocOptimizationOptions::default(),
        concentration_search: ConcentrationSearchOptions::new(0.05, 16).unwrap(),
        reference_assessment: ReferenceAssessment::not_asserted("unit test").unwrap(),
    }
}

fn nested_assessment(
    evaluations: &[PairedEvaluation],
    execution: Execution,
) -> NestedAuRocAssessment {
    NestedAuRocAssessment {
        empirical_order: SupportOrder::new(2).unwrap(),
        computational_order: SupportOrder::new(2).unwrap(),
        rows: evaluations
            .iter()
            .enumerate()
            .map(|(row, evaluation)| NestedRowResampling {
                replications: ComputationalReplicationCount::new(4 + row).unwrap(),
                replication_counts: evaluation.class_counts(),
                seed: 20260806 + row as u64,
                resampling_unit: ResamplingUnit::IndependentObservation,
            })
            .collect(),
        optimization: AuRocOptimizationOptions::default(),
        concentration_search: ConcentrationSearchOptions::new(0.05, 16).unwrap(),
        execution,
        reference_assessment: ReferenceAssessment::not_asserted("unit test").unwrap(),
    }
}

fn strong_evaluations() -> Vec<PairedEvaluation> {
    vec![
        PairedEvaluation::new(
            &[0.9, 0.8, 0.7, 0.3, 0.2, 0.1],
            &[0.3, 0.2, 0.1, 0.9, 0.8, 0.7],
            &[true, true, true, false, false, false],
        )
        .unwrap(),
        PairedEvaluation::new(
            &[0.95, 0.85, 0.75, 0.25, 0.15, 0.05],
            &[0.25, 0.15, 0.05, 0.95, 0.85, 0.75],
            &[true, true, true, false, false, false],
        )
        .unwrap(),
    ]
}

fn varying_evaluations() -> Vec<PairedEvaluation> {
    vec![
        PairedEvaluation::new(
            &[0.9, 0.7, 0.6, 0.2, 0.1, 0.0],
            &[0.8, 0.6, 0.3, 0.7, 0.2, 0.1],
            &[true, true, true, false, false, false],
        )
        .unwrap(),
        PairedEvaluation::new(
            &[0.95, 0.8, 0.5, 0.4, 0.2, 0.1],
            &[0.7, 0.6, 0.4, 0.8, 0.3, 0.2],
            &[true, true, true, false, false, false],
        )
        .unwrap(),
    ]
}

#[test]
fn gamma_one_is_the_ordinary_paired_auroc_difference() {
    let evaluation = PairedEvaluation::new(
        &[0.9, 0.6, 0.5, 0.4, 0.3, 0.1],
        &[0.8, 0.7, 0.5, 0.6, 0.2, 0.1],
        &[true, true, true, false, false, false],
    )
    .unwrap();
    let ordinary = paired_auroc_difference(&evaluation);
    let certificate = worst_case_auroc_difference(
        &evaluation,
        ConcentrationFactor::new(1.0).unwrap(),
        AuRocOptimizationOptions::default(),
    )
    .unwrap();
    assert_eq!(certificate.global_lower_bound, ordinary);
    assert_eq!(certificate.feasible_upper_bound, ordinary);
    assert_eq!(certificate.absolute_gap, 0.0);
}

#[test]
fn saturation_is_the_least_favorable_observed_pair() {
    let evaluation = PairedEvaluation::new(
        &[0.1, 0.6, 0.5, 0.9, 0.3, 0.1],
        &[0.9, 0.7, 0.5, 0.1, 0.2, 0.1],
        &[true, true, true, false, false, false],
    )
    .unwrap();
    let certificate = worst_case_auroc_difference(
        &evaluation,
        ConcentrationFactor::new(3.0).unwrap(),
        AuRocOptimizationOptions::default(),
    )
    .unwrap();
    assert_eq!(certificate.global_lower_bound, -1.0);
    assert_eq!(certificate.feasible_upper_bound, -1.0);
}

#[test]
fn support_through_saturation_returns_no_empirical_breakdown() {
    let evaluation = PairedEvaluation::new(
        &[0.9, 0.8, 0.7, 0.3, 0.2, 0.1],
        &[0.3, 0.2, 0.1, 0.9, 0.8, 0.7],
        &[true, true, true, false, false, false],
    )
    .unwrap();
    let result = estimate_projected_auroc_breakdown(
        &evaluation,
        &projected_assessment(&evaluation, Execution::Sequential),
        policy(0.5, 0.5, 0.5),
    )
    .unwrap();
    assert_eq!(
        result.breakdown,
        AuRocBreakdown::NoBreakdownOnEmpiricalSupport
    );
    assert_eq!(result.boundary.supported_magnitude, 1.0);
    assert_eq!(result.boundary.literal_survival_fraction, 1.0);
}

#[test]
fn rayon_and_sequential_projected_analyses_are_seed_identical() {
    let evaluation = PairedEvaluation::new(
        &[0.9, 0.8, 0.6, 0.4, 0.2, 0.1],
        &[0.7, 0.5, 0.4, 0.8, 0.3, 0.2],
        &[true, true, true, false, false, false],
    )
    .unwrap();
    let sequential = estimate_projected_auroc_breakdown(
        &evaluation,
        &projected_assessment(&evaluation, Execution::Sequential),
        policy(0.01, 0.0, 0.2),
    )
    .unwrap();
    let parallel = estimate_projected_auroc_breakdown(
        &evaluation,
        &projected_assessment(&evaluation, Execution::Parallel),
        policy(0.01, 0.0, 0.2),
    )
    .unwrap();
    assert_eq!(sequential, parallel);
}

#[test]
fn nested_auroc_preserves_heterogeneous_rows_and_certified_bounds() {
    let evaluations = strong_evaluations();
    let result = estimate_nested_auroc_breakdown(
        &evaluations,
        &nested_assessment(&evaluations, Execution::Sequential),
        policy(0.5, 0.5, 0.5),
    )
    .unwrap();

    assert_eq!(
        result.breakdown,
        AuRocBreakdown::NoBreakdownOnEmpiricalSupport
    );
    let baseline = result.baseline.as_ref().unwrap();
    assert_eq!(baseline.literal_survival.rows.len(), 2);
    assert_eq!(
        baseline.literal_survival.rows[0].computational_effect_count,
        4
    );
    assert_eq!(
        baseline.literal_survival.rows[1].computational_effect_count,
        5
    );
    for point in &result.evaluated_profile {
        assert!(point.supported_magnitude_lower <= point.supported_magnitude + 1e-12);
        assert!(point.supported_magnitude <= point.supported_magnitude_upper + 1e-12);
        assert!(
            point.literal_survival_lower.subset_fraction
                <= point.literal_survival.subset_fraction + 1e-12
        );
        assert!(
            point.literal_survival.subset_fraction
                <= point.literal_survival_upper.subset_fraction + 1e-12
        );
    }
}

#[test]
fn nested_auroc_is_seed_identical_across_execution_modes() {
    let evaluations = strong_evaluations();
    let sequential = estimate_nested_auroc_breakdown(
        &evaluations,
        &nested_assessment(&evaluations, Execution::Sequential),
        policy(0.5, 0.5, 0.5),
    )
    .unwrap();
    let parallel = estimate_nested_auroc_breakdown(
        &evaluations,
        &nested_assessment(&evaluations, Execution::Parallel),
        policy(0.5, 0.5, 0.5),
    )
    .unwrap();
    assert_eq!(sequential, parallel);
}

#[test]
fn nested_auroc_stops_at_a_failed_observed_gate() {
    let evaluations = vec![
        PairedEvaluation::new(
            &[0.9, 0.8, 0.7, 0.3, 0.2, 0.1],
            &[0.9, 0.8, 0.7, 0.3, 0.2, 0.1],
            &[true, true, true, false, false, false],
        )
        .unwrap(),
        PairedEvaluation::new(
            &[0.95, 0.85, 0.75, 0.25, 0.15, 0.05],
            &[0.95, 0.85, 0.75, 0.25, 0.15, 0.05],
            &[true, true, true, false, false, false],
        )
        .unwrap(),
    ];
    let result = estimate_nested_auroc_breakdown(
        &evaluations,
        &nested_assessment(&evaluations, Execution::Sequential),
        policy(0.0, 0.0, 0.5),
    )
    .unwrap();

    assert!(matches!(
        result.breakdown,
        AuRocBreakdown::ObservedGateFailure { .. }
    ));
    assert!(result.baseline.is_none());
    assert!(result.boundary.is_none());
    assert!(result.evaluated_profile.is_empty());
}

#[test]
fn nested_auroc_profile_is_monotone_on_evaluated_factors() {
    let evaluations = varying_evaluations();
    let result = estimate_nested_auroc_breakdown(
        &evaluations,
        &nested_assessment(&evaluations, Execution::Sequential),
        policy(0.01, 0.0, 0.2),
    )
    .unwrap();

    assert!(result.baseline.is_some());
    for pair in result.evaluated_profile.windows(2) {
        assert!(
            pair[1].supported_magnitude_upper <= pair[0].supported_magnitude_upper + 1e-8,
            "{:?}",
            result.evaluated_profile
        );
        assert!(
            pair[1].literal_survival_upper.subset_fraction
                <= pair[0].literal_survival_upper.subset_fraction + 1e-12
        );
    }
}
