use std::fs;
use std::path::{Path, PathBuf};

use external_validation_inputs::io::{read_csv, write_json};
use external_validation_inputs::{build_bundle, validate_bundle};
use serde_json::json;
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    input: PathBuf,
    config: PathBuf,
    output: PathBuf,
}

fn write(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
}

fn fixture(long_peptide: &str) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    fs::create_dir(&input).unwrap();
    write(
        &input.join("env.csv"),
        "env_id,allele_environment\n0,\"A0101,A0101,B0702\"\n",
    );
    write(
        &input.join("mapping.csv"),
        &format!(
            "patient_id,env_id,long_peptide,nmer,long_peptide_label\nP1,0,{long_peptide},NMERPEPTI,1\n"
        ),
    );
    write(
        &input.join("secondary.csv"),
        &format!(
            "patient_id,env_id,long_peptide,nmer,long_peptide_label\nP1,0,{long_peptide},NMERPEPTI,1\n"
        ),
    );
    write(
        &input.join("template.toml"),
        "hla_env_dict = \"old-env.csv\"\nquery_peptide_input_tuples_file = \"old-query.csv\"\n",
    );
    let config = temp.path().join("config.json");
    write_json(
        &config,
        &json!({
            "schema_version": 2,
            "datasets": [{
                "id": "TEST",
                "prefix": "test",
                "expression": "TYPE",
                "env_dict": "env.csv",
                "primary_view_id": "test_full",
                "primary_mapping": "mapping.csv",
                "config_template": "template.toml",
                "generated_mono_config": "test_mono.toml",
                "generated_full_deduplicated_config": "test_full.toml",
                "response_column": "long_peptide_label",
                "response_kind": "binary",
                "endpoint_fields": ["patient_id", "long_peptide"],
                "views": [{
                    "id": "test_secondary",
                    "role": "secondary",
                    "mapping": "secondary.csv",
                    "require_subset_of": "test_full",
                    "selection_eligible": false,
                    "bundle_eligible": false,
                    "expected": {
                        "source_rows": 1,
                        "mapping_rows": 2,
                        "endpoints": 1,
                        "compute_keys": 2
                    }
                }],
                "expected": {
                    "full_query_rows": 2,
                    "mono_environments": 2,
                    "primary_source_rows": 1,
                    "primary_mapping_rows": 2,
                    "primary_endpoints": 1
                }
            }]
        }),
    )
    .unwrap();
    let output = temp.path().join("output");
    Fixture {
        _temp: temp,
        input,
        config,
        output,
    }
}

#[test]
fn builds_full_roster_and_both_representations() {
    let fixture = fixture("XXNMERPEPTIYY");
    let summary = build_bundle(&fixture.config, &fixture.input, &fixture.output).unwrap();
    assert_eq!(summary["datasets"]["TEST"]["candidate_rows"], 2);
    assert_eq!(summary["datasets"]["TEST"]["scoreable_rows"], 2);
    assert_eq!(summary["datasets"]["TEST"]["floor_rows"], 0);
    assert_eq!(validate_bundle(&fixture.output).unwrap()["status"], "pass");
    assert!(fixture.output.join("manifest.json").is_file());
    assert!(!fixture.output.join("mono_inputs_manifest.json").exists());
    assert!(!fixture.output.join("test_mono_inputs_audit.json").exists());
    assert!(fixture.output.join("test_full_hla_env_dict.csv").is_file());
    let full_config = fs::read_to_string(fixture.output.join("test_full.toml")).unwrap();
    assert!(full_config.contains("${EXTERNAL_VALIDATION_INPUT_ROOT}/test_full_hla_env_dict.csv"));

    let mono = read_csv(&fixture.output.join("test_mono_longpep_mapping.csv")).unwrap();
    let full = read_csv(&fixture.output.join("test_full_longpep_mapping.csv")).unwrap();
    let canonical = read_csv(&fixture.output.join("test_candidate_roster.csv")).unwrap();
    assert_eq!(mono.rows.len(), 2);
    assert_eq!(full.rows.len(), 2);
    assert_eq!(canonical.rows.len(), 2);

    assert_eq!(mono.rows[0]["HLA-RE"], "A0101");
    assert_eq!(mono.rows[0]["mapping_status"], "scoreable");
    assert_eq!(mono.rows[0]["env_id"], "0");
    assert_eq!(mono.rows[1]["HLA-RE"], "B0702");
    assert_eq!(mono.rows[1]["mapping_status"], "scoreable");
    assert_eq!(mono.rows[1]["env_id"], "1");
    assert!(full.rows.iter().all(|row| row["env_id"] == "0"));
    assert_eq!(canonical.rows[1]["mono_env_id"], "1");
    assert!(
        fixture
            .output
            .join("test_secondary_full_longpep_mapping.csv")
            .is_file()
    );

    let duplicate = build_bundle(&fixture.config, &fixture.input, &fixture.output).unwrap_err();
    assert!(duplicate.to_string().contains("refusing to overwrite"));
}

#[test]
fn fails_closed_on_noncontained_scoreable_candidate() {
    let fixture = fixture("ABCDEFGHIJKLM");
    let error = build_bundle(&fixture.config, &fixture.input, &fixture.output).unwrap_err();
    assert!(error.to_string().contains("is not contained"));
    assert!(!fixture.output.exists());
}

#[test]
fn validator_detects_mutation_after_build() {
    let fixture = fixture("XXNMERPEPTIYY");
    build_bundle(&fixture.config, &fixture.input, &fixture.output).unwrap();
    let mapping = fixture.output.join("test_mono_longpep_mapping.csv");
    let mut contents = fs::read_to_string(&mapping).unwrap();
    contents = contents.replacen("scoreable", "floor", 1);
    fs::write(mapping, contents).unwrap();
    let error = validate_bundle(&fixture.output).unwrap_err();
    assert!(error.to_string().contains("hash mismatch"));
}

#[test]
fn shared_compute_is_run_once_and_crosswalked_to_both_endpoints() {
    let fixture = fixture("XXNMERPEPTIYY");
    write(
        &fixture.input.join("mapping.csv"),
        concat!(
            "patient_id,env_id,long_peptide,nmer,long_peptide_label\n",
            "P1,0,XXNMERPEPTIYY,NMERPEPTI,1\n",
            "P1,0,ZZNMERPEPTIXX,NMERPEPTI,0\n",
        ),
    );
    let mut config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&fixture.config).unwrap()).unwrap();
    config["datasets"][0]["expected"]["primary_source_rows"] = json!(2);
    config["datasets"][0]["expected"]["primary_mapping_rows"] = json!(4);
    config["datasets"][0]["expected"]["primary_endpoints"] = json!(2);
    write_json(&fixture.config, &config).unwrap();

    build_bundle(&fixture.config, &fixture.input, &fixture.output).unwrap();
    let query = read_csv(&fixture.output.join("test_full_deduplicated_query.csv")).unwrap();
    let mapping = read_csv(&fixture.output.join("test_full_longpep_mapping.csv")).unwrap();
    assert_eq!(query.rows.len(), 2);
    assert_eq!(mapping.rows.len(), 4);
    for query_row in &query.rows {
        let id = &query_row["compute_key_id"];
        assert_eq!(
            mapping
                .rows
                .iter()
                .filter(|mapping_row| &mapping_row["compute_key_id"] == id)
                .count(),
            2
        );
    }
}

#[test]
fn secondary_view_must_be_inside_primary_compute_universe() {
    let fixture = fixture("XXNMERPEPTIYY");
    write(
        &fixture.input.join("secondary.csv"),
        "patient_id,env_id,long_peptide,nmer,long_peptide_label\nP1,0,XXABCDEFGHIYY,ABCDEFGHI,1\n",
    );
    let error = build_bundle(&fixture.config, &fixture.input, &fixture.output).unwrap_err();
    assert!(error.to_string().contains("outside primary view"));
    assert!(!fixture.output.exists());
}
