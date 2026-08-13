use std::fs;
use std::process::Command;

use serde_json::Value;
use tempfile::tempdir;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_supported_ap_metrics"))
}

#[test]
fn cli_writes_valid_schema_eight_json_and_tracks_skip_reasons() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("predictions.csv");
    let output = directory.path().join("report.json");
    fs::write(
        &input,
        "label,score_a,score_b\n\
         1,0.9,0.8\n\
         0,0.7,\n\
         1,0.6,0.9\n\
         0,0.2,0.1\n",
    )
    .unwrap();
    fs::write(&output, "old report").unwrap();

    let result = binary()
        .args([
            "--input",
            input.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--label-col",
            "label",
            "--score-col",
            "score_a",
            "--score-col",
            "score_b",
            "--reference-prevalence",
            "0.5",
            "--bootstrap-replicates",
            "4",
            "--permutations",
            "2",
        ])
        .output()
        .unwrap();

    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(report["schema_version"], 8);
    assert_eq!(report["rows"]["read"], 4);
    assert_eq!(report["rows"]["skipped_any_score"], 1);
    assert_eq!(report["rows"]["retained_shared"], 3);

    let metrics = report["metrics"].as_array().unwrap();
    assert_eq!(metrics.len(), 2);
    let score_a = metrics
        .iter()
        .find(|metric| metric["score_col"] == "score_a")
        .unwrap();
    let score_b = metrics
        .iter()
        .find(|metric| metric["score_col"] == "score_b")
        .unwrap();
    assert_eq!(score_a["n_skipped_score"], 0);
    assert_eq!(score_b["n_skipped_score"], 1);
    assert_eq!(score_a["n"], 3);
    assert_eq!(score_b["n"], 3);
    assert_eq!(score_a["status"], "ok");
    assert_eq!(score_b["status"], "ok");
}

#[test]
fn fail_on_error_returns_failure_after_writing_the_report() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("one_class.csv");
    let output = directory.path().join("report.json");
    fs::write(&input, "label,score_a\n1,0.9\n1,0.8\n").unwrap();

    let result = binary()
        .args([
            "--input",
            input.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--label-col",
            "label",
            "--score-col",
            "score_a",
            "--reference-prevalence",
            "0.5",
            "--bootstrap-replicates",
            "2",
            "--permutations",
            "2",
            "--fail-on-error",
        ])
        .output()
        .unwrap();

    assert!(!result.status.success());
    let report: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(report["failures"], 1);
    assert_eq!(report["metrics"][0]["status"], "error");
    assert!(
        report["metrics"][0]["error"]
            .as_str()
            .unwrap()
            .contains("negative")
    );
}

#[test]
fn paired_cli_reports_declared_prospective_replication_counts() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("predictions.csv");
    let output = directory.path().join("report.json");
    fs::write(
        &input,
        "label,score_a,score_b\n\
         1,0.9,0.8\n\
         0,0.7,0.6\n\
         1,0.6,0.9\n\
         0,0.2,0.1\n",
    )
    .unwrap();

    let result = binary()
        .args([
            "--input",
            input.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--label-col",
            "label",
            "--score-col",
            "score_a",
            "--score-col",
            "score_b",
            "--reference-prevalence",
            "0.5",
            "--paired-baseline",
            "score_b",
            "--replication-positive-count",
            "10",
            "--replication-negative-count",
            "30",
            "--bootstrap-replicates",
            "4",
            "--permutations",
            "2",
        ])
        .output()
        .unwrap();

    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(report["paired_replication_positive_count"], 10);
    assert_eq!(report["paired_replication_negative_count"], 30);
    assert_eq!(report["paired"][0]["observed_positive_count"], 2);
    assert_eq!(report["paired"][0]["observed_negative_count"], 2);
    assert_eq!(report["paired"][0]["replication_positive_count"], 10);
    assert_eq!(report["paired"][0]["replication_negative_count"], 30);
}

#[test]
fn label_column_cannot_also_be_selected_as_a_score() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("predictions.csv");
    let output = directory.path().join("report.json");
    fs::write(&input, "score_label,score_a\n1,0.9\n0,0.1\n").unwrap();

    let result = binary()
        .args([
            "--input",
            input.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--label-col",
            "score_label",
            "--reference-prevalence",
            "0.5",
        ])
        .output()
        .unwrap();

    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("cannot also be selected"));
    assert!(!output.exists());
}
