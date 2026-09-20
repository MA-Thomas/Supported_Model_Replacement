//! A closed scientific input package; no upstream directory is needed at runtime.
use crate::{PipelineConfig, io::*};
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, fs, path::Path};

pub fn export_inputs(config_path: &Path, output: &Path) -> Result<()> {
    let mut config = PipelineConfig::load(config_path)?;
    let stage = stage_path(output)?;
    let result = (|| {
        fs::create_dir(stage.join("inputs"))?;
        fs::copy(config_path, stage.join("source_config.json"))?;
        for input in config.inputs_mut() {
            let name = input
                .path
                .file_name()
                .context("input filename")?
                .to_string_lossy();
            let relative =
                std::path::PathBuf::from("inputs").join(format!("{}_{name}", input.sha256));
            let dest = stage.join(&relative);
            if !dest.exists() {
                fs::copy(&input.path, &dest)?;
            }
            ensure!(
                sha256_file(&dest)? == input.sha256,
                "input changed during export"
            );
            input.path = relative;
        }
        write_json_new(&stage.join("config.json"), &config)?;
        write_json_new(
            &stage.join("manifest.json"),
            &serde_json::json!({
                "schema_version": 1, "kind": "nci_portable_inputs",
                "source_config_sha256": sha256_file(config_path)?,
                "files": collect_file_hashes(&stage)?,
            }),
        )?;
        audit_inputs(&stage)?;
        fs::rename(&stage, output)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(stage);
    }
    result
}

pub fn audit_inputs(root: &Path) -> Result<()> {
    let root = root.canonicalize()?;
    let manifest: serde_json::Value = read_json(&root.join("manifest.json"))?;
    ensure!(
        manifest["schema_version"] == 1 && manifest["kind"] == "nci_portable_inputs",
        "unknown input package"
    );
    let expected: BTreeMap<String, String> = serde_json::from_value(manifest["files"].clone())?;
    ensure!(
        expected == collect_file_hashes(&root)?,
        "input package hashes differ"
    );
    ensure!(
        manifest["source_config_sha256"].as_str()
            == Some(sha256_file(&root.join("source_config.json"))?.as_str()),
        "source configuration hash mismatch"
    );
    let mut config = PipelineConfig::load(&root.join("config.json"))?;
    for input in config.inputs_mut() {
        ensure!(
            input.path.canonicalize()?.starts_with(&root),
            "input escapes portable package"
        );
    }
    Ok(())
}
