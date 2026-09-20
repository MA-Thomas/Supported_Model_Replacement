use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use arrow::array::{ArrayRef, Float32Array, StringArray, UInt8Array, UInt32Array};
use arrow::record_batch::RecordBatch;
use directed_round_robin_organizer::accelerated::{
    AcceleratedOptions, plan_accelerated, run_accelerated,
};
use directed_round_robin_organizer::load_bundle;
use iris_nci_parameter_tournament::config::HashedInput;
use iris_nci_parameter_tournament::io::{read_json, sha256_file};
use iris_nci_parameter_tournament::{
    PipelineConfig, RosterMode, audit_prepared, audit_prepared_for_config,
    finalize_version2_accelerated, prepare,
};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use tempfile::TempDir;

fn parquet(path: &Path, columns: Vec<(&str, ArrayRef)>) {
    let batch = RecordBatch::try_from_iter(columns).unwrap();
    let mut writer =
        ArrowWriter::try_new(File::create(path).unwrap(), batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

fn input(path: PathBuf) -> HashedInput {
    HashedInput {
        sha256: sha256_file(&path).unwrap(),
        path,
    }
}

fn fixture(root: &Path, mode: RosterMode) -> PathBuf {
    let observations = root.join("observations.parquet");
    let strings = |v: Vec<&str>| Arc::new(StringArray::from(v)) as ArrayRef;
    parquet(
        &observations,
        vec![
            ("obs_idx", Arc::new(UInt32Array::from(vec![0, 1, 2, 3]))),
            ("peptide", strings(vec!["p0", "p1", "p2", "p3"])),
            ("hla", strings(vec!["H"; 4])),
            ("env_id", Arc::new(UInt32Array::from(vec![0; 4]))),
            (
                "patient_id",
                strings(vec!["person0", "person1", "person2", "person3"]),
            ),
            ("label", Arc::new(UInt8Array::from(vec![1, 0, 1, 0]))),
            ("gene", strings(vec!["gene"; 4])),
            ("cancer_type", strings(vec!["type"; 4])),
            ("wt_mt_group_id", strings(vec!["group"; 4])),
        ],
    );
    let metadata = root.join("metadata.json");
    fs::write(
        &metadata,
        serde_json::to_vec(&serde_json::json!({
            "n_params": 4, "n_mn": 2, "n_tau": 2,
            "geometry_params": [[15.0, 9.0, 1.0, 1.0], [15.0, 9.0, 2.0, 1.0]],
            "mn_tuples": [[1, 1], [1, 2]],
        }))
        .unwrap(),
    )
    .unwrap();
    let tensor = root.join("tensor.parquet");
    let mut obs = Vec::new();
    let mut param = Vec::new();
    let mut mn = Vec::new();
    let mut pos = Vec::new();
    for o in 0..4 {
        for p in 0..4 {
            for m in 0..2 {
                obs.push(o);
                param.push(p);
                mn.push(m);
                pos.push(if o % 2 == 0 {
                    0.6 + 0.03 * p as f32
                } else {
                    0.1 + 0.01 * m as f32
                });
            }
        }
    }
    let n = obs.len();
    parquet(
        &tensor,
        vec![
            ("obs_idx", Arc::new(UInt32Array::from(obs))),
            ("param_idx", Arc::new(UInt32Array::from(param))),
            ("mn_idx", Arc::new(UInt32Array::from(mn))),
            ("q_value", Arc::new(Float32Array::from(vec![0.5; n]))),
            ("pos_prob", Arc::new(Float32Array::from(pos))),
            ("neg_prob", Arc::new(Float32Array::from(vec![0.8; n]))),
            ("pi_eh", Arc::new(Float32Array::from(vec![0.1; n]))),
        ],
    );
    let metrics = root.join("metrics.csv");
    fs::write(&metrics, "regime_idx,param_idx,mn_idx,geometry_idx,d_pos,d_neg,steepness_pos,steepness_neg,M,N,roc_auc,pr_auc\n3,1,1,1,15,9,2,1,1,2,0.9,0.6\n1,0,1,0,15,9,1,1,1,2,0.7,0.8\n2,1,0,1,15,9,2,1,1,1,0.8,0.7\n0,0,0,0,15,9,1,1,1,1,0.6,0.9\n").unwrap();
    let mut config: PipelineConfig = serde_json::from_str(include_str!(
        "../../../../IRIS_scripts/nci_parameter_tournament/config.nci.v1.json"
    ))
    .unwrap();
    config.roster_mode = mode;
    config.candidate_count = if mode == RosterMode::AllRegimes { 4 } else { 2 };
    config.tournament.shared.computational_design.replications = 6;
    config
        .tournament
        .shared
        .computational_design
        .computational_order = 1;
    config.tournament.pr.search = supported_ap::SearchOptions::new(9, 1e-6, 8).unwrap();
    config.tournament.operational_search = config.tournament.pr.search;
    for model in config.models.values_mut() {
        model.metrics = input(metrics.clone());
        model.primary.tensor = input(tensor.clone());
        model.primary.metadata = input(metadata.clone());
        model.primary.observations = input(observations.clone());
        // Exercise native and reciprocal Q-source branches using aligned data.
        if model.query_q.is_some() {
            model.query_q = Some(model.primary.clone());
        }
    }
    let path = root.join("config.json");
    fs::write(&path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    path
}

fn rewrite_parquet(path: &Path, edit: impl FnOnce(RecordBatch) -> RecordBatch) {
    let mut reader = ParquetRecordBatchReaderBuilder::try_new(File::open(path).unwrap())
        .unwrap()
        .build()
        .unwrap();
    let batch = reader.next().unwrap().unwrap();
    assert!(reader.next().is_none());
    drop(reader);
    let batch = edit(batch);
    let mut writer =
        ArrowWriter::try_new(File::create(path).unwrap(), batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

fn replace_column(batch: RecordBatch, name: &str, values: ArrayRef) -> RecordBatch {
    let schema = batch.schema();
    RecordBatch::try_from_iter(
        schema
            .fields()
            .iter()
            .zip(batch.columns())
            .map(|(field, array)| {
                (
                    field.name().as_str(),
                    if field.name() == name {
                        values.clone()
                    } else {
                        array.clone()
                    },
                )
            }),
    )
    .unwrap()
}

fn reseal_fixture(config_path: &Path) {
    let mut config: PipelineConfig = read_json(config_path).unwrap();
    for model in config.models.values_mut() {
        model.primary.tensor = input(model.primary.tensor.path.clone());
        model.primary.observations = input(model.primary.observations.path.clone());
        if model.query_q.is_some() {
            model.query_q = Some(model.primary.clone());
        }
    }
    fs::write(config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
}

#[test]
fn ingestion_rejects_nulls_in_every_required_observation_and_tensor_column() {
    for (file, columns) in [
        (
            "observations.parquet",
            &[
                "obs_idx",
                "peptide",
                "hla",
                "env_id",
                "patient_id",
                "label",
                "gene",
                "cancer_type",
                "wt_mt_group_id",
            ][..],
        ),
        (
            "tensor.parquet",
            &[
                "obs_idx",
                "param_idx",
                "mn_idx",
                "q_value",
                "pos_prob",
                "neg_prob",
                "pi_eh",
            ][..],
        ),
    ] {
        for &column in columns {
            let temp = TempDir::new().unwrap();
            let config = fixture(temp.path(), RosterMode::AllRegimes);
            rewrite_parquet(&temp.path().join(file), |batch| {
                let original = batch.column_by_name(column).unwrap();
                let missing = arrow::array::new_null_array(original.data_type(), 1);
                let first = original.slice(0, 2);
                let rest = original.slice(3, original.len() - 3);
                let values =
                    arrow::compute::concat(&[first.as_ref(), missing.as_ref(), rest.as_ref()])
                        .unwrap();
                replace_column(batch, column, values)
            });
            reseal_fixture(&config);
            let output = temp.path().join("prepared");
            let error = prepare(&config, &output).unwrap_err();
            let message = format!("{error:#}");
            assert!(
                message.contains(file) && message.contains(column) && message.contains("null"),
                "{message}"
            );
            assert!(
                !output.exists(),
                "invalid data must not publish a prepared package"
            );
        }
    }
}

#[test]
fn ingestion_rejects_invalid_binary_labels() {
    let temp = TempDir::new().unwrap();
    let config = fixture(temp.path(), RosterMode::AllRegimes);
    rewrite_parquet(&temp.path().join("observations.parquet"), |batch| {
        replace_column(batch, "label", Arc::new(UInt8Array::from(vec![1, 0, 2, 0])))
    });
    reseal_fixture(&config);
    let message = format!(
        "{:#}",
        prepare(&config, &temp.path().join("prepared")).unwrap_err()
    );
    assert!(
        message.contains("observations.parquet")
            && message.contains("label")
            && message.contains("2"),
        "{message}"
    );
}

#[test]
fn ingestion_rejects_invalid_components_even_when_another_factor_is_zero() {
    for column in ["q_value", "pos_prob", "neg_prob", "pi_eh"] {
        for invalid in [f32::NAN, f32::INFINITY, -0.1] {
            let temp = TempDir::new().unwrap();
            let config = fixture(temp.path(), RosterMode::AllRegimes);
            rewrite_parquet(&temp.path().join("tensor.parquet"), |batch| {
                let rows = batch.num_rows();
                let other = if column == "pos_prob" {
                    "neg_prob"
                } else {
                    "pos_prob"
                };
                let batch =
                    replace_column(batch, other, Arc::new(Float32Array::from(vec![0.0; rows])));
                let mut values = vec![0.5; rows];
                values[2] = invalid;
                replace_column(batch, column, Arc::new(Float32Array::from(values)))
            });
            reseal_fixture(&config);
            let message = format!(
                "{:#}",
                prepare(&config, &temp.path().join("prepared")).unwrap_err()
            );
            assert!(
                message.contains("tensor.parquet") && message.contains(column),
                "{message}"
            );
        }
    }
}

#[test]
fn ingestion_rejects_duplicate_tau_substitutions_and_missing_cells() {
    for defect in ["duplicate", "missing", "missing_observation"] {
        let temp = TempDir::new().unwrap();
        let config = fixture(temp.path(), RosterMode::AllRegimes);
        rewrite_parquet(&temp.path().join("tensor.parquet"), |batch| {
            let mut indices: Vec<u32> = (0..batch.num_rows() as u32).collect();
            match defect {
                "duplicate" => {
                    indices[2] = 0;
                    indices[3] = 1;
                }
                "missing" => {
                    indices.remove(2);
                }
                _ => {
                    indices.drain(..8);
                }
            }
            let indices = UInt32Array::from(indices);
            let columns = batch
                .columns()
                .iter()
                .map(|column| arrow::compute::take(column.as_ref(), &indices, None).unwrap())
                .collect();
            RecordBatch::try_new(batch.schema(), columns).unwrap()
        });
        reseal_fixture(&config);
        let output = temp.path().join("prepared");
        let message = format!("{:#}", prepare(&config, &output).unwrap_err());
        assert!(
            message.contains("tensor.parquet")
                && message.contains("obs_idx")
                && (message.contains("tau")
                    // Parallel model preparation can detect an absent observation
                    // through its Q-source branch before checking tensor cells.
                    || (defect == "missing_observation"
                        && message.contains("Q source contains no value"))),
            "{message}"
        );
        assert!(!output.exists());
    }
}

#[test]
fn ingestion_accepts_real_zero_tensor_values() {
    let temp = TempDir::new().unwrap();
    let config = fixture(temp.path(), RosterMode::AllRegimes);
    rewrite_parquet(&temp.path().join("tensor.parquet"), |batch| {
        let rows = batch.num_rows();
        replace_column(
            batch,
            "q_value",
            Arc::new(Float32Array::from(vec![0.0; rows])),
        )
    });
    reseal_fixture(&config);
    let output = temp.path().join("prepared");
    prepare(&config, &output).unwrap();
    let bundle = load_bundle(&output.join("full_hla/pr/bundle")).unwrap();
    assert!(
        bundle
            .evaluations
            .values()
            .flat_map(|e| e.scores.values())
            .flatten()
            .all(|s| s.is_finite())
    );
}

#[test]
fn complete_grid_prepares_all_ten_bundles_and_runs_accelerated_version2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let config = fixture(root, RosterMode::AllRegimes);
    let output = root.join("prepared");
    let result = Command::new(env!("CARGO_BIN_EXE_iris_nci_parameter_tournament"))
        .args(["prepare", "--config"])
        .arg(&config)
        .arg("--output")
        .arg(&output)
        .args(["--threads", "2"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    audit_prepared_for_config(&output, &config).unwrap();
    let manifest: serde_json::Value = read_json(&output.join("manifest.json")).unwrap();
    assert_eq!(manifest["roster_mode"], "all_regimes");
    for model in iris_nci_parameter_tournament::config::MODELS {
        for branch in ["pr", "roc"] {
            let branch_root = output.join(model).join(branch);
            let roster: serde_json::Value =
                read_json(&branch_root.join("candidate_roster.json")).unwrap();
            assert_eq!(roster["candidate_count"], 4);
            assert_eq!(roster["ranking_rule"], "regime_idx_asc");
            assert_eq!(roster["diversity_key"], serde_json::json!([]));
            let indices: Vec<_> = roster["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["regime_idx"].as_u64().unwrap())
                .collect();
            assert_eq!(indices, [0, 1, 2, 3]);
            let bundle = load_bundle(&branch_root.join("bundle")).unwrap();
            assert_eq!(bundle.registry.systems.len(), 4);
            assert_eq!(
                bundle.spec.annotations["parameter_roster_rule"],
                "all_regimes_in_regime_idx_order"
            );
            assert!(
                bundle
                    .registry
                    .systems
                    .iter()
                    .all(|s| s.display_label.contains("regime"))
            );
        }
        let pr = load_bundle(&output.join(model).join("pr/bundle")).unwrap();
        let roc = load_bundle(&output.join(model).join("roc/bundle")).unwrap();
        for i in 0..4 {
            assert_eq!(
                pr.evaluations["nci"].scores[&format!("{model}__pr__regime_{i}")],
                roc.evaluations["nci"].scores[&format!("{model}__roc__regime_{i}")]
            );
        }
    }
    for branch in ["pr", "roc"] {
        let bundle_root = output.join("full_hla").join(branch).join("bundle");
        let bundle = load_bundle(&bundle_root).unwrap();
        let plan = plan_accelerated(
            &bundle,
            AcceleratedOptions {
                batch_size: 1,
                ..Default::default()
            },
        )
        .unwrap();
        let plan_root = root.join(format!("{branch}_plan"));
        directed_round_robin_organizer::accelerated::write_accelerated_plan(&plan, &plan_root)
            .unwrap();
        let results = root.join(format!("{branch}_results"));
        let selection = root.join(format!("{branch}_selection"));
        let summary = run_accelerated(&bundle, &plan, &results, &selection, 2).unwrap();
        assert_eq!(summary.possible_matches, 6);
        let v2 = root.join(format!("{branch}_v2"));
        finalize_version2_accelerated(&config, &bundle_root, &plan_root, &results, &selection, &v2)
            .unwrap();
        iris_nci_parameter_tournament::audit_version2(&v2).unwrap();
    }
    let different_config = root.join("other_config.json");
    let mut changed: PipelineConfig = read_json(&config).unwrap();
    changed.roster_mode = RosterMode::default();
    changed.candidate_count = 2;
    fs::write(&different_config, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(audit_prepared_for_config(&output, &different_config).is_err());
}

#[test]
fn historical_rosters_still_profile_steepness_and_keep_their_metadata() {
    let temp = TempDir::new().unwrap();
    let config = fixture(temp.path(), RosterMode::default());
    let output = temp.path().join("prepared");
    let summary = prepare(&config, &output).unwrap();
    assert!(summary.branches.iter().all(|b| b.candidate_count == 2));
    audit_prepared(&output).unwrap();
    for (branch, expected) in [("pr", vec![0, 1]), ("roc", vec![3, 2])] {
        let roster: serde_json::Value = read_json(
            &output
                .join("full_hla")
                .join(branch)
                .join("candidate_roster.json"),
        )
        .unwrap();
        assert!(roster.get("roster_mode").is_none());
        assert_eq!(roster["ranking_rule"], "primary_metric_desc_regime_idx_asc");
        assert_eq!(
            roster["diversity_key"],
            serde_json::json!(["d_pos", "d_neg", "M", "N"])
        );
        let actual: Vec<_> = roster["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["regime_idx"].as_u64().unwrap())
            .collect();
        assert_eq!(actual, expected);
    }
}

#[test]
fn all_regimes_rejects_a_metrics_table_that_omits_part_of_the_tensor_grid() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let config_path = fixture(root, RosterMode::AllRegimes);
    let mut config: PipelineConfig = read_json(&config_path).unwrap();
    let metrics = root.join("metrics.csv");
    // A contiguous two-row table is still incomplete relative to the four-regime tensor.
    let contents = fs::read_to_string(&metrics).unwrap();
    let shortened = contents
        .lines()
        .filter(|line| {
            line.starts_with("regime_idx") || line.starts_with("0,") || line.starts_with("1,")
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&metrics, shortened + "\n").unwrap();
    config.candidate_count = 2;
    for model in config.models.values_mut() {
        model.metrics = input(metrics.clone());
    }
    fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let output = root.join("prepared");
    let error = prepare(&config_path, &output).unwrap_err();
    assert!(format!("{error:#}").contains("tensor metadata declares 4 regimes"));
    assert!(!output.exists());
}

#[test]
fn portable_inputs_finalists_and_completed_outputs_survive_relocation() {
    use directed_round_robin_organizer::accelerated::write_accelerated_plan;
    use iris_nci_parameter_tournament::{finalists::*, io::collect_file_hashes, package::*};
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("upstream");
    fs::create_dir(&source).unwrap();
    let original = fixture(&source, RosterMode::AllRegimes);
    let run = temp.path().join("run");
    fs::create_dir(&run).unwrap();
    export_inputs(&original, &run.join("input_package")).unwrap();
    if let Ok(path) = std::env::var("NCI_SMOKE_EXPORT") {
        export_inputs(&original, Path::new(&path)).unwrap();
    }
    // The original directory is deliberately unavailable for the whole pipeline.
    fs::remove_dir_all(source).unwrap();
    let config = run.join("input_package/config.json");
    audit_inputs(&run.join("input_package")).unwrap();
    let prepared = run.join("prepared");
    let result = Command::new(env!("CARGO_BIN_EXE_iris_nci_parameter_tournament"))
        .current_dir(temp.path())
        .args(["prepare", "--config"])
        .arg(&config)
        .arg("--output")
        .arg(&prepared)
        .args(["--threads", "2"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for model in iris_nci_parameter_tournament::config::MODELS {
        for metric in ["pr", "roc"] {
            let bundle_root = prepared.join(model).join(metric).join("bundle");
            let bundle = load_bundle(&bundle_root).unwrap();
            let branch = run.join("evidence").join(model).join(metric);
            fs::create_dir_all(&branch).unwrap();
            let plan = plan_accelerated(
                &bundle,
                AcceleratedOptions {
                    batch_size: 2,
                    ..Default::default()
                },
            )
            .unwrap();
            write_accelerated_plan(&plan, &branch.join("plan")).unwrap();
            run_accelerated(
                &bundle,
                &plan,
                &branch.join("results"),
                &branch.join("selection"),
                2,
            )
            .unwrap();
            let output = run.join("version2").join(model).join(metric);
            fs::create_dir_all(output.parent().unwrap()).unwrap();
            iris_nci_parameter_tournament::finalize_version2_accelerated(
                &config,
                &bundle_root,
                &branch.join("plan"),
                &branch.join("results"),
                &branch.join("selection"),
                &output,
            )
            .unwrap();
        }
    }
    let finalists = run.join("finalists");
    prepare_finalists(&config, &prepared, &run.join("version2"), &finalists).unwrap();
    let branches = audit_finalists(&finalists).unwrap();
    for (metric, branch) in &branches {
        assert!(branch.systems.len() >= 5); // Tied winners all advance.
        let bundle_root = finalists.join(metric).join("bundle");
        let bundle = load_bundle(&bundle_root).unwrap();
        let evidence = run.join(format!("final_evidence_{metric}"));
        fs::create_dir(&evidence).unwrap();
        let plan = plan_accelerated(
            &bundle,
            AcceleratedOptions {
                batch_size: 4,
                ..Default::default()
            },
        )
        .unwrap();
        write_accelerated_plan(&plan, &evidence.join("plan")).unwrap();
        run_accelerated(
            &bundle,
            &plan,
            &evidence.join("results"),
            &evidence.join("selection"),
            2,
        )
        .unwrap();
        let output = run.join("final_version2").join(metric);
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        iris_nci_parameter_tournament::finalize_version2_accelerated(
            &config,
            &bundle_root,
            &evidence.join("plan"),
            &evidence.join("results"),
            &evidence.join("selection"),
            &output,
        )
        .unwrap();
    }
    summarize_components(
        &config,
        &finalists,
        &run.join("final_version2"),
        &run.join("component_winners.json"),
    )
    .unwrap();
    let summary: serde_json::Value = read_json(&run.join("component_winners.json")).unwrap();
    assert_eq!(
        summary["metrics"]["roc"]["winning_components"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert!(summary["metrics"]["roc"]["unique_winning_component"].is_null());
    let moved = temp.path().join("relocated");
    fs::rename(&run, &moved).unwrap();
    let config = moved.join("input_package/config.json");
    audit_inputs(&moved.join("input_package")).unwrap();
    audit_prepared_for_config(&moved.join("prepared"), &config).unwrap();
    summarize_components(
        &config,
        &moved.join("finalists"),
        &moved.join("final_version2"),
        &moved.join("component_winners.json"),
    )
    .unwrap();
    // Parent selection from another component must never be accepted by a builder.
    let wrong = load_bundle(&moved.join("prepared/focal_hla/pr/bundle")).unwrap();
    assert!(audit_selection_for(&moved.join("version2/full_hla/pr"), &config, &wrong).is_err());
    let inputs = moved.join("input_package");
    let file = collect_file_hashes(&inputs)
        .unwrap()
        .keys()
        .find(|p| p.starts_with("inputs/"))
        .unwrap()
        .clone();
    fs::write(inputs.join(file), "changed").unwrap();
    assert!(audit_inputs(&inputs).is_err());
}
