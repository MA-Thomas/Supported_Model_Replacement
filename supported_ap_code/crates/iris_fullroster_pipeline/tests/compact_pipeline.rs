use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow::array::{Float32Array, StringArray, UInt8Array, UInt32Array};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use directed_round_robin_organizer::load_bundle;
use external_validation_inputs::build_bundle as build_input_bundle;
use iris_fullroster_pipeline::bundle::{build_combined_bundles, build_fixed_bundles};
use iris_fullroster_pipeline::io::sha256_file;
use iris_fullroster_pipeline::transfer::{build_transfer_package, validate_transfer_package};
use parquet::arrow::ArrowWriter;
use serde_json::{Value, json};
use tempfile::TempDir;

const COHORTS: [&str; 3] = ["pdac", "covid_spike", "covid_nonspike"];
const MODELS: [&str; 5] = [
    "full_hla",
    "focal_hla",
    "old_monoallelic",
    "mono_q_full_pn",
    "full_q_mono_pn",
];
const BRANCHES: [&str; 2] = ["pr", "roc"];
const POLICIES: [&str; 2] = ["pdac_only", "all_contexts_equal_weight"];

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
    fixed: PathBuf,
    selection: PathBuf,
    combined: PathBuf,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    let query = "peptide,HLA-RE,PatientID,TCGA_EXPR_TYPE,env_id,gene\n\
        AAAAAAAAA,A0101,P1,TYPE,0,G1\n\
        BBBBBBBBB,B0702,P2,TYPE,1,G2\n";
    let env = "env_id,allele_environment\n0,A0101\n1,B0702\n";
    let template =
        "hla_env_dict = \"old-env.csv\"\nquery_peptide_input_tuples_file = \"old-query.csv\"\n";
    let mut dataset_specs = Vec::new();
    for cohort in COHORTS {
        write(&source.join(format!("{cohort}_query.csv")), query);
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
        let mut spec = json!({
            "id": cohort.to_ascii_uppercase(),
            "prefix": cohort,
            "query": format!("{cohort}_query.csv"),
            "env_dict": format!("{cohort}_env.csv"),
            "mapping": format!("{cohort}_mapping.csv"),
            "config_template": format!("{cohort}_template.toml"),
            "generated_mono_config": format!("{cohort}_mono.toml"),
            "response_column": response_column,
            "response_kind": response_kind,
            "endpoint_fields": endpoint_fields
        });
        if let Some(value) = noise {
            spec["noise_ceiling_threshold"] = json!(value);
        }
        dataset_specs.push(spec);
    }
    let input_config = root.join("input_config.json");
    write_json(
        &input_config,
        &json!({"schema_version": 1, "datasets": dataset_specs}),
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
            "dataset": "pdac", "input_prefix": "pdac", "tensors": cohort_tensor_contracts["pdac"],
            "endpoint_fields": ["patient_id", "long_peptide"], "response_field": null,
            "label_field": "long_peptide_label", "threshold": null, "comparison_operator": null,
            "measurement_error_policy": "committed binary label"
        },
        "covid_spike": {
            "dataset": "covid_spike", "input_prefix": "covid_spike", "tensors": cohort_tensor_contracts["covid_spike"],
            "endpoint_fields": ["patient_id", "mutation", "long_peptide"], "response_field": "cd8_IFNg_dmso_adj",
            "label_field": null, "threshold": 0.0, "comparison_operator": "greater_than",
            "measurement_error_policy": "mutation-specific threshold-zero label"
        },
        "covid_nonspike": {
            "dataset": "covid_nonspike", "input_prefix": "covid_nonspike", "tensors": cohort_tensor_contracts["covid_nonspike"],
            "endpoint_fields": ["patient_id", "long_peptide"], "response_field": "cd8_TNFa_IFNg_dmso_adj",
            "label_field": null, "threshold": 0.0, "comparison_operator": "greater_than",
            "measurement_error_policy": "threshold-zero label"
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
            "schema_version": 1,
            "provider_identity": "compact-rust-pipeline-test",
            "input_bundle": input_bundle,
            "input_manifest_sha256": sha256_file(&input_bundle.join("manifest.json")).unwrap(),
            "log_epsilon": 1e-12,
            "models": models,
            "cohorts": cohorts,
            "fixed_l2": fixed_l2,
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
        fixed: root.join("fixed"),
        selection: root.join("selection"),
        combined: root.join("combined"),
    }
}

fn read_csv(path: &Path) -> (csv::StringRecord, Vec<csv::StringRecord>) {
    let mut reader = csv::Reader::from_path(path).unwrap();
    let headers = reader.headers().unwrap().clone();
    let rows = reader.records().map(|row| row.unwrap()).collect();
    (headers, rows)
}

fn build_selection_fixture(fixture: &Fixture) {
    fs::create_dir(&fixture.selection).unwrap();
    let parameter_path = fixture.selection.join("selected_parameters.csv");
    let mut parameters = csv::Writer::from_path(&parameter_path).unwrap();
    parameters
        .write_record([
            "selection_policy",
            "model",
            "branch",
            "selection_metric",
            "q_id",
            "q",
            "c",
            "kappa",
            "c_at_boundary",
            "kappa_at_boundary",
            "reference_system_id",
            "pdac_cnap_staged_supported",
            "covid_spike_cnap_staged_supported",
            "covid_nonspike_cnap_staged_supported",
            "system_id",
        ])
        .unwrap();
    for policy in POLICIES {
        for branch in BRANCHES {
            for model in MODELS {
                let suffix = if policy == "pdac_only" {
                    "self_gated_hillq_pdac_selected"
                } else {
                    "self_gated_hillq_all_contexts_selected"
                };
                let reference_system_id = if branch == "pr" {
                    format!("{model}__max")
                } else {
                    String::new()
                };
                let system_id = format!("{model}__{suffix}");
                parameters
                    .write_record([
                        policy,
                        model,
                        branch,
                        if branch == "pr" {
                            "paired_supported_cnap_vs_model_matched_max"
                        } else {
                            "auroc"
                        },
                        "q2",
                        "2",
                        "-1",
                        "0.5",
                        "False",
                        "False",
                        &reference_system_id,
                        if branch == "pr" { "True" } else { "" },
                        if branch == "pr" { "True" } else { "" },
                        if branch == "pr" { "True" } else { "" },
                        &system_id,
                    ])
                    .unwrap();
            }
        }
    }
    parameters.flush().unwrap();

    let endpoint_score_path = fixture.selection.join("selected_endpoint_scores.csv");
    let mut endpoint_scores = csv::Writer::from_path(&endpoint_score_path).unwrap();
    endpoint_scores
        .write_record([
            "selection_policy",
            "model",
            "branch",
            "cohort",
            "endpoint_id",
            "label",
            "self_gated_hillq_score",
        ])
        .unwrap();
    for branch in BRANCHES {
        for cohort in COHORTS {
            let evaluation = fixture.fixed.join(branch).join("evaluations").join(cohort);
            let (_, endpoints) = read_csv(&evaluation.join("endpoints.csv"));
            let (score_headers, score_rows) = read_csv(&evaluation.join("scores.csv"));
            let endpoint_col = score_headers
                .iter()
                .position(|field| field == "endpoint_id")
                .unwrap();
            let score_by_endpoint: BTreeMap<_, _> = score_rows
                .iter()
                .map(|row| (row[endpoint_col].to_owned(), row.clone()))
                .collect();
            for policy in POLICIES {
                for model in MODELS {
                    let score_col = score_headers
                        .iter()
                        .position(|field| field == format!("score_{model}__max"))
                        .unwrap();
                    for endpoint in &endpoints {
                        let row = &score_by_endpoint[&endpoint[0]];
                        endpoint_scores
                            .write_record([
                                policy,
                                model,
                                branch,
                                cohort,
                                &endpoint[0],
                                &endpoint[1],
                                &row[score_col],
                            ])
                            .unwrap();
                    }
                }
            }
        }
    }
    endpoint_scores.flush().unwrap();

    let transfer_manifest = fixture
        .transfers
        .join("manifest.json")
        .canonicalize()
        .unwrap();
    let mut base_content = serde_json::Map::new();
    let mut base_files = serde_json::Map::new();
    for branch in BRANCHES {
        let path = fixture
            .fixed
            .join(branch)
            .join("bundle_manifest.json")
            .canonicalize()
            .unwrap();
        let value: Value = serde_json::from_reader(File::open(&path).unwrap()).unwrap();
        base_content.insert(branch.into(), value["bundle_content_hash"].clone());
        base_files.insert(
            path.display().to_string(),
            json!(sha256_file(&path).unwrap()),
        );
    }
    let generated = json!({
        "selected_parameters.csv": sha256_file(&parameter_path).unwrap(),
        "selected_endpoint_scores.csv": sha256_file(&endpoint_score_path).unwrap()
    });
    write_json(
        &fixture.selection.join("selection_manifest.json"),
        &json!({
            "analysis":"component-metric-specific self-gated Hill-q L2 parameter selection",
            "schema_version":4,
            "status":"post_hoc_model_development",
            "implementation":"rust_native",
            "source_root": fixture.transfers.canonicalize().unwrap(),
            "base_bundle_root": fixture.fixed.canonicalize().unwrap(),
            "transfer_manifest_sha256": sha256_file(&transfer_manifest).unwrap(),
            "base_bundle_content_hashes": base_content,
            "formula": {
                "score":"S solves S = m + C_q * sigmoid((c-S)/kappa)",
                "floor_definition":"ln(1e-12)",
                "empty_roster_score": (1e-12_f64).ln()
            },
            "selection":{
                "pr_selection_computational_design": {
                    "replications": 200,
                    "computational_order": 2
                }
            }, "grid":{}, "label_contracts":{}, "selected_parameters":[],
            "source_files": {transfer_manifest.display().to_string(): sha256_file(&transfer_manifest).unwrap()},
            "software_versions":{}, "executable":{},
            "base_bundle_files": base_files,
            "generated_files": generated
        }),
    );
}

#[test]
fn compact_rust_pipeline_reaches_seventy_system_bundles_and_plans_7245_matches() {
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
    let mismatch = build_fixed_bundles(
        &altered_config,
        &fixture.transfers,
        &fixture.fixed.with_file_name("wrong_fixed"),
    )
    .unwrap_err();
    assert!(mismatch.to_string().contains("configuration hash differs"));

    build_fixed_bundles(&fixture.pipeline_config, &fixture.transfers, &fixture.fixed).unwrap();
    for branch in BRANCHES {
        assert_eq!(
            load_bundle(&fixture.fixed.join(branch))
                .unwrap()
                .summary()
                .system_count,
            60
        );
    }
    build_selection_fixture(&fixture);
    let bad_selection = fixture.selection.with_file_name("bad_selection");
    fs::create_dir(&bad_selection).unwrap();
    for name in ["selected_parameters.csv", "selected_endpoint_scores.csv"] {
        fs::copy(fixture.selection.join(name), bad_selection.join(name)).unwrap();
    }
    let mut bad_manifest: Value = serde_json::from_reader(
        File::open(fixture.selection.join("selection_manifest.json")).unwrap(),
    )
    .unwrap();
    bad_manifest["base_bundle_content_hashes"]["pr"] = json!("wrong-content-hash");
    write_json(
        &bad_selection.join("selection_manifest.json"),
        &bad_manifest,
    );
    let bad_combined = fixture.combined.with_file_name("bad_combined");
    let mismatch =
        build_combined_bundles(&fixture.fixed, &bad_selection, &bad_combined).unwrap_err();
    assert!(
        mismatch
            .to_string()
            .contains("content hash does not resolve")
    );
    assert!(!bad_combined.exists());

    build_combined_bundles(&fixture.fixed, &fixture.selection, &fixture.combined).unwrap();
    for branch in BRANCHES {
        let bundle = load_bundle(&fixture.combined.join(branch)).unwrap();
        let summary = bundle.summary();
        assert_eq!(summary.system_count, 70);
        assert_eq!(summary.expected_match_count, 7_245);
        let plan = directed_round_robin_organizer::plan_with_shard_count(&bundle, 3).unwrap();
        assert_eq!(plan.matches.len(), 7_245);
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
