use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::{Float32Array, StringArray, UInt8Array, UInt32Array};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use directed_round_robin_organizer::load_bundle;
use external_validation_inputs::build_bundle as build_input_bundle;
use iris_fullroster_pipeline::bundle::build_tournament_bundles;
use iris_fullroster_pipeline::io::sha256_file;
use iris_fullroster_pipeline::transfer::{build_transfer_package, validate_transfer_package};
use parquet::arrow::ArrowWriter;
use serde_json::{Value, json};
use tempfile::TempDir;

const COHORTS: [&str; 3] = ["pdac", "covid_spike", "covid_nonspike"];
const BRANCHES: [&str; 2] = ["pr", "roc"];

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn write_json(path: &Path, value: &Value) {
    write(path, &(serde_json::to_string_pretty(value).unwrap() + "\n"));
}

fn write_parquet(path: &Path, schema: Arc<Schema>, batch: RecordBatch) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut writer = ArrowWriter::try_new(File::create(path).unwrap(), schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

fn write_tensor_run(directory: &Path) {
    let observation_schema = Arc::new(Schema::new(vec![
        Field::new("obs_idx", DataType::UInt32, false),
        Field::new("peptide", DataType::Utf8, false),
        Field::new("hla", DataType::Utf8, false),
        Field::new("env_id", DataType::UInt32, false),
        Field::new("patient_id", DataType::Utf8, false),
        Field::new("label", DataType::UInt8, false),
        Field::new("gene", DataType::Utf8, false),
        Field::new("cancer_type", DataType::Utf8, false),
        Field::new("wt_mt_group_id", DataType::Utf8, false),
    ]));
    let observations = RecordBatch::try_new(
        observation_schema.clone(),
        vec![
            Arc::new(UInt32Array::from(vec![0, 1])),
            Arc::new(StringArray::from(vec!["AAAAAAAAA", "BBBBBBBBB"])),
            Arc::new(StringArray::from(vec!["A0101", "B0702"])),
            Arc::new(UInt32Array::from(vec![0, 1])),
            Arc::new(StringArray::from(vec!["P1", "P2"])),
            Arc::new(UInt8Array::from(vec![1, 0])),
            Arc::new(StringArray::from(vec!["G1", "G2"])),
            Arc::new(StringArray::from(vec!["TYPE", "TYPE"])),
            Arc::new(StringArray::from(vec!["W1", "W2"])),
        ],
    )
    .unwrap();
    write_parquet(
        &directory.join("f_tensor.observations.parquet"),
        observation_schema,
        observations,
    );

    let tensor_schema = Arc::new(Schema::new(vec![
        Field::new("obs_idx", DataType::UInt32, false),
        Field::new("param_idx", DataType::UInt32, false),
        Field::new("mn_idx", DataType::UInt32, false),
        Field::new("q_value", DataType::Float32, false),
        Field::new("pos_prob", DataType::Float32, false),
        Field::new("neg_prob", DataType::Float32, false),
        Field::new("pi_eh", DataType::Float32, false),
    ]));
    let tensor = RecordBatch::try_new(
        tensor_schema.clone(),
        vec![
            Arc::new(UInt32Array::from(vec![0, 1])),
            Arc::new(UInt32Array::from(vec![0, 0])),
            Arc::new(UInt32Array::from(vec![0, 0])),
            Arc::new(Float32Array::from(vec![0.9, 0.1])),
            Arc::new(Float32Array::from(vec![0.8, 0.2])),
            Arc::new(Float32Array::from(vec![0.7, 0.3])),
            Arc::new(Float32Array::from(vec![0.6, 0.4])),
        ],
    )
    .unwrap();
    write_parquet(&directory.join("f_tensor.parquet"), tensor_schema, tensor);
    write_json(
        &directory.join("f_tensor.metadata.json"),
        &json!({
            "n_params": 1,
            "n_mn": 1,
            "n_tau": 1,
            "geometry_params": [[1.0, 1.0, 1.0, 1.0]],
            "tau_values": [0.1],
            "mn_tuples": [[1, 1]]
        }),
    );
}

fn tensor_contract(directory: &Path) -> Value {
    json!({
        "directory": directory,
        "tensor_sha256": sha256_file(&directory.join("f_tensor.parquet")).unwrap(),
        "observations_sha256": sha256_file(&directory.join("f_tensor.observations.parquet")).unwrap(),
        "metadata_sha256": sha256_file(&directory.join("f_tensor.metadata.json")).unwrap()
    })
}

struct Fixture {
    _temp: TempDir,
    pipeline_config: PathBuf,
    transfers: PathBuf,
    bundles: PathBuf,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    let env = "env_id,allele_environment\n0,A0101\n1,B0702\n";
    let template =
        "hla_env_dict = \"old-env.csv\"\nquery_peptide_input_tuples_file = \"old-query.csv\"\n";
    let mut dataset_specs = Vec::new();
    for cohort in COHORTS {
        write(&source.join(format!("{cohort}_env.csv")), env);
        write(&source.join(format!("{cohort}_template.toml")), template);
        let (mapping, response_column, response_kind, endpoint_fields, noise) = match cohort {
            "pdac" => (
                "patient_id,env_id,long_peptide,nmer,long_peptide_label\nP1,0,XXAAAAAAAAAYY,AAAAAAAAA,1\nP2,1,XXBBBBBBBBBYY,BBBBBBBBB,0\n",
                "long_peptide_label",
                "binary",
                json!(["patient_id", "long_peptide"]),
                None,
            ),
            "covid_spike" => (
                "patient_id,mutation,env_id,long_peptide,nmer,cd8_IFNg_dmso_adj\nP1,M1,0,XXAAAAAAAAAYY,AAAAAAAAA,1.0\nP2,M2,1,XXBBBBBBBBBYY,BBBBBBBBB,0.0\n",
                "cd8_IFNg_dmso_adj",
                "continuous",
                json!(["patient_id", "mutation", "long_peptide"]),
                Some(0.53),
            ),
            _ => (
                "patient_id,env_id,long_peptide,nmer,cd8_TNFa_IFNg_dmso_adj\nP1,0,XXAAAAAAAAAYY,AAAAAAAAA,1.0\nP2,1,XXBBBBBBBBBYY,BBBBBBBBB,0.0\n",
                "cd8_TNFa_IFNg_dmso_adj",
                "continuous",
                json!(["patient_id", "long_peptide"]),
                None,
            ),
        };
        write(&source.join(format!("{cohort}_mapping.csv")), mapping);
        if cohort == "pdac" {
            write(&source.join("pdac_rojas_sethna_mapping.csv"), mapping);
        }
        let primary_view_id = if cohort == "pdac" {
            "pdac_full"
        } else {
            cohort
        };
        let mut spec = json!({
            "id": cohort.to_ascii_uppercase(),
            "prefix": cohort,
            "expression": "TYPE",
            "env_dict": format!("{cohort}_env.csv"),
            "primary_view_id": primary_view_id,
            "primary_mapping": format!("{cohort}_mapping.csv"),
            "config_template": format!("{cohort}_template.toml"),
            "generated_mono_config": format!("{cohort}_mono.toml"),
            "generated_full_deduplicated_config": format!("{cohort}_full.toml"),
            "response_column": response_column,
            "response_kind": response_kind,
            "endpoint_fields": endpoint_fields,
            "views": []
        });
        if cohort == "pdac" {
            spec["views"] = json!([{
                "id": "pdac_rojas_sethna",
                "role": "secondary",
                "mapping": "pdac_rojas_sethna_mapping.csv",
                "require_subset_of": "pdac_full",
                "selection_eligible": false,
                "bundle_eligible": false
            }]);
        }
        if let Some(value) = noise {
            spec["noise_ceiling_threshold"] = json!(value);
        }
        dataset_specs.push(spec);
    }
    let input_config = root.join("input_config.json");
    write_json(
        &input_config,
        &json!({"schema_version": 2, "datasets": dataset_specs}),
    );
    let input_bundle = root.join("input_bundle");
    build_input_bundle(&input_config, &source, &input_bundle).unwrap();

    let tensors = root.join("tensors");
    let mut cohort_tensor_contracts = BTreeMap::new();
    for cohort in COHORTS {
        let mut representations = serde_json::Map::new();
        for representation in ["full", "focal", "mono"] {
            let directory = tensors.join(cohort).join(representation);
            write_tensor_run(&directory);
            representations.insert(representation.into(), tensor_contract(&directory));
        }
        cohort_tensor_contracts.insert(cohort, Value::Object(representations));
    }

    let nci = root.join("nci_summary.json");
    let regime = json!({
        "d_pos": 1.0,
        "d_neg": 1.0,
        "steepness_pos": 1.0,
        "steepness_neg": 1.0,
        "M": 1,
        "N": 1,
        "regime_idx": 0
    });
    write_json(
        &nci,
        &json!({
            "tau_mode": "per_observation",
            "best_by_pr_auc": regime,
            "best_by_roc_auc": regime,
            "tau_per_observation": {
                "pr": {"histogram": [{"tau_idx": 0, "tau_thymus": 0.1}]},
                "roc": {"histogram": [{"tau_idx": 0, "tau_thymus": 0.1}]}
            }
        }),
    );
    let nci_hash = sha256_file(&nci).unwrap();
    let models = json!({
        "full_hla": {"display_label": "Full HLA", "nci_summary": nci, "nci_summary_sha256": nci_hash, "primary": "full", "query_q": null},
        "focal_hla": {"display_label": "Focal HLA", "nci_summary": nci, "nci_summary_sha256": nci_hash, "primary": "focal", "query_q": null},
        "old_monoallelic": {"display_label": "Old monoallelic", "nci_summary": nci, "nci_summary_sha256": nci_hash, "primary": "mono", "query_q": null},
        "mono_q_full_pn": {"display_label": "Mono-Q / full-PN", "nci_summary": nci, "nci_summary_sha256": nci_hash, "primary": "full", "query_q": "mono"},
        "full_q_mono_pn": {"display_label": "Full-Q / mono-PN", "nci_summary": nci, "nci_summary_sha256": nci_hash, "primary": "mono", "query_q": "full"}
    });
    let cohorts = json!({
        "pdac": {
            "dataset": "pdac", "input_prefix": "pdac", "primary_view_id": "pdac_full", "tensors": cohort_tensor_contracts["pdac"],
            "endpoint_fields": ["patient_id", "long_peptide"], "response_field": null,
            "label_field": "long_peptide_label", "threshold": null, "comparison_operator": null,
            "measurement_error_policy": "committed binary label",
            "views": {
                "pdac_rojas_sethna": {
                    "input_view_id": "pdac_rojas_sethna", "role": "secondary",
                    "selection_eligible": false, "bundle_eligible": false
                }
            }
        },
        "covid_spike": {
            "dataset": "covid_spike", "input_prefix": "covid_spike", "primary_view_id": "covid_spike", "tensors": cohort_tensor_contracts["covid_spike"],
            "endpoint_fields": ["patient_id", "mutation", "long_peptide"], "response_field": "cd8_IFNg_dmso_adj",
            "label_field": null, "threshold": 0.0, "comparison_operator": "greater_than",
            "measurement_error_policy": "mutation-specific threshold-zero label", "views": {}
        },
        "covid_nonspike": {
            "dataset": "covid_nonspike", "input_prefix": "covid_nonspike", "primary_view_id": "covid_nonspike", "tensors": cohort_tensor_contracts["covid_nonspike"],
            "endpoint_fields": ["patient_id", "long_peptide"], "response_field": "cd8_TNFa_IFNg_dmso_adj",
            "label_field": null, "threshold": 0.0, "comparison_operator": "greater_than",
            "measurement_error_policy": "threshold-zero label", "views": {}
        }
    });
    let fixed_l2 = json!([
        {"method":"max"}, {"method":"mean"}, {"method":"median"},
        {"method":"logsumexp"}, {"method":"logmeanexp"},
        {"method":"top_fraction_mean","fraction":0.01},
        {"method":"top_fraction_mean","fraction":0.02},
        {"method":"top_fraction_mean","fraction":0.05},
        {"method":"top_k_mean","k":2}, {"method":"top_k_mean","k":3},
        {"method":"top_k_logsumexp","k":2}, {"method":"top_k_logsumexp","k":10}
    ]);
    let pipeline_config = root.join("pipeline.json");
    write_json(
        &pipeline_config,
        &json!({
            "schema_version": 3,
            "provider_identity": "compact-rust-pipeline-test",
            "input_bundle": input_bundle,
            "input_manifest_sha256": sha256_file(&input_bundle.join("manifest.json")).unwrap(),
            "log_epsilon": 1e-12,
            "models": models,
            "cohorts": cohorts,
            "fixed_l2": fixed_l2,
            "adaptive_l2": {
                "method": "endpoint_local_epitope_second_hla_hybrid",
                "epitope_gate_center": -2.2,
                "epitope_gate_width": 0.13,
                "second_hla_threshold": -6.45,
                "second_hla_gate_width": 0.02,
                "hla_bonus": 1.0,
                "hla_weight": 0.12,
                "solver_absolute_tolerance": 1e-10,
                "solver_max_iterations": 64
            },
            "tournament": {
                "accepted_measurement_error": {},
                "covid_spike_label_specification": {
                    "id":"threshold_zero", "description":"fixture", "threshold":0.0,
                    "comparison_operator":">", "response_field":"cd8_IFNg_dmso_adj"
                },
                "shared": {
                    "evidence_policy": {"magnitude_threshold":0.0,"survival_floor":0.0,"survival_requirement":0.81},
                    "computational_design": {"replications":4,"computational_order":2,"replication_positive_count":null,"replication_negative_count":null,"resampling_unit":"independent_observation"},
                    "reference_assessment": {"status":"not_asserted","reason":"fixture"},
                    "master_seed": 7
                },
                "pr": {
                    "target_prevalences":{"kind":"closed_interval","lower":0.1,"upper":0.3},
                    "search":{"grid_points":3,"tolerance":1e-8,"max_iterations":8},
                    "transport_justification":"fixture", "retain_replication_profiles":false
                },
                "roc": {
                    "optimization":{"absolute_gap":1e-6,"relative_gap":1e-8,"time_limit_seconds":1.0,"solver_threads":1,"backend":"combinatorial","node_budget":100,"restarts":1},
                    "concentration_search":{"tolerance":0.01,"max_iterations":8}
                }
            }
        }),
    );
    Fixture {
        _temp: temp,
        pipeline_config,
        transfers: root.join("transfers"),
        bundles: root.join("tournament_bundles"),
    }
}

#[test]
fn compact_rust_pipeline_reaches_sixty_five_system_bundles_and_plans_6240_matches() {
    let fixture = fixture();
    let wrong_floor_config = fixture.pipeline_config.with_file_name("wrong_floor.json");
    let mut wrong_floor: Value =
        serde_json::from_reader(File::open(&fixture.pipeline_config).unwrap()).unwrap();
    wrong_floor["log_epsilon"] = json!(1e-300);
    write_json(&wrong_floor_config, &wrong_floor);
    let wrong_floor_output = fixture.transfers.with_file_name("wrong_floor_transfers");
    let error = build_transfer_package(&wrong_floor_config, &wrong_floor_output).unwrap_err();
    assert!(error.to_string().contains("log_epsilon must be exactly"));
    assert!(!wrong_floor_output.exists());

    build_transfer_package(&fixture.pipeline_config, &fixture.transfers).unwrap();
    let transfer_manifest: Value =
        serde_json::from_reader(File::open(fixture.transfers.join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(transfer_manifest["jobs"].as_array().unwrap().len(), 40);
    assert!(
        fixture
            .transfers
            .join("secondary/pdac_rojas_sethna/full_hla/pr/summary.json")
            .is_file()
    );
    assert!(
        fixture
            .transfers
            .join("secondary/pdac_rojas_sethna/view_manifest.json")
            .is_file()
    );
    assert!(
        fixture
            .transfers
            .join("secondary/pdac_rojas_sethna/comparison_to_pdac_full.csv")
            .is_file()
    );
    assert_eq!(
        transfer_manifest["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|job| job["view_role"] == "secondary")
            .count(),
        10
    );
    let pipeline: Value =
        serde_json::from_reader(File::open(&fixture.pipeline_config).unwrap()).unwrap();
    let input_bundle = PathBuf::from(pipeline["input_bundle"].as_str().unwrap());
    let mapping = input_bundle.join("pdac_full_longpep_mapping.csv");
    let original_mapping = fs::read(&mapping).unwrap();
    let mut mutated_mapping = original_mapping.clone();
    mutated_mapping.extend_from_slice(b"\n");
    fs::write(&mapping, mutated_mapping).unwrap();
    let stale = validate_transfer_package(&fixture.transfers).unwrap_err();
    assert!(stale.to_string().contains("hash mismatch"));
    fs::write(&mapping, original_mapping).unwrap();
    validate_transfer_package(&fixture.transfers).unwrap();

    let altered_config = fixture
        .pipeline_config
        .with_file_name("altered_pipeline.json");
    let mut altered: Value =
        serde_json::from_reader(File::open(&fixture.pipeline_config).unwrap()).unwrap();
    altered["provider_identity"] = json!("different-provider");
    write_json(&altered_config, &altered);
    let mismatch = build_tournament_bundles(
        &altered_config,
        &fixture.transfers,
        &fixture.bundles.with_file_name("wrong_bundles"),
    )
    .unwrap_err();
    assert!(mismatch.to_string().contains("configuration hash differs"));

    build_tournament_bundles(
        &fixture.pipeline_config,
        &fixture.transfers,
        &fixture.bundles,
    )
    .unwrap();
    for branch in BRANCHES {
        let bundle = load_bundle(&fixture.bundles.join(branch)).unwrap();
        let summary = bundle.summary();
        assert_eq!(summary.system_count, 65);
        assert_eq!(summary.expected_match_count, 6_240);
        assert_eq!(
            bundle
                .registry
                .systems
                .iter()
                .filter(|system| system
                    .system_id
                    .ends_with("__endpoint_local_epitope_second_hla_hybrid_v1"))
                .count(),
            5
        );
        let plan = directed_round_robin_organizer::plan_with_shard_count(&bundle, 3).unwrap();
        assert_eq!(plan.matches.len(), 6_240);
    }

    let pipeline: Value =
        serde_json::from_reader(File::open(&fixture.pipeline_config).unwrap()).unwrap();
    let original_input_bundle = PathBuf::from(pipeline["input_bundle"].as_str().unwrap());
    let relocated = fixture
        .transfers
        .with_file_name("relocated_production_artifacts");
    fs::create_dir(&relocated).unwrap();
    let relocated_config = relocated.join(fixture.pipeline_config.file_name().unwrap());
    let relocated_inputs = relocated.join(original_input_bundle.file_name().unwrap());
    let relocated_transfers = relocated.join(fixture.transfers.file_name().unwrap());
    fs::rename(&fixture.pipeline_config, &relocated_config).unwrap();
    fs::rename(&original_input_bundle, &relocated_inputs).unwrap();
    fs::rename(&fixture.transfers, &relocated_transfers).unwrap();
    validate_transfer_package(&relocated_transfers).unwrap();
}
