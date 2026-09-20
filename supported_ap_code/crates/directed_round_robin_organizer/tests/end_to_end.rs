use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use directed_round_robin_organizer::accelerated::{
    AcceleratedOptions, ComparisonEvidence, ContextExecution, OutputScope, audit_selection,
    plan_accelerated, read_accelerated_plan, read_certificate, run_accelerated,
    validate_accelerated_plan, write_accelerated_plan,
};
use directed_round_robin_organizer::artifact::{
    checksum_path, read_artifact, result_path, run_shard, validate_artifact,
};
use directed_round_robin_organizer::audit::audit_results;
use directed_round_robin_organizer::identity::{
    canonical_label_hash, canonical_vector_hash, sha256_file,
};
use directed_round_robin_organizer::input::{
    BUNDLE_SCHEMA_NAME, BUNDLE_SCHEMA_VERSION, BundleManifest, EvaluationManifest, HashedPath,
    PortableEvaluationHashes, compute_bundle_content_hash, load_bundle,
};
use directed_round_robin_organizer::plan::{
    plan_with_match_target, plan_with_shard_count, validate_plan, write_plan,
};
use directed_round_robin_organizer::reduce::{
    SelectionOutcome, audit_reduction, induce_reduction_view, load_reduction_for,
    reduce_tournament, write_induced_reduction_view, write_reduction,
};
use directed_round_robin_organizer::revision::{
    TournamentReductionSource, TournamentSource, VerdictSource, audit_revision_composition_for,
    compose_revision, compose_revision_from_reductions, write_revision_composition,
};
use directed_round_robin_organizer::spec::{
    ComputationalDesign, EvidencePolicy, MetricKind, PrCnapSpecification, ResamplingUnitSpec,
    SPEC_SCHEMA_VERSION, SelectionStrategy, TournamentSpec,
};
use directed_round_robin_organizer::system::{SystemRecord, SystemRegistry};
use supported_ap::{Prevalence, ReferenceAssessment, SearchOptions, TargetPrevalences};
use tempfile::TempDir;

#[test]
fn accelerated_selection_matches_exhaustive_and_retains_operational_reports() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path();
    build_pr_bundle(&root.join("bundle"), false);
    let mut bundle = load_bundle(&root.join("bundle")).unwrap();
    bundle.spec.selection_strategy = SelectionStrategy::CandidateConservative;
    let full = plan_with_shard_count(&bundle, 1).unwrap();
    run_shard(&bundle, &full, 0, &root.join("full"), 2).unwrap();
    let reduction = reduce_tournament(&bundle, &full, &root.join("full")).unwrap();
    for (name, exhaustive_contexts) in [
        ("ordered", vec![]),
        ("mixed", vec!["eval_b".to_owned()]),
        ("exhaustive", vec!["eval_a".to_owned(), "eval_b".to_owned()]),
    ] {
        let options = AcceleratedOptions {
            batch_size: 1,
            exhaustive_contexts: exhaustive_contexts.into_iter().collect(),
            ..Default::default()
        };
        let plan = plan_accelerated(&bundle, options).unwrap();
        write_accelerated_plan(&plan, &root.join(format!("{name}_plan"))).unwrap();
        let plan = read_accelerated_plan(&root.join(format!("{name}_plan"))).unwrap();
        validate_accelerated_plan(&plan, &bundle).unwrap();
        let results = root.join(format!("{name}_results"));
        let output = root.join(format!("{name}_selection"));
        let summary = run_accelerated(&bundle, &plan, &results, &output, 2).unwrap();
        assert_eq!(summary.survivors, reduction.selection.graph_maximal_systems);
        if name == "ordered" {
            assert!(summary.assessed_matches < full.matches.len());
        }
        if name == "exhaustive" {
            assert_eq!(summary.assessed_matches, full.matches.len());
        }
        let certificate = read_certificate(&output).unwrap();
        let audited = audit_selection(&bundle, &plan, &results, &certificate).unwrap();
        let operational = audited.operational_match_references().unwrap();
        assert_eq!(
            operational.len(),
            audited.survivors().len() * (audited.survivors().len() - 1) / 2
                * bundle.evaluations.len()
        );
        if name == "ordered" {
            let mut missing = certificate.clone();
            let id = &operational[0].match_id;
            missing.matches.retain(|r| &r.match_id != id);
            let error = audit_selection(&bundle, &plan, &results, &missing).unwrap_err();
            assert!(error.to_string().contains("missing S0 operational"));
        }
        for reference in audited.match_references() {
            let actual = audited.read_match(&bundle, &results, reference).unwrap();
            let expected =
                read_artifact(&result_path(&root.join("full"), &reference.match_id)).unwrap();
            assert_eq!(actual.seed, expected.seed);
            assert_eq!(actual.judged, expected.judged);
        }
        let resumed = run_accelerated(
            &bundle,
            &plan,
            &results,
            &root.join(format!("{name}_resumed")),
            1,
        )
        .unwrap();
        assert_eq!(resumed.computed, 0);
        assert_eq!(resumed.already_valid, summary.assessed_matches);
        assert_eq!(
            read_certificate(&root.join(format!("{name}_resumed"))).unwrap(),
            certificate
        );
        for target in audited.survivors() {
            for evaluation in plan.contexts.keys() {
                for challenger in &plan.systems {
                    if challenger != target {
                        assert!(!matches!(
                            audited.comparison(evaluation, challenger, target).unwrap(),
                            ComparisonEvidence::NotComputed { .. }
                        ));
                    }
                }
            }
        }
    }
}

#[test]
fn accelerated_audit_rejects_missing_witnesses_and_false_scope() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path();
    build_pr_bundle(&root.join("bundle"), false);
    let bundle = load_bundle(&root.join("bundle")).unwrap();
    let plan = plan_accelerated(
        &bundle,
        AcceleratedOptions {
            batch_size: 1,
            output_scope: OutputScope::SurvivorSet,
            ..Default::default()
        },
    )
    .unwrap();
    run_accelerated(
        &bundle,
        &plan,
        &root.join("results"),
        &root.join("selection"),
        1,
    )
    .unwrap();
    let certificate = read_certificate(&root.join("selection")).unwrap();
    let audited = audit_selection(&bundle, &plan, &root.join("results"), &certificate).unwrap();
    assert!(audited.operational_match_references().is_err());
    let mut bad = certificate.clone();
    bad.output_scope = OutputScope::SurvivorSetAndOperationalInputs;
    assert!(audit_selection(&bundle, &plan, &root.join("results"), &bad).is_err());
    let mut bad = certificate.clone();
    bad.matches.clear();
    assert!(audit_selection(&bundle, &plan, &root.join("results"), &bad).is_err());
    let mut bad = certificate.clone();
    bad.survivors.push(plan.systems[0].clone());
    assert!(audit_selection(&bundle, &plan, &root.join("results"), &bad).is_err());
    let mut bad_plan = plan.clone();
    if let ContextExecution::OrderedCnap { values, .. } =
        bad_plan.contexts.get_mut("eval_a").unwrap()
    {
        *values.values_mut().next().unwrap() += 0.1;
    }
    assert!(validate_accelerated_plan(&bad_plan, &bundle).is_err());
    let first = &certificate.matches[0];
    let path = result_path(&root.join("results"), &first.match_id);
    fs::remove_file(checksum_path(&path)).unwrap();
    assert!(audit_selection(&bundle, &plan, &root.join("results"), &certificate).is_err());
    assert!(!checksum_path(&path).exists());
    let resumed = run_accelerated(
        &bundle,
        &plan,
        &root.join("results"),
        &root.join("repaired"),
        2,
    )
    .unwrap();
    assert_eq!(resumed.computed, 0);
    assert!(checksum_path(&path).exists());
    let mut artifact = read_artifact(&path).unwrap();
    if let directed_round_robin_organizer::judge::JudgeReport::PrCnap { result, .. } =
        &mut artifact.judged.report
    {
        result.observed_evaluation.forward.value = 100.0;
    }
    fs::write(&path, serde_json::to_vec(&artifact).unwrap()).unwrap();
    let digest = sha256_file(&path).unwrap();
    fs::write(checksum_path(&path), &digest).unwrap();
    let mut inconsistent = certificate.clone();
    inconsistent.matches[0].sha256 = digest;
    let error = audit_selection(&bundle, &plan, &root.join("results"), &inconsistent).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("numerical ordering contradiction")
    );
    assert!(
        audited
            .read_match(&bundle, &root.join("results"), first)
            .is_err()
    );
}

/// Reseal in-memory synthetic variants using the same content identity as the
/// loader. Raw source hashes still name the underlying fixture input files.
fn reseal_fixture(bundle: &mut directed_round_robin_organizer::LoadedBundle) {
    let evaluations: Vec<_> = bundle
        .evaluations
        .values_mut()
        .map(|e| {
            e.score_vector_hashes = e
                .scores
                .iter()
                .map(|(s, v)| (s.clone(), canonical_vector_hash(&e.endpoint_ids, v)))
                .collect();
            PortableEvaluationHashes {
                evaluation_id: e.evaluation_id.clone(),
                label_vector_hash: e.label_vector_hash.clone(),
                score_vector_hashes: e.score_vector_hashes.clone(),
            }
        })
        .collect();
    bundle.spec.evaluations = bundle.evaluations.keys().cloned().collect();
    bundle.policy_hash = bundle.spec.assessment_policy_hash().unwrap();
    bundle.manifest.metric = bundle.spec.metric;
    bundle.manifest.bundle_content_hash =
        compute_bundle_content_hash(&bundle.spec, &bundle.registry, &evaluations).unwrap();
}

#[test]
fn accelerated_plan_handles_4200_candidates_without_enumerating_pairs() {
    let temporary = TempDir::new().unwrap();
    build_pr_bundle(&temporary.path().join("bundle"), false);
    let mut bundle = load_bundle(&temporary.path().join("bundle")).unwrap();
    let prototype = bundle.registry.systems[0].clone();
    bundle.registry.systems = (0..4200)
        .map(|i| {
            let mut record = prototype.clone();
            record.system_id = format!("candidate_{i:04}");
            record.score_column = record.system_id.clone();
            record
        })
        .collect();
    for e in bundle.evaluations.values_mut() {
        e.scores = bundle
            .registry
            .systems
            .iter()
            .map(|s| (s.system_id.clone(), vec![1., 1., 1., 0., 0., 0.]))
            .collect();
    }
    reseal_fixture(&mut bundle);
    let plan = plan_accelerated(&bundle, AcceleratedOptions::default()).unwrap();
    validate_accelerated_plan(&plan, &bundle).unwrap();
    assert_eq!(plan.systems.len(), 4200);
    let serialized = serde_json::to_vec(&plan).unwrap();
    assert!(
        serialized.len() < 1_000_000,
        "compact two-context plan must not contain 17,635,800 pair records"
    );
    let json: serde_json::Value = serde_json::from_slice(&serialized).unwrap();
    assert!(json.get("matches").is_none());
}

#[test]
fn accelerated_pr_and_roc_cover_empty_intersections_ties_and_single_contexts() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path();
    build_pr_bundle(&root.join("bundle"), false);
    for metric in [MetricKind::PrCnap, MetricKind::Auroc] {
        for scenario in ["opponent_switch", "all_tied", "single", "positive_delta"] {
            let mut bundle = load_bundle(&root.join("bundle")).unwrap();
            bundle.spec.selection_strategy = SelectionStrategy::CandidateConservative;
            if scenario == "single" {
                bundle.evaluations.remove("eval_b");
            }
            if scenario == "positive_delta" {
                bundle.spec.evidence_policy.magnitude_threshold = 2.0;
            }
            for (id, e) in &mut bundle.evaluations {
                for (i, scores) in e.scores.values_mut().enumerate() {
                    *scores = if scenario == "all_tied" {
                        vec![0.0; 6]
                    } else if (id == "eval_a" && i == 0) || (id == "eval_b" && i == 3) {
                        vec![1.0, 1.0, 1.0, 0.0, 0.0, 0.0]
                    } else {
                        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0]
                    };
                }
            }
            bundle.spec.metric = metric;
            if metric == MetricKind::Auroc {
                bundle.spec.verdict_field = "baseline.verdict".into();
                bundle.spec.pr_cnap = None;
                bundle.spec.auroc =
                    Some(directed_round_robin_organizer::spec::AurocSpecification {
                        optimization: supported_ap::AuRocOptimizationOptions::default(),
                        concentration_search: supported_ap::ConcentrationSearchOptions::new(
                            0.01, 4,
                        )
                        .unwrap(),
                    });
            } else {
                let pr = bundle.spec.pr_cnap.as_mut().unwrap();
                pr.target_prevalences = TargetPrevalences::closed_interval(
                    Prevalence::new(0.001).unwrap(),
                    Prevalence::new(0.5).unwrap(),
                )
                .unwrap();
                pr.search = SearchOptions::new(9, 1e-6, 8).unwrap();
            }
            reseal_fixture(&mut bundle);
            let name = format!("{metric:?}_{scenario}");
            let full = plan_with_shard_count(&bundle, 1).unwrap();
            let full_results = root.join(format!("{name}_full"));
            run_shard(&bundle, &full, 0, &full_results, 2).unwrap();
            let expected = reduce_tournament(&bundle, &full, &full_results).unwrap();
            for batch_size in [1, 3] {
                let plan = plan_accelerated(
                    &bundle,
                    AcceleratedOptions {
                        batch_size,
                        ..Default::default()
                    },
                )
                .unwrap();
                let results = root.join(format!("{name}_{batch_size}_results"));
                let output = root.join(format!("{name}_{batch_size}_output"));
                let actual = run_accelerated(&bundle, &plan, &results, &output, 2).unwrap();
                assert_eq!(
                    actual.survivors, expected.selection.graph_maximal_systems,
                    "{name}"
                );
                if scenario == "opponent_switch" {
                    assert!(actual.survivors.is_empty());
                }
                if scenario == "all_tied" || scenario == "positive_delta" {
                    assert_eq!(actual.survivors.len(), 4);
                }
                let certificate = read_certificate(&output).unwrap();
                let audited = audit_selection(&bundle, &plan, &results, &certificate).unwrap();
                for r in audited.match_references() {
                    let actual = audited.read_match(&bundle, &results, r).unwrap();
                    let expected = read_artifact(&result_path(&full_results, &r.match_id)).unwrap();
                    assert_eq!(actual.judged, expected.judged);
                }
            }
        }
    }
}

#[test]
fn accelerated_cli_publishes_and_reaudits_a_separate_selection_schema() {
    let temporary = TempDir::new().unwrap();
    let root = temporary.path();
    build_pr_bundle(&root.join("bundle"), false);
    let binary = env!("CARGO_BIN_EXE_directed_round_robin_organizer");
    let run = |args: &[&str]| {
        let result = Command::new(binary)
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        result
    };
    run(&[
        "accelerated",
        "plan",
        "--bundle",
        "bundle",
        "--output",
        "plan",
        "--batch-size",
        "1",
    ]);
    run(&[
        "accelerated",
        "run",
        "--bundle",
        "bundle",
        "--plan",
        "plan",
        "--results",
        "results",
        "--output",
        "selection",
        "--threads",
        "2",
    ]);
    let result = run(&[
        "accelerated",
        "audit",
        "--bundle",
        "bundle",
        "--plan",
        "plan",
        "--results",
        "results",
        "--selection",
        "selection",
    ]);
    let selection: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(selection["survivors"].is_array());
    assert!(!root.join("selection/reduction_manifest.json").exists());
    let invalid = Command::new(binary)
        .current_dir(root)
        .args([
            "reduce",
            "--bundle",
            "bundle",
            "--plan",
            "plan",
            "--results",
            "results",
            "--output",
            "false_full",
        ])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
}

#[test]
fn complete_pr_tournament_is_resumable_reducible_and_auditable() {
    let temporary = TempDir::new().unwrap();
    let bundle_root = temporary.path().join("bundle");
    build_pr_bundle(&bundle_root, false);
    let bundle = load_bundle(&bundle_root).unwrap();
    let plan = plan_with_shard_count(&bundle, 3).unwrap();
    validate_plan(&plan, &bundle).unwrap();
    assert_eq!(plan.expected_match_count, 12);
    let results = temporary.path().join("results");
    for shard in 0..plan.shard_count {
        let first = run_shard(&bundle, &plan, shard, &results, 2).unwrap();
        assert_eq!(first.failed, 0);
        assert_eq!(first.computed, first.assigned);
        let resumed = run_shard(&bundle, &plan, shard, &results, 2).unwrap();
        assert_eq!(resumed.computed, 0);
        assert_eq!(resumed.already_valid, resumed.assigned);
    }
    let audit = audit_results(&bundle, &plan, &results).unwrap();
    assert!(audit.complete);
    assert_eq!(audit.valid_completed_matches, 12);
    let first_match = &plan.matches[0];
    let first_result = result_path(&results, &first_match.match_id);
    let mut old_kernel_result = read_artifact(&first_result).unwrap();
    validate_artifact(&old_kernel_result, &bundle, &plan, first_match).unwrap();
    for obsolete_version in ["0.2.0", "0.2.1"] {
        old_kernel_result.supported_ap.version = obsolete_version.into();
        assert!(
            validate_artifact(&old_kernel_result, &bundle, &plan, first_match).is_err(),
            "results from older numerical kernels/search contracts must not be reused"
        );
    }
    let first_checksum = checksum_path(&first_result);
    let duplicate = results.join("matches/duplicate.json");
    fs::copy(&first_result, &duplicate).unwrap();
    let duplicate_audit = audit_results(&bundle, &plan, &results).unwrap();
    assert!(!duplicate_audit.complete);
    assert_eq!(duplicate_audit.duplicated_results, 1);
    fs::remove_file(&duplicate).unwrap();
    fs::remove_file(&first_checksum).unwrap();
    let missing_checksum_audit = audit_results(&bundle, &plan, &results).unwrap();
    assert!(!missing_checksum_audit.complete);
    assert_eq!(missing_checksum_audit.invalid_or_corrupt_artifacts, 1);
    assert!(!first_checksum.exists(), "audit must not repair artifacts");
    let repaired = run_shard(&bundle, &plan, first_match.shard_id, &results, 2).unwrap();
    assert_eq!(repaired.computed, 0);
    assert_eq!(repaired.already_valid, repaired.assigned);
    assert!(audit_results(&bundle, &plan, &results).unwrap().complete);
    let reduction = reduce_tournament(&bundle, &plan, &results).unwrap();
    assert_eq!(reduction.atomic_directed_verdicts.len(), 24);
    assert_eq!(reduction.evaluation_conjunctive_verdicts.len(), 12);
    assert_eq!(reduction.score_vector_comparisons.len(), 12);
    assert!(!reduction.selection.graph_maximal_systems.is_empty());
    assert!(matches!(
        reduction.selection.outcome,
        SelectionOutcome::UnresolvedCandidateSet { ref system_ids }
            if system_ids == &["m1_max".to_owned(), "m1_mean".to_owned()]
    ));
    // Both selections are always produced; the active one honors the spec.
    assert_eq!(
        reduction.selection.strategy,
        SelectionStrategy::ReplacementConservative
    );
    assert_eq!(
        reduction.alternate_selection.strategy,
        SelectionStrategy::CandidateConservative
    );
    assert_eq!(reduction.per_evaluation_maximal.len(), 2);
    // Candidate-conservative survivors are the intersection of the per-evaluation
    // maximal sets, so every survivor is maximal in every evaluation.
    for survivor in &reduction.alternate_selection.graph_maximal_systems {
        assert!(
            reduction
                .per_evaluation_maximal
                .iter()
                .all(|set| set.maximal_systems.contains(survivor))
        );
    }
    let reloaded = load_reduction_for(
        &{
            let root = temporary.path().join("reduction-roundtrip");
            write_reduction(&reduction, &root).unwrap();
            root
        },
        &bundle,
        &plan,
    )
    .unwrap();
    assert_eq!(reloaded.selection, reduction.selection);
    assert_eq!(reloaded.alternate_selection, reduction.alternate_selection);
    let reduction_root = temporary.path().join("reduction");
    write_reduction(&reduction, &reduction_root).unwrap();
    audit_reduction(&reduction_root).unwrap();
    let view = induce_reduction_view(
        &bundle,
        &reduction,
        &reduction_root,
        &["m2_mean".to_owned()],
    )
    .unwrap();
    assert_eq!(view.included_systems.len(), 3);
    assert_eq!(view.induced_pair_cohort_count, 6);
    assert_eq!(view.atomic_directed_verdicts.len(), 12);
    assert!(
        view.selection
            .survivor_evidence
            .iter()
            .all(|evidence| evidence.comparisons.len() == 2)
    );
    let view_root = temporary.path().join("induced-view");
    write_induced_reduction_view(&view, &view_root).unwrap();
    assert!(view_root.join("induced_view_manifest.json").is_file());
}

#[test]
fn pdac_style_revision_replaces_one_evaluation_and_preserves_sources() {
    let temporary = TempDir::new().unwrap();
    let base_root = temporary.path().join("base-bundle");
    let revision_root = temporary.path().join("revision-bundle");
    build_pr_bundle(&base_root, false);
    build_revision_bundle(&revision_root);
    let base_bundle = load_bundle(&base_root).unwrap();
    let revision_bundle = load_bundle(&revision_root).unwrap();
    let base_plan = plan_with_shard_count(&base_bundle, 1).unwrap();
    let revision_plan = plan_with_shard_count(&revision_bundle, 1).unwrap();
    let base_results = temporary.path().join("base-results");
    let revision_results = temporary.path().join("revision-results");
    run_shard(&base_bundle, &base_plan, 0, &base_results, 2).unwrap();
    run_shard(&revision_bundle, &revision_plan, 0, &revision_results, 2).unwrap();

    let composition = compose_revision(
        TournamentSource {
            bundle: &base_bundle,
            plan: &base_plan,
            results: &base_results,
        },
        TournamentSource {
            bundle: &revision_bundle,
            plan: &revision_plan,
            results: &revision_results,
        },
        "eval_b",
        "pdac_style_revision",
    )
    .unwrap();
    let base_reduction_root = temporary.path().join("base-reduction");
    let revision_reduction_root = temporary.path().join("revision-reduction");
    write_reduction(
        &reduce_tournament(&base_bundle, &base_plan, &base_results).unwrap(),
        &base_reduction_root,
    )
    .unwrap();
    write_reduction(
        &reduce_tournament(&revision_bundle, &revision_plan, &revision_results).unwrap(),
        &revision_reduction_root,
    )
    .unwrap();
    let base_reduction =
        load_reduction_for(&base_reduction_root, &base_bundle, &base_plan).unwrap();
    let revision_reduction =
        load_reduction_for(&revision_reduction_root, &revision_bundle, &revision_plan).unwrap();
    let base_plan_root = temporary.path().join("base-plan");
    let revision_plan_root = temporary.path().join("revision-plan");
    write_plan(&base_plan, &base_plan_root).unwrap();
    write_plan(&revision_plan, &revision_plan_root).unwrap();
    run_cli([
        "audit-reduction",
        "--bundle",
        path(&base_root),
        "--plan",
        path(&base_plan_root),
        "--reduction",
        path(&base_reduction_root),
        "--threads",
        "2",
    ]);
    let cli_composition = temporary.path().join("cli-composition");
    run_cli([
        "compose-revision-from-reductions",
        "--base-bundle",
        path(&base_root),
        "--base-plan",
        path(&base_plan_root),
        "--base-reduction",
        path(&base_reduction_root),
        "--revision-bundle",
        path(&revision_root),
        "--revision-plan",
        path(&revision_plan_root),
        "--revision-reduction",
        path(&revision_reduction_root),
        "--replace-evaluation",
        "eval_b",
        "--revision-id",
        "pdac_style_revision",
        "--output",
        path(&cli_composition),
        "--threads",
        "2",
    ]);
    audit_revision_composition_for(
        &cli_composition,
        &base_plan,
        &revision_plan,
        "eval_b",
        "pdac_style_revision",
    )
    .unwrap();
    let reduction_composition = compose_revision_from_reductions(
        TournamentReductionSource {
            bundle: &base_bundle,
            plan: &base_plan,
            reduction: &base_reduction,
        },
        TournamentReductionSource {
            bundle: &revision_bundle,
            plan: &revision_plan,
            reduction: &revision_reduction,
        },
        "eval_b",
        "pdac_style_revision",
    )
    .unwrap();
    assert_eq!(reduction_composition, composition);
    assert_eq!(composition.atomic_directed_verdicts.len(), 24);
    assert!(composition.atomic_directed_verdicts.iter().all(|row| {
        (row.evaluation_id == "eval_a" && row.source == VerdictSource::Base)
            || (row.evaluation_id == "eval_b" && row.source == VerdictSource::Revision)
    }));
    assert_eq!(composition.evaluation_conjunctive_verdicts.len(), 12);
    let output = temporary.path().join("composition");
    write_revision_composition(&composition, &output).unwrap();
    audit_revision_composition_for(
        &output,
        &base_plan,
        &revision_plan,
        "eval_b",
        "pdac_style_revision",
    )
    .unwrap();
}

#[test]
fn row_and_registry_order_do_not_change_match_ids_or_seeds() {
    let temporary = TempDir::new().unwrap();
    let first_root = temporary.path().join("first");
    let second_root = temporary.path().join("second");
    build_pr_bundle(&first_root, false);
    build_pr_bundle(&second_root, true);
    let first = load_bundle(&first_root).unwrap();
    let second = load_bundle(&second_root).unwrap();
    let first_plan = plan_with_shard_count(&first, 1).unwrap();
    let second_plan = plan_with_shard_count(&second, 4).unwrap();
    let first_scientific: Vec<_> = first_plan
        .matches
        .iter()
        .map(|item| (&item.match_id, item.seed))
        .collect();
    let second_scientific: Vec<_> = second_plan
        .matches
        .iter()
        .map(|item| (&item.match_id, item.seed))
        .collect();
    assert_eq!(
        first.manifest.bundle_content_hash,
        second.manifest.bundle_content_hash
    );
    assert_eq!(first_scientific, second_scientific);
    assert_ne!(first_plan.plan_id, second_plan.plan_id);
}

#[test]
fn rayon_pool_size_does_not_change_scientific_results() {
    let temporary = TempDir::new().unwrap();
    let bundle_root = temporary.path().join("bundle");
    build_pr_bundle(&bundle_root, false);
    let bundle = load_bundle(&bundle_root).unwrap();
    let plan = plan_with_shard_count(&bundle, 1).unwrap();
    let serial_results = temporary.path().join("one-thread-results");
    let parallel_results = temporary.path().join("four-thread-results");

    let serial = run_shard(&bundle, &plan, 0, &serial_results, 1).unwrap();
    let parallel = run_shard(&bundle, &plan, 0, &parallel_results, 4).unwrap();
    assert_eq!(serial.worker_threads, 1);
    assert_eq!(parallel.worker_threads, 4);

    for item in &plan.matches {
        let serial_artifact = read_artifact(&result_path(&serial_results, &item.match_id)).unwrap();
        let parallel_artifact =
            read_artifact(&result_path(&parallel_results, &item.match_id)).unwrap();
        assert_eq!(serial_artifact.judged, parallel_artifact.judged);
        assert_eq!(serial_artifact.seed, parallel_artifact.seed);
        assert_eq!(
            serial_artifact.judge_execution,
            "rayon_shared_pool:1_threads"
        );
        assert_eq!(
            parallel_artifact.judge_execution,
            "rayon_shared_pool:4_threads"
        );
    }
}

#[test]
fn small_target_shards_are_evaluation_stratified() {
    let temporary = TempDir::new().unwrap();
    let bundle_root = temporary.path().join("bundle");
    build_pr_bundle(&bundle_root, false);
    let bundle = load_bundle(&bundle_root).unwrap();
    let plan = plan_with_match_target(&bundle, bundle.evaluations.len()).unwrap();
    let first_evaluations: std::collections::BTreeSet<_> = plan
        .matches
        .iter()
        .filter(|item| item.shard_id == 0)
        .map(|item| item.evaluation_id.as_str())
        .collect();
    assert_eq!(first_evaluations.len(), bundle.evaluations.len());
}

#[test]
fn reduction_fails_closed_before_all_matches_exist() {
    let temporary = TempDir::new().unwrap();
    let bundle_root = temporary.path().join("bundle");
    build_pr_bundle(&bundle_root, false);
    let bundle = load_bundle(&bundle_root).unwrap();
    let plan = plan_with_shard_count(&bundle, 2).unwrap();
    let error =
        reduce_tournament(&bundle, &plan, &temporary.path().join("empty-results")).unwrap_err();
    assert!(error.to_string().contains("operationally incomplete"));
}

#[test]
fn opaque_annotations_and_domain_like_names_are_not_interpreted() {
    let spec = tournament_spec();
    let mut registry = system_registry();
    registry.systems[0].system_id = "count".into();
    registry.systems[0].annotations = std::collections::BTreeMap::from([
        ("domain_group".into(), serde_json::json!("anything")),
        (
            "nested_provider_metadata".into(),
            serde_json::json!({"model": "opaque", "aggregation": [1, 2, 3]}),
        ),
    ]);
    registry.validate(&spec).unwrap();
}

#[test]
fn cli_runs_the_complete_scheduler_neutral_workflow() {
    let temporary = TempDir::new().unwrap();
    let bundle = temporary.path().join("bundle");
    let plan = temporary.path().join("plan");
    let results = temporary.path().join("results");
    let reduction = temporary.path().join("reduction");
    build_pr_bundle(&bundle, false);
    run_cli(["validate-bundle", "--bundle", path(&bundle)]);
    let hash = run_cli(["bundle-content-hash", "--bundle", path(&bundle)]);
    let loaded = load_bundle(&bundle).unwrap();
    assert_eq!(hash.trim(), loaded.manifest.bundle_content_hash);
    run_cli([
        "plan",
        "--bundle",
        path(&bundle),
        "--output",
        path(&plan),
        "--shards",
        "1",
    ]);
    run_cli([
        "run-shard",
        "--bundle",
        path(&bundle),
        "--plan",
        path(&plan),
        "--shard-id",
        "0",
        "--results",
        path(&results),
        "--threads",
        "2",
    ]);
    run_cli(["status", "--plan", path(&plan), "--results", path(&results)]);
    run_cli([
        "reduce",
        "--bundle",
        path(&bundle),
        "--plan",
        path(&plan),
        "--results",
        path(&results),
        "--output",
        path(&reduction),
    ]);
    run_cli([
        "audit",
        "--bundle",
        path(&bundle),
        "--plan",
        path(&plan),
        "--results",
        path(&results),
        "--reduction",
        path(&reduction),
    ]);
}

fn build_pr_bundle(root: &Path, permuted: bool) {
    fs::create_dir_all(root.join("evaluations/eval_a")).unwrap();
    fs::create_dir_all(root.join("evaluations/eval_b")).unwrap();
    let spec = tournament_spec();
    let mut registry = system_registry();
    if permuted {
        registry.systems.reverse();
    }
    write_json(&root.join("tournament_spec.json"), &spec);
    write_json(&root.join("systems.json"), &registry);
    for (evaluation, flip) in [("eval_a", false), ("eval_b", true)] {
        let directory = root.join("evaluations").join(evaluation);
        let endpoints = if permuted {
            "endpoint_id,label\ne6,0\ne4,0\ne2,1\ne1,1\ne5,0\ne3,1\n"
        } else {
            "endpoint_id,label\ne1,1\ne2,1\ne3,1\ne4,0\ne5,0\ne6,0\n"
        };
        fs::write(directory.join("endpoints.csv"), endpoints).unwrap();
        let rows = score_rows(flip);
        let row_order = if permuted {
            [5, 3, 1, 0, 4, 2]
        } else {
            [0, 1, 2, 3, 4, 5]
        };
        let mut scores =
            String::from("endpoint_id,score_m1_max,score_m1_mean,score_m2_max,score_m2_mean\n");
        for index in row_order {
            scores.push_str(&rows[index]);
            scores.push('\n');
        }
        fs::write(directory.join("scores.csv"), scores).unwrap();
        fs::write(directory.join("source_provenance.json"), "{}\n").unwrap();
    }
    let evaluation_names = [("eval_a", false), ("eval_b", true)];
    let evaluations = evaluation_names
        .iter()
        .map(|(evaluation, _)| {
            let base = PathBuf::from("evaluations").join(evaluation);
            EvaluationManifest {
                evaluation_id: (*evaluation).into(),
                endpoints: hashed(root, base.join("endpoints.csv")),
                scores: hashed(root, base.join("scores.csv")),
                source_provenance: hashed(root, base.join("source_provenance.json")),
            }
        })
        .collect();
    let mut manifest = BundleManifest {
        schema_name: BUNDLE_SCHEMA_NAME.into(),
        schema_version: BUNDLE_SCHEMA_VERSION,
        metric: MetricKind::PrCnap,
        bundle_creation_time: "2026-08-13T00:00:00Z".into(),
        systems: hashed(root, "systems.json".into()),
        tournament_spec: hashed(root, "tournament_spec.json".into()),
        evaluations,
        score_provider_identity: "synthetic-test-provider-v1".into(),
        minimum_organizer_schema_version: BUNDLE_SCHEMA_VERSION,
        bundle_content_hash: String::new(),
    };
    let endpoint_ids: Vec<_> = (1..=6).map(|index| format!("e{index}")).collect();
    let labels = vec![true, true, true, false, false, false];
    let portable_evaluations: Vec<_> = evaluation_names
        .iter()
        .map(|(evaluation, flip)| {
            let rows = score_values(*flip);
            let score_vector_hashes = [
                ("m1_max", rows[0].clone()),
                ("m1_mean", rows[1].clone()),
                ("m2_max", rows[2].clone()),
                ("m2_mean", rows[3].clone()),
            ]
            .into_iter()
            .map(|(system, values)| {
                (
                    system.to_owned(),
                    canonical_vector_hash(&endpoint_ids, &values),
                )
            })
            .collect();
            PortableEvaluationHashes {
                evaluation_id: (*evaluation).into(),
                label_vector_hash: canonical_label_hash(&endpoint_ids, &labels),
                score_vector_hashes,
            }
        })
        .collect();
    manifest.bundle_content_hash =
        compute_bundle_content_hash(&spec, &registry, &portable_evaluations).unwrap();
    write_json(&root.join("bundle_manifest.json"), &manifest);
}

fn build_revision_bundle(root: &Path) {
    fs::create_dir_all(root.join("evaluations/eval_b")).unwrap();
    let mut spec = tournament_spec();
    spec.evaluations = vec!["eval_b".into()];
    spec.annotations.insert(
        "context_revision".into(),
        serde_json::json!({"revision_id": "pdac_style_revision"}),
    );
    let registry = system_registry();
    write_json(&root.join("tournament_spec.json"), &spec);
    write_json(&root.join("systems.json"), &registry);
    let directory = root.join("evaluations/eval_b");
    fs::write(
        directory.join("endpoints.csv"),
        "endpoint_id,label\ne1,1\ne2,1\ne3,1\ne4,0\ne5,0\ne6,0\n",
    )
    .unwrap();
    let rows = score_rows(false);
    let mut scores =
        String::from("endpoint_id,score_m1_max,score_m1_mean,score_m2_max,score_m2_mean\n");
    for row in rows {
        scores.push_str(&row);
        scores.push('\n');
    }
    fs::write(directory.join("scores.csv"), scores).unwrap();
    fs::write(directory.join("source_provenance.json"), "{}\n").unwrap();
    let base = PathBuf::from("evaluations/eval_b");
    let evaluations = vec![EvaluationManifest {
        evaluation_id: "eval_b".into(),
        endpoints: hashed(root, base.join("endpoints.csv")),
        scores: hashed(root, base.join("scores.csv")),
        source_provenance: hashed(root, base.join("source_provenance.json")),
    }];
    let endpoint_ids: Vec<_> = (1..=6).map(|index| format!("e{index}")).collect();
    let labels = vec![true, true, true, false, false, false];
    let values = score_values(false);
    let score_vector_hashes = [
        ("m1_max", values[0].clone()),
        ("m1_mean", values[1].clone()),
        ("m2_max", values[2].clone()),
        ("m2_mean", values[3].clone()),
    ]
    .into_iter()
    .map(|(system, values)| {
        (
            system.to_owned(),
            canonical_vector_hash(&endpoint_ids, &values),
        )
    })
    .collect();
    let portable = vec![PortableEvaluationHashes {
        evaluation_id: "eval_b".into(),
        label_vector_hash: canonical_label_hash(&endpoint_ids, &labels),
        score_vector_hashes,
    }];
    let mut manifest = BundleManifest {
        schema_name: BUNDLE_SCHEMA_NAME.into(),
        schema_version: BUNDLE_SCHEMA_VERSION,
        metric: MetricKind::PrCnap,
        bundle_creation_time: "2026-08-17T00:00:00Z".into(),
        systems: hashed(root, "systems.json".into()),
        tournament_spec: hashed(root, "tournament_spec.json".into()),
        evaluations,
        score_provider_identity: "synthetic-revision-provider-v1".into(),
        minimum_organizer_schema_version: BUNDLE_SCHEMA_VERSION,
        bundle_content_hash: String::new(),
    };
    manifest.bundle_content_hash =
        compute_bundle_content_hash(&spec, &registry, &portable).unwrap();
    write_json(&root.join("bundle_manifest.json"), &manifest);
}

fn tournament_spec() -> TournamentSpec {
    TournamentSpec {
        schema_version: SPEC_SCHEMA_VERSION,
        metric: MetricKind::PrCnap,
        verdict_field: "forward.staged_verdict".into(),
        evidence_policy: EvidencePolicy {
            magnitude_threshold: 0.0,
            survival_floor: 0.0,
            survival_requirement: 0.2,
        },
        computational_design: ComputationalDesign {
            replications: 6,
            computational_order: 1,
            replication_positive_count: None,
            replication_negative_count: None,
            resampling_unit: ResamplingUnitSpec::IndependentObservation,
        },
        reference_assessment: ReferenceAssessment::not_asserted("synthetic fixture").unwrap(),
        master_seed: 20260813,
        seed_derivation_version: 1,
        evaluations: vec!["eval_a".into(), "eval_b".into()],
        conjunction_rule: "all_evaluations".into(),
        graph_maximality_rule: "source_strongly_connected_components".into(),
        selection_rule: "source_scc_maximal_vertices".into(),
        selection_strategy: SelectionStrategy::ReplacementConservative,
        pr_cnap: Some(PrCnapSpecification {
            target_prevalences: TargetPrevalences::finite([
                Prevalence::new(0.1).unwrap(),
                Prevalence::new(0.3).unwrap(),
            ])
            .unwrap(),
            search: SearchOptions::default(),
            transport_justification: "synthetic prior shift".into(),
            retain_replication_profiles: false,
        }),
        auroc: None,
        annotations: std::collections::BTreeMap::new(),
        operational_tie_break: None,
    }
}

fn system_registry() -> SystemRegistry {
    let entries = ["m1_max", "m1_mean", "m2_max", "m2_mean"];
    SystemRegistry {
        schema_version: 2,
        systems: entries
            .into_iter()
            .map(|system_id| SystemRecord {
                system_id: system_id.into(),
                display_label: system_id.into(),
                score_column: format!("score_{system_id}"),
                annotations: std::collections::BTreeMap::new(),
            })
            .collect(),
    }
}

fn score_rows(flip: bool) -> [String; 6] {
    let [m1, m1_mean, m2, m2_mean] = score_values(flip);
    std::array::from_fn(|index| {
        format!(
            "e{},{},{},{},{}",
            index + 1,
            m1[index],
            m1_mean[index],
            m2[index],
            m2_mean[index]
        )
    })
}

fn score_values(flip: bool) -> [Vec<f64>; 4] {
    let m1 = if flip {
        [0.95, 0.8, 0.65, 0.35, 0.2, 0.05]
    } else {
        [0.9, 0.8, 0.7, 0.3, 0.2, 0.1]
    };
    let m2 = [0.7, 0.6, 0.4, 0.8, 0.3, 0.2];
    [
        m1.to_vec(),
        m1.iter().map(|value| value - 0.01).collect(),
        m2.to_vec(),
        m2.iter().map(|value| value - 0.01).collect(),
    ]
}

fn hashed(root: &Path, relative: PathBuf) -> HashedPath {
    HashedPath {
        sha256: sha256_file(&root.join(&relative)).unwrap(),
        path: relative,
    }
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) {
    let bytes = serde_json::to_vec_pretty(value).unwrap();
    fs::write(path, bytes).unwrap();
}

fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn run_cli<const N: usize>(arguments: [&str; N]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_directed_round_robin_organizer"))
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn distributed_targets_and_completion_match_local_with_resume_and_missing_receipts() {
    use directed_round_robin_organizer::accelerated::*;
    let temporary = TempDir::new().unwrap();
    let root = temporary.path();
    build_pr_bundle(&root.join("bundle"), false);
    for (name, fallback, roc) in [
        ("ordered", false, false),
        ("fallback", true, false),
        ("roc", false, true),
    ] {
        let base = root.join(name);
        fs::create_dir(&base).unwrap();
        let mut bundle = load_bundle(&root.join("bundle")).unwrap();
        if roc {
            bundle.spec.metric = MetricKind::Auroc;
            bundle.spec.verdict_field = "baseline.verdict".into();
            bundle.spec.pr_cnap = None;
            bundle.spec.auroc = Some(directed_round_robin_organizer::spec::AurocSpecification {
                optimization: supported_ap::AuRocOptimizationOptions::default(),
                concentration_search: supported_ap::ConcentrationSearchOptions::new(0.01, 4)
                    .unwrap(),
            });
            reseal_fixture(&mut bundle);
        }
        let plan = plan_accelerated(
            &bundle,
            AcceleratedOptions {
                batch_size: 1,
                exhaustive_contexts: if fallback {
                    ["eval_b".to_string()].into_iter().collect()
                } else {
                    Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        let local = run_accelerated(
            &bundle,
            &plan,
            &base.join("local_results"),
            &base.join("local"),
            1,
        )
        .unwrap();
        let results = base.join("results");
        let receipts = base.join("targets");
        let survivors = base.join("survivors");
        run_target_shard(&bundle, &plan, &results, &receipts, 3, 0, 1).unwrap();
        assert!(merge_target_shards(&bundle, &plan, &results, &receipts, 3, &survivors).is_err());
        std::thread::scope(|scope| {
            for id in 1..3 {
                let (bundle, plan, results, receipts) = (&bundle, &plan, &results, &receipts);
                scope.spawn(move || {
                    run_target_shard(bundle, plan, results, receipts, 3, id, 2).unwrap()
                });
            }
        });
        // Restart reconstructs the same receipt from validated existing reports.
        run_target_shard(&bundle, &plan, &results, &receipts, 3, 0, 2).unwrap();
        merge_target_shards(&bundle, &plan, &results, &receipts, 3, &survivors).unwrap();
        let partial = read_certificate(&survivors).unwrap();
        assert_eq!(partial.survivors, local.survivors);
        assert!(audit_selection(&bundle, &plan, &results, &partial).is_err());
        let completion = base.join("completion");
        let output = base.join("selection");
        assert!(
            finish_distributed(
                &bundle,
                &plan,
                &results,
                &survivors,
                &completion,
                2,
                &output
            )
            .is_err()
        );
        for id in 0..2 {
            run_completion_shard(&bundle, &plan, &results, &survivors, &completion, 2, id, 2)
                .unwrap();
        }
        finish_distributed(
            &bundle,
            &plan,
            &results,
            &survivors,
            &completion,
            2,
            &output,
        )
        .unwrap();
        finish_distributed(
            &bundle,
            &plan,
            &results,
            &survivors,
            &completion,
            2,
            &output,
        )
        .unwrap();
        let complete = read_certificate(&output).unwrap();
        let audited = audit_selection(&bundle, &plan, &results, &complete).unwrap();
        assert_eq!(audited.survivors(), local.survivors);
        let full = plan_with_shard_count(&bundle, 1).unwrap();
        run_shard(&bundle, &full, 0, &base.join("full_results"), 2).unwrap();
        for reference in audited.match_references() {
            let actual = audited.read_match(&bundle, &results, reference).unwrap();
            let expected = read_artifact(&result_path(
                &base.join("full_results"),
                &reference.match_id,
            ))
            .unwrap();
            assert_eq!(actual.seed, expected.seed);
            assert_eq!(actual.judged, expected.judged);
        }
        let receipt = receipts.join("0/targets.json");
        let mut changed: serde_json::Value =
            serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
        changed["targets"] = serde_json::json!([]);
        fs::write(&receipt, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(
            merge_target_shards(
                &bundle,
                &plan,
                &results,
                &receipts,
                3,
                &base.join("bad_merge")
            )
            .is_err()
        );
    }
}
