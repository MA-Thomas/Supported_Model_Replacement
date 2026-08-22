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
        &input.join("query.csv"),
        concat!(
            "peptide,HLA-RE,PatientID,TCGA_EXPR_TYPE,env_id,gene\n",
            "NMERPEPTI,HLA-A*01:01,P1,TYPE,0,G\n",
            "NMERPEPTI,HLA-A*01:01,P1,TYPE,0,G\n",
        ),
    );
    write(
        &input.join("env.csv"),
        "env_id,allele_environment\n0,\"A0101,B0702\"\n",
    );
    write(
        &input.join("mapping.csv"),
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
            "schema_version": 1,
            "datasets": [{
                "id": "TEST",
                "prefix": "test",
                "query": "query.csv",
                "env_dict": "env.csv",
                "mapping": "mapping.csv",
                "config_template": "template.toml",
                "generated_mono_config": "test_mono.toml",
                "generated_full_deduplicated_config": "test_full.toml",
                "response_column": "long_peptide_label",
                "response_kind": "binary",
                "endpoint_fields": ["patient_id", "long_peptide"],
                "expected": {
                    "unique_query_rows": 1,
                    "mono_environments": 1,
                    "mapping_rows": 2,
                    "scoreable_rows": 1,
                    "floor_rows": 1
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
    assert_eq!(summary["datasets"]["TEST"]["scoreable_rows"], 1);
    assert_eq!(summary["datasets"]["TEST"]["floor_rows"], 1);
    assert_eq!(validate_bundle(&fixture.output).unwrap()["status"], "pass");
    assert!(fixture.output.join("manifest.json").is_file());
    assert!(!fixture.output.join("mono_inputs_manifest.json").exists());
    assert!(!fixture.output.join("test_mono_inputs_audit.json").exists());

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
    assert_eq!(mono.rows[1]["mapping_status"], "floor");
    assert_eq!(mono.rows[1]["env_id"], "1");
    assert!(full.rows.iter().all(|row| row["env_id"] == "0"));
    assert_eq!(canonical.rows[1]["mono_env_id"], "1");

    let duplicate = build_bundle(&fixture.config, &fixture.input, &fixture.output).unwrap_err();
    assert!(duplicate.to_string().contains("refusing to overwrite"));
}

#[test]
fn fails_closed_on_noncontained_scoreable_candidate() {
    let fixture = fixture("ABCDEFGHIJKLM");
    let error = build_bundle(&fixture.config, &fixture.input, &fixture.output).unwrap_err();
    assert!(error.to_string().contains("does not contain it"));
    assert!(!fixture.output.exists());
}

#[test]
fn validator_detects_mutation_after_build() {
    let fixture = fixture("XXNMERPEPTIYY");
    build_bundle(&fixture.config, &fixture.input, &fixture.output).unwrap();
    let mapping = fixture.output.join("test_mono_longpep_mapping.csv");
    let mut contents = fs::read_to_string(&mapping).unwrap();
    contents = contents.replacen("floor", "unknown", 1);
    fs::write(mapping, contents).unwrap();
    let error = validate_bundle(&fixture.output).unwrap_err();
    assert!(error.to_string().contains("hash mismatch"));
}
