use supported_ap::*;

fn valley() -> PairedEvaluation {
    PairedEvaluation::new(
        &[
            20., 19., 16., 15., 12., 11., 9., 6., 2., 1., 18., 17., 14., 13., 10., 8., 7., 5., 4.,
            3.,
        ],
        &[
            18., 17., 16., 15., 13., 10., 9., 8., 6., 2., 20., 19., 14., 12., 11., 7., 5., 4., 3.,
            1.,
        ],
        &(0..20).map(|i| i < 10).collect::<Vec<_>>(),
    )
    .unwrap()
}

fn interval(left: f64, right: f64) -> TargetPrevalences {
    TargetPrevalences::closed_interval(
        Prevalence::new(left).unwrap(),
        Prevalence::new(right).unwrap(),
    )
    .unwrap()
}

fn policy(delta: f64, floor: f64) -> ReplacementPolicy {
    ReplacementPolicy::new(
        MagnitudeThreshold::new(delta).unwrap(),
        SurvivalFloor::new(floor).unwrap(),
        SurvivalRequirement::new(0.5).unwrap(),
    )
}

fn assessment(target: TargetPrevalences, search: SearchOptions) -> ObservedApAssessment {
    ObservedApAssessment {
        target_prevalences: target,
        search,
        transport: ScoreTransportAssumption::new("test prior shift").unwrap(),
        empirical_order: SupportOrder::new(1).unwrap(),
        execution: Execution::Sequential,
        reference_assessment: ReferenceAssessment::not_asserted("test").unwrap(),
    }
}

#[test]
fn endpoint_cell_valley_is_bounded_and_cannot_produce_false_replacement() {
    let evaluation = valley();
    let x = 0.441291675958276;
    let witness = evaluation.tie_averaged_difference(Prevalence::new(x).unwrap());
    assert!((witness - 0.18311061901043554).abs() < 2e-14);
    for (grid, offset) in [(3, 0.05), (257, 0.0001)] {
        let target = interval(x - offset, 0.99);
        let search = SearchOptions::new(grid, 1e-8, 128).unwrap();
        let result = assess_observed_ap(
            std::slice::from_ref(&evaluation),
            &assessment(target.clone(), search),
            policy(0.1832, 0.0),
        )
        .unwrap();
        let certificate = result.evaluations[0].forward.search.unwrap();
        assert!(certificate.lower_bound <= witness);
        assert!(certificate.upper_bound - witness <= 1e-8);
        assert_eq!(
            certificate.stop_reason,
            PrevalenceSearchStop::AccuracyReached
        );
        assert_eq!(result.forward.verdict, FiniteEvidenceVerdict::NoVerdict);
        assert_eq!(
            result.forward.prevalence_search.unwrap().verdict,
            PolicyVerdict::VerifiedFailure
        );
        // More than one direction uses the same evaluations, not two budgets.
        assert_eq!(certificate.evaluations, grid + certificate.iterations);
        assert!(certificate.iterations <= search.max_iterations);
        assert_eq!(
            certificate.evaluations,
            result.evaluations[0].reverse.search.unwrap().evaluations
        );
        // Independent dense evaluations must lie within the returned global range.
        for i in 0..=2000 {
            let pi = x - offset + (0.99 - x + offset) * i as f64 / 2000.0;
            let d = evaluation.tie_averaged_difference(Prevalence::new(pi).unwrap());
            assert!(d >= certificate.lower_bound - 1e-14);
            assert!(d <= -result.evaluations[0].reverse.value + 1e-14);
        }
    }
}

#[test]
fn exhausted_budget_is_explicit_and_more_work_only_tightens_bounds() {
    let evaluation = valley();
    let target = interval(0.391291675958276, 0.99);
    let mut previous = None;
    for budget in [1, 2, 4, 8, 16, 64, 128] {
        let search = SearchOptions::new(3, 1e-10, budget).unwrap();
        let result = assess_observed_ap(
            std::slice::from_ref(&evaluation),
            &assessment(target.clone(), search),
            policy(0.1832, 0.0),
        )
        .unwrap();
        let certificate = result.evaluations[0].forward.search.unwrap();
        if budget == 1 {
            assert_eq!(
                certificate.stop_reason,
                PrevalenceSearchStop::BudgetExhausted
            );
            assert_eq!(result.forward.verdict, FiniteEvidenceVerdict::NoVerdict);
            assert_eq!(
                result.forward.prevalence_search.unwrap().verdict,
                PolicyVerdict::Unresolved
            );
        }
        if let Some((lower, upper)) = previous {
            assert!(certificate.lower_bound >= lower);
            assert!(certificate.upper_bound <= upper);
        }
        previous = Some((certificate.lower_bound, certificate.upper_bound));
        assert!(certificate.evaluations <= 3 + budget);
    }
}

#[test]
fn finite_sets_stay_finite_and_have_no_interval_claim() {
    let evaluation = valley();
    let points = [0.1, 0.4, 0.9].map(|p| Prevalence::new(p).unwrap());
    let expected: Vec<_> = points
        .iter()
        .map(|&p| evaluation.tie_averaged_difference(p))
        .collect();
    let (a, b) = evaluation
        .retained_effects_for_multiplicities(
            &[1; 20],
            &TargetPrevalences::finite(points).unwrap(),
            SearchOptions::default(),
        )
        .unwrap();
    assert_eq!(
        a.value,
        expected.iter().copied().fold(f64::INFINITY, f64::min)
    );
    assert_eq!(
        b.value,
        -expected.iter().copied().fold(f64::NEG_INFINITY, f64::max)
    );
    assert!(a.search.is_none() && b.search.is_none());
}

#[test]
fn deserialized_invalid_prevalence_cannot_reach_the_bound_oracle() {
    let evaluation = valley();
    for json in [
        r#"{"kind":"closed_interval","lower":0.0,"upper":0.9}"#,
        r#"{"kind":"closed_interval","lower":0.1,"upper":1.0}"#,
        r#"{"kind":"finite","values":[-0.1]}"#,
    ] {
        let target: TargetPrevalences = serde_json::from_str(json).unwrap();
        assert!(
            evaluation
                .retained_effects_for_multiplicities(&[1; 20], &target, SearchOptions::default())
                .is_err()
        );
    }
}

#[test]
fn external_nested_rows_cannot_silently_supply_inverted_search_bounds() {
    let mut effect = valley()
        .retained_effects_for_multiplicities(
            &[1; 20],
            &interval(0.1, 0.9),
            SearchOptions::default(),
        )
        .unwrap()
        .0;
    effect.search.as_mut().unwrap().upper_bound = effect.value - 1.0;
    let row = NestedRetainedEffectRow {
        observed: effect,
        computational: vec![effect],
    };
    assert!(matches!(
        DirectionalNestedFiniteEvidence::from_retained_effect_rows(
            vec![row],
            SupportOrder::new(1).unwrap(),
            SupportOrder::new(1).unwrap(),
            policy(0.0, 0.0)
        ),
        Err(Error::InvalidPrevalenceSearchCertificate)
    ));
}

#[test]
fn all_small_binary_rankings_enclose_both_directions_and_use_strict_thresholds() {
    let rankings: Vec<_> = (0usize..64)
        .filter(|mask| mask.count_ones() == 3)
        .map(|mask| {
            let mut scores = [0.0; 6];
            let (mut positive, mut negative) = (0, 3);
            for rank in 0..6 {
                let index = if mask & (1 << rank) != 0 {
                    let index = positive;
                    positive += 1;
                    index
                } else {
                    let index = negative;
                    negative += 1;
                    index
                };
                scores[index] = (6 - rank) as f64;
            }
            scores
        })
        .collect();
    for a in &rankings {
        for b in &rankings {
            let evaluation =
                PairedEvaluation::new(a, b, &[true, true, true, false, false, false]).unwrap();
            let result = assess_observed_ap(
                std::slice::from_ref(&evaluation),
                &assessment(interval(0.01, 0.99), SearchOptions::default()),
                policy(0.0, 0.0),
            )
            .unwrap();
            let forward = result.evaluations[0].forward;
            let reverse = result.evaluations[0].reverse;
            assert!(forward.value <= forward.search.unwrap().upper_bound);
            assert!(reverse.value <= reverse.search.unwrap().upper_bound);
            for i in 0..=200 {
                let pi = Prevalence::new(0.01 + 0.98 * i as f64 / 200.0).unwrap();
                let value = evaluation.tie_averaged_difference(pi);
                assert!(forward.value <= value + 1e-13 && reverse.value <= -value + 1e-13);
                if result.forward.verdict == FiniteEvidenceVerdict::SupportedReplacement {
                    assert!(value > 0.0);
                }
                if result.reverse.verdict == FiniteEvidenceVerdict::SupportedReplacement {
                    assert!(value < 0.0);
                }
            }
            if a == b {
                assert_eq!(
                    result.forward.prevalence_search.unwrap().verdict,
                    PolicyVerdict::VerifiedFailure
                );
                assert_eq!(
                    result.reverse.prevalence_search.unwrap().verdict,
                    PolicyVerdict::VerifiedFailure
                );
            }
        }
    }
}

#[test]
fn flat_singleton_and_machine_adjacent_intervals_are_well_defined() {
    let identical =
        PairedEvaluation::new(&[0.0; 4], &[0.0; 4], &[true, true, false, false]).unwrap();
    let (forward, reverse) = identical
        .retained_effects_for_multiplicities(
            &[1; 4],
            &interval(0.0001, 0.9999),
            SearchOptions::default(),
        )
        .unwrap();
    for effect in [forward, reverse] {
        assert_eq!(effect.value, 0.0);
        assert_eq!(effect.search.unwrap().iterations, 0);
    }
    let evaluation = valley();
    for (left, right) in [
        (0.4, 0.4),
        (0.4, f64::from_bits(0.4f64.to_bits() + 1)),
        (f64::from_bits(1), f64::from_bits(1)),
        (1.0 - f64::EPSILON, 1.0 - f64::EPSILON),
    ] {
        let (effect, _) = evaluation
            .retained_effects_for_multiplicities(
                &[1; 20],
                &interval(left, right),
                SearchOptions::new(3, 1e-30, 10).unwrap(),
            )
            .unwrap();
        let certificate = effect.search.unwrap();
        assert!(certificate.lower_bound.is_finite() && certificate.upper_bound.is_finite());
        assert!(certificate.lower_bound <= certificate.upper_bound);
        assert!(certificate.evaluations <= 2);
        assert_eq!(
            certificate.stop_reason,
            PrevalenceSearchStop::FloatingPointLimit
        );
    }
}

#[test]
fn interval_projected_and_nested_results_are_deterministic_and_serialize_bounds() {
    let evaluation = valley();
    let projected = |execution| ProjectedApAssessment {
        target_prevalences: interval(0.1, 0.9),
        search: SearchOptions::new(3, 1e-7, 64).unwrap(),
        transport: ScoreTransportAssumption::new("test").unwrap(),
        resampling: ProjectedResampling::new(
            ComputationalReplicationCount::new(8).unwrap(),
            SupportOrder::new(2).unwrap(),
            evaluation.class_counts(),
            9123,
            execution,
            ResamplingUnit::IndependentObservation,
        )
        .unwrap(),
        reference_assessment: ReferenceAssessment::not_asserted("test").unwrap(),
    };
    let sequential = assess_staged_projected_ap(
        &evaluation,
        &projected(Execution::Sequential),
        policy(0.0, 0.0),
        true,
    )
    .unwrap();
    let parallel = assess_staged_projected_ap(
        &evaluation,
        &projected(Execution::Parallel),
        policy(0.0, 0.0),
        true,
    )
    .unwrap();
    assert_eq!(sequential, parallel);
    let full = sequential.forward.full_assessment.as_ref().unwrap();
    let bounds = full.prevalence_search.unwrap();
    assert!(bounds.supported_magnitude_lower <= bounds.supported_magnitude_upper);
    assert!(bounds.literal_survival_lower <= bounds.literal_survival_upper);
    for effect in &full.retained_effects {
        assert!(effect.value <= effect.search.unwrap().upper_bound);
    }
    let json = serde_json::to_string(&sequential).unwrap();
    assert_eq!(
        serde_json::from_str::<StagedProjectedApResult>(&json).unwrap(),
        sequential
    );
    let mut nested = NestedApAssessment {
        target_prevalences: interval(0.1, 0.9),
        search: SearchOptions::new(3, 1e-7, 32).unwrap(),
        transport: ScoreTransportAssumption::new("test").unwrap(),
        empirical_order: SupportOrder::new(2).unwrap(),
        computational_order: SupportOrder::new(2).unwrap(),
        rows: vec![
            NestedRowResampling {
                replications: ComputationalReplicationCount::new(4).unwrap(),
                replication_counts: evaluation.class_counts(),
                seed: 21,
                resampling_unit: ResamplingUnit::IndependentObservation
            };
            2
        ],
        execution: Execution::Sequential,
        reference_assessment: ReferenceAssessment::not_asserted("test").unwrap(),
    };
    let evaluations = [evaluation.clone(), evaluation];
    let first = assess_staged_nested_ap(&evaluations, &nested, policy(0.0, 0.0)).unwrap();
    nested.execution = Execution::Parallel;
    assert_eq!(
        first,
        assess_staged_nested_ap(&evaluations, &nested, policy(0.0, 0.0)).unwrap()
    );
    let bounds = first
        .forward
        .full_assessment
        .unwrap()
        .prevalence_search
        .unwrap();
    assert!(bounds.supported_magnitude_lower <= bounds.supported_magnitude_upper);
    assert!(bounds.literal_survival_lower <= bounds.literal_survival_upper);
}

/// Reproducible microbenchmark, deliberately excluded from normal tests.
/// Run with --release --test prevalence_search -- --ignored --nocapture.
#[test]
#[ignore]
fn compare_adaptive_search_with_257_point_scan() {
    use std::{hint::black_box, time::Instant};
    for tie_width in [1, 10, 100] {
        let n = 1000;
        let a: Vec<_> = (0..n)
            .map(|i| (((i * 37 + 11) % n) / tie_width) as f64)
            .collect();
        let b: Vec<_> = (0..n)
            .map(|i| (((i * 73 + 3) % n) / tie_width) as f64)
            .collect();
        let labels: Vec<_> = (0..n).map(|i| i % 3 == 0).collect();
        let evaluation = PairedEvaluation::new(&a, &b, &labels).unwrap();
        let start = Instant::now();
        let mut grid_min = f64::INFINITY;
        let mut grid_max = f64::NEG_INFINITY;
        for i in 0..257 {
            let pi = Prevalence::new(0.01 + 0.89 * i as f64 / 256.0).unwrap();
            let value = black_box(evaluation.tie_averaged_difference(pi));
            grid_min = grid_min.min(value);
            grid_max = grid_max.max(value);
        }
        let scan_ms = start.elapsed().as_secs_f64() * 1000.0;
        for grid in [3, 9, 17] {
            let start = Instant::now();
            let result = black_box(
                evaluation
                    .retained_effects_for_multiplicities(
                        &vec![1; n],
                        &interval(0.01, 0.9),
                        SearchOptions::new(grid, 1e-8, 128).unwrap(),
                    )
                    .unwrap(),
            );
            let search_ms = start.elapsed().as_secs_f64() * 1000.0;
            let certificate = result.0.search.unwrap();
            assert!(certificate.lower_bound <= grid_min + 1e-12);
            assert!(-result.1.value >= grid_max - 1e-12);
            println!(
                "n={n} tie_width={tie_width} grid={grid}: scan_ms={scan_ms:.3} adaptive_ms={search_ms:.3} evaluations={} iterations={} gap={:.3e} stop={:?}",
                certificate.evaluations,
                certificate.iterations,
                certificate.upper_bound - certificate.lower_bound,
                certificate.stop_reason
            );
        }
    }
}
