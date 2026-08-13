use std::fs;
use std::process::Command;

use serde_json::{Value, json};
use tempfile::tempdir;

fn common_policy() -> [&'static str; 6] {
    [
        "--magnitude-threshold",
        "0",
        "--survival-floor",
        "0",
        "--survival-requirement",
        "0.5",
    ]
}

#[test]
fn ap_projected_cli_reports_v17_stages_anchors_and_trace() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("paired.csv");
    let output = directory.path().join("result.json");
    fs::write(
        &input,
        "label,candidate,incumbent\n1,0.9,0.8\n0,0.8,0.9\n1,0.4,0.2\n0,0.1,0.4\n",
    )
    .unwrap();
    let mut args = vec![
        "ap",
        "projected",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--label-col",
        "label",
        "--model-a-col",
        "candidate",
        "--model-b-col",
        "incumbent",
        "--prevalences",
        "0.1,0.4",
        "--computational-replications",
        "12",
        "--computational-order",
        "2",
        "--resampling-seed",
        "7",
        "--resampling-unit",
        "independent-observation",
        "--transport-justification",
        "declared prior shift",
        "--reference-limitation",
        "observational risks are heterogeneous",
        "--retain-replication-profiles",
        "--execution",
        "sequential",
    ];
    args.extend(common_policy());
    let result = Command::new(env!("CARGO_BIN_EXE_supported_ap"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(report["schema_version"], 17);
    assert_eq!(report["manuscript_version"], "v17");
    assert_eq!(report["analysis"], "ap_staged_projected_finite_evidence");
    assert_eq!(report["result"]["evidence"], "projected");
    assert_eq!(
        report["result"]["forward"]["observed_gate"]["literal_survival"]["list_length"],
        1
    );
    assert_eq!(
        report["result"]["trace"]["replication_difference_profiles"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
}

#[test]
fn ap_observed_cli_groups_full_evaluations() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("observed.csv");
    let output = directory.path().join("result.json");
    fs::write(
        &input,
        "evaluation,label,candidate,incumbent\na,1,0.9,0.6\na,0,0.2,0.7\na,1,0.8,0.5\na,0,0.1,0.1\nb,1,0.95,0.6\nb,0,0.3,0.8\nb,1,0.7,0.4\nb,0,0.2,0.2\n",
    )
    .unwrap();
    let mut args = vec![
        "ap",
        "observed",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--evaluation-id-col",
        "evaluation",
        "--label-col",
        "label",
        "--model-a-col",
        "candidate",
        "--model-b-col",
        "incumbent",
        "--prevalences",
        "0.1,0.4",
        "--empirical-order",
        "2",
        "--transport-justification",
        "declared prior shift",
        "--reference-limitation",
        "observational evaluations",
        "--execution",
        "sequential",
    ];
    args.extend(common_policy());
    let result = Command::new(env!("CARGO_BIN_EXE_supported_ap"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(report["analysis"], "ap_observed_empirical_gate");
    assert_eq!(report["evaluation_ids"], json!(["a", "b"]));
    assert_eq!(report["result"]["evidence"], "observed");
    assert_eq!(report["result"]["evaluation_count"], 2);
}

#[test]
fn auroc_projected_cli_has_no_inference_or_confidence_surface() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("paired.csv");
    let output = directory.path().join("result.json");
    fs::write(
        &input,
        "label,candidate,incumbent\n1,0.9,0.6\n1,0.8,0.5\n1,0.7,0.4\n0,0.3,0.8\n0,0.2,0.2\n0,0.1,0.1\n",
    )
    .unwrap();
    let mut args = vec![
        "auroc",
        "projected",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--label-col",
        "label",
        "--model-a-col",
        "candidate",
        "--model-b-col",
        "incumbent",
        "--computational-order",
        "2",
        "--computational-replications",
        "6",
        "--resampling-seed",
        "7",
        "--resampling-unit",
        "independent-observation",
        "--reference-limitation",
        "observational risks are heterogeneous",
        "--execution",
        "sequential",
        "--concentration-tolerance",
        "0.1",
    ];
    args.extend(common_policy());
    let result = Command::new(env!("CARGO_BIN_EXE_supported_ap"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(report["analysis"], "auroc_staged_projected_breakdown");
    assert!(report.get("inference").is_none());
    assert!(report.get("confidence_level").is_none());
}

#[test]
fn cli_rejects_removed_outer_inference_options() {
    let result = Command::new(env!("CARGO_BIN_EXE_supported_ap"))
        .args(["ap", "projected", "--outer-replicates", "10"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("unexpected argument"));
}
