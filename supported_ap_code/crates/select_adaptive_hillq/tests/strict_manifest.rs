use std::process::Command;

#[test]
fn cli_rejects_a_source_root_without_a_rust_transfer_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let bundle = temp.path().join("bundle");
    let output = temp.path().join("output");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&bundle).unwrap();

    let result = Command::new(env!("CARGO_BIN_EXE_select_adaptive_hillq"))
        .args([
            "--source-root",
            source.to_str().unwrap(),
            "--bundle-root",
            bundle.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--alpha-values",
            "0",
        ])
        .output()
        .unwrap();

    assert!(!result.status.success());
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(stderr.contains("must be a Rust transfer package with manifest.json"));
    assert!(!output.exists());
}
