use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use directed_round_robin_organizer::artifact::{
    checksum_path, read_artifact, result_path, run_shard,
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
