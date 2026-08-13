use supported_ap::{
    ApEvidence, ClassCounts, ComputationalReplicationCount, Execution, FiniteEvidenceVerdict,
    MagnitudeThreshold, NestedApAssessment, NestedRowResampling, ObservedApAssessment,
    PairedEvaluation, Prevalence, ProjectedApAssessment, ProjectedResampling, ReferenceAssessment,
    ReplacementPolicy, ResamplingUnit, ScoreTransportAssumption, SearchOptions, SupportOrder,
    SurvivalFloor, SurvivalRequirement, TargetPrevalences, assess_observed_ap,
    assess_staged_nested_ap, assess_staged_projected_ap, support,
};

fn policy(delta: f64, floor: f64, gamma: f64) -> ReplacementPolicy {
    ReplacementPolicy::new(
        MagnitudeThreshold::new(delta).unwrap(),
        SurvivalFloor::new(floor).unwrap(),
        SurvivalRequirement::new(gamma).unwrap(),
    )
}

fn target(values: &[f64]) -> TargetPrevalences {
    TargetPrevalences::finite(
        values
            .iter()
            .copied()
            .map(|value| Prevalence::new(value).unwrap())
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

fn projected_assessment(
    evaluation: &PairedEvaluation,
    execution: Execution,
) -> ProjectedApAssessment {
    ProjectedApAssessment {
        target_prevalences: target(&[0.05, 0.2, 0.5]),
        transport: ScoreTransportAssumption::new("declared prior shift").unwrap(),
        search: SearchOptions::default(),
        resampling: ProjectedResampling::new(
            ComputationalReplicationCount::new(48).unwrap(),
            SupportOrder::new(2).unwrap(),
            evaluation.class_counts(),
            20260805,
            execution,
            ResamplingUnit::IndependentObservation,
        )
        .unwrap(),
        reference_assessment: ReferenceAssessment::not_asserted(
            "heterogeneous risks are not assumed exchangeable",
        )
        .unwrap(),
    }
}

fn example() -> PairedEvaluation {
    PairedEvaluation::new(
        &[0.95, 0.8, 0.7, 0.4, 0.3, 0.1],
        &[0.8, 0.7, 0.4, 0.9, 0.2, 0.1],
        &[true, true, true, false, false, false],
    )
    .unwrap()
}

#[test]
fn projected_assessment_applies_the_staged_anchored_rule() {
    let evaluation = example();
    let result = assess_staged_projected_ap(
        &evaluation,
        &projected_assessment(&evaluation, Execution::Sequential),
        policy(0.0, 0.0, 0.2),
        false,
    )
    .unwrap();
    assert_eq!(result.evidence, ApEvidence::Projected);
    assert_eq!(result.forward.observed_gate.literal_survival.list_length, 1);
    if let Some(full) = &result.forward.full_assessment {
        assert!(full.supported_magnitude <= result.forward.observed_gate.supported_magnitude);
        assert_eq!(result.forward.staged_verdict, full.verdict);
    } else {
        assert_eq!(
            result.forward.staged_verdict,
            FiniteEvidenceVerdict::NoVerdict
        );
    }
}

#[test]
fn reference_assessment_does_not_gate_the_verdict() {
    let evaluation = example();
    let mut not_asserted = projected_assessment(&evaluation, Execution::Sequential);
    let first =
        assess_staged_projected_ap(&evaluation, &not_asserted, policy(0.0, 0.0, 0.2), false)
            .unwrap();
    not_asserted.reference_assessment =
        ReferenceAssessment::exchangeable_labels("randomized unit-test labels").unwrap();
    let second =
        assess_staged_projected_ap(&evaluation, &not_asserted, policy(0.0, 0.0, 0.2), false)
            .unwrap();
    assert_eq!(first.forward.staged_verdict, second.forward.staged_verdict);
    assert_eq!(
        first.forward.observed_gate.supported_magnitude,
        second.forward.observed_gate.supported_magnitude
    );
}

#[test]
fn observed_assessment_uses_the_finite_evaluation_list_directly() {
    let evaluations = vec![
        example(),
        PairedEvaluation::new(
            &[0.9, 0.85, 0.6, 0.4, 0.2, 0.1],
            &[0.8, 0.6, 0.5, 0.7, 0.3, 0.2],
            &[true, true, true, false, false, false],
        )
        .unwrap(),
        PairedEvaluation::new(
            &[0.85, 0.75, 0.65, 0.5, 0.25, 0.05],
            &[0.7, 0.6, 0.4, 0.8, 0.2, 0.1],
            &[true, true, true, false, false, false],
        )
        .unwrap(),
    ];
    let assessment = ObservedApAssessment {
        target_prevalences: target(&[0.1, 0.4]),
        transport: ScoreTransportAssumption::new("declared prior shift").unwrap(),
        search: SearchOptions::default(),
        empirical_order: SupportOrder::new(2).unwrap(),
        execution: Execution::Sequential,
        reference_assessment: ReferenceAssessment::not_asserted("observational evaluations")
            .unwrap(),
    };
    let result = assess_observed_ap(&evaluations, &assessment, policy(0.0, 0.0, 0.5)).unwrap();
    let effects = result
        .forward
        .retained_effects
        .iter()
        .map(|effect| effect.value)
        .collect::<Vec<_>>();
    assert_eq!(result.evidence, ApEvidence::Observed);
    assert_eq!(result.evaluation_count, 3);
    assert_eq!(result.forward.monte_carlo_standard_error, None);
    assert_eq!(
        result.forward.supported_magnitude,
        support(&effects, SupportOrder::new(2).unwrap()).unwrap()
    );
}

#[test]
fn observed_assessment_requires_at_least_k_evaluations() {
    let evaluation = example();
    let assessment = ObservedApAssessment {
        target_prevalences: target(&[0.2]),
        transport: ScoreTransportAssumption::new("declared prior shift").unwrap(),
        search: SearchOptions::default(),
        empirical_order: SupportOrder::new(2).unwrap(),
        execution: Execution::Sequential,
        reference_assessment: ReferenceAssessment::not_asserted("unit test").unwrap(),
    };
    assert!(assess_observed_ap(&[evaluation], &assessment, policy(0.0, 0.0, 0.5)).is_err());
}

#[test]
fn projected_trace_records_the_profiles_used_before_reduction() {
    let evaluation = example();
    let result = assess_staged_projected_ap(
        &evaluation,
        &projected_assessment(&evaluation, Execution::Sequential),
        policy(0.0, 0.0, 0.2),
        true,
    )
    .unwrap();
    let trace = result.trace.unwrap();
    assert_eq!(trace.replication_difference_profiles.len(), 48);
    assert_eq!(trace.forward_retained_effects.len(), 48);
    assert_eq!(trace.computational_replications.get(), 48);
}

#[test]
fn sequential_and_parallel_projected_results_are_seed_identical() {
    let evaluation = example();
    let sequential = assess_staged_projected_ap(
        &evaluation,
        &projected_assessment(&evaluation, Execution::Sequential),
        policy(0.0, 0.0, 0.2),
        true,
    )
    .unwrap();
    let parallel = assess_staged_projected_ap(
        &evaluation,
        &projected_assessment(&evaluation, Execution::Parallel),
        policy(0.0, 0.0, 0.2),
        true,
    )
    .unwrap();
    assert_eq!(sequential, parallel);
}

#[test]
fn failed_observed_gate_stops_before_computational_ap() {
    let evaluation = PairedEvaluation::new(
        &[0.9, 0.7, 0.4, 0.1],
        &[0.9, 0.7, 0.4, 0.1],
        &[true, true, false, false],
    )
    .unwrap();
    let result = assess_staged_projected_ap(
        &evaluation,
        &projected_assessment(&evaluation, Execution::Sequential),
        policy(0.0, 0.0, 0.2),
        false,
    )
    .unwrap();
    assert!(result.trace.is_none());
    assert!(result.mean_diagnostic_difference_profile.is_none());
    assert!(result.forward.full_assessment.is_none());
    assert!(result.forward.secondary_unanchored_projection.is_none());
    assert!(result.reverse.full_assessment.is_none());
    assert!(result.reverse.secondary_unanchored_projection.is_none());
}

#[test]
fn prospective_projected_counts_are_recorded_and_used() {
    let evaluation = example();
    let mut assessment = projected_assessment(&evaluation, Execution::Sequential);
    assessment.resampling.replication_counts = ClassCounts::new(5, 7).unwrap();
    let result =
        assess_staged_projected_ap(&evaluation, &assessment, policy(0.0, 0.0, 0.2), false).unwrap();
    assert_eq!(
        result
            .forward
            .secondary_unanchored_projection
            .as_ref()
            .unwrap()
            .literal_survival
            .list_length,
        48
    );
}

fn nested_assessment(evaluations: &[PairedEvaluation], execution: Execution) -> NestedApAssessment {
    NestedApAssessment {
        target_prevalences: target(&[0.05, 0.2, 0.5]),
        transport: ScoreTransportAssumption::new("declared prior shift").unwrap(),
        search: SearchOptions::default(),
        empirical_order: SupportOrder::new(2).unwrap(),
        computational_order: SupportOrder::new(2).unwrap(),
        rows: evaluations
            .iter()
            .enumerate()
            .map(|(row, evaluation)| NestedRowResampling {
                replications: ComputationalReplicationCount::new(12 + row).unwrap(),
                replication_counts: evaluation.class_counts(),
                seed: 20260806 + row as u64,
                resampling_unit: ResamplingUnit::IndependentObservation,
            })
            .collect(),
        execution,
        reference_assessment: ReferenceAssessment::not_asserted("unit test").unwrap(),
    }
}

fn nested_examples() -> Vec<PairedEvaluation> {
    vec![
        PairedEvaluation::new(
            &[0.95, 0.85, 0.75, 0.35, 0.25, 0.15],
            &[0.80, 0.70, 0.40, 0.90, 0.30, 0.20],
            &[true, true, true, false, false, false],
        )
        .unwrap(),
        PairedEvaluation::new(
            &[0.92, 0.82, 0.72, 0.32, 0.22, 0.12],
            &[0.75, 0.65, 0.45, 0.85, 0.35, 0.25],
            &[true, true, true, false, false, false],
        )
        .unwrap(),
    ]
}

#[test]
fn nested_ap_preserves_rows_and_observed_anchor_domination() {
    let evaluations = nested_examples();
    let assessment = nested_assessment(&evaluations, Execution::Sequential);
    let result = assess_staged_nested_ap(&evaluations, &assessment, policy(0.0, 0.0, 0.2)).unwrap();
    assert_eq!(result.evaluation_count, 2);
    assert_eq!(result.empirical_order.get(), 2);
    let full = result.forward.full_assessment.as_ref().unwrap();
    assert_eq!(full.retained_effect_rows.len(), 2);
    assert_eq!(full.retained_effect_rows[0].computational.len(), 12);
    assert_eq!(full.retained_effect_rows[1].computational.len(), 13);
    assert!(full.supported_magnitude <= result.forward.observed_gate.supported_magnitude + 1e-14);
    assert!(
        full.literal_survival.subset_fraction
            <= result
                .forward
                .observed_gate
                .literal_survival
                .subset_fraction
                + 1e-14
    );
}

#[test]
fn nested_ap_is_seed_identical_across_execution_modes() {
    let evaluations = nested_examples();
    let sequential = assess_staged_nested_ap(
        &evaluations,
        &nested_assessment(&evaluations, Execution::Sequential),
        policy(0.0, 0.0, 0.2),
    )
    .unwrap();
    let parallel = assess_staged_nested_ap(
        &evaluations,
        &nested_assessment(&evaluations, Execution::Parallel),
        policy(0.0, 0.0, 0.2),
    )
    .unwrap();
    assert_eq!(sequential, parallel);
}

#[test]
fn nested_ap_stops_when_the_observed_gate_fails() {
    let evaluations = vec![
        PairedEvaluation::new(
            &[0.9, 0.7, 0.3, 0.1],
            &[0.9, 0.7, 0.3, 0.1],
            &[true, true, false, false],
        )
        .unwrap(),
        PairedEvaluation::new(
            &[0.8, 0.6, 0.4, 0.2],
            &[0.8, 0.6, 0.4, 0.2],
            &[true, true, false, false],
        )
        .unwrap(),
    ];
    let result = assess_staged_nested_ap(
        &evaluations,
        &nested_assessment(&evaluations, Execution::Sequential),
        policy(0.0, 0.0, 0.2),
    )
    .unwrap();
    assert!(result.forward.full_assessment.is_none());
    assert!(result.reverse.full_assessment.is_none());
}
