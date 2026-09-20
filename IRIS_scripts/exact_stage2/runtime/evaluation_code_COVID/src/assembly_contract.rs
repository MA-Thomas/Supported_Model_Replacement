use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use csv::ReaderBuilder;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ManifestInput {
    role: String,
    path: PathBuf,
    kind: String,
    size: u64,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct RunManifest {
    dataset: String,
    run_id: String,
    hla_environment_representation: String,
    query_input_file: PathBuf,
    common_fingerprint: String,
    inputs: Vec<ManifestInput>,
}

#[derive(Debug, Deserialize)]
struct ModeManifest {
    dataset: String,
    run_id: String,
    mode: String,
    common_fingerprint: String,
    mode_fingerprint: String,
    query_input_file: PathBuf,
    total_params: usize,
    param_chunks: usize,
    pn_hla_scope: Option<String>,
    max_num_ps_values_log2: Option<u32>,
    max_num_ps_values: Option<u64>,
    inputs: Vec<ManifestInput>,
}

#[derive(Debug, Deserialize)]
struct TaskOutput {
    path: PathBuf,
    size: u64,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct TaskManifest {
    status: String,
    run_id: String,
    fingerprint: String,
    mode: String,
    output_files: Vec<TaskOutput>,
}

#[derive(Debug)]
pub struct ExpectedGrid {
    pub parameter_rows: usize,
    pub geometry_params: Vec<(f32, f32, f32, f32)>,
    pub tau_values: Vec<f32>,
    pub mn_tuples: Vec<(u32, u32)>,
}

#[derive(Debug)]
pub struct AssemblyProvenance {
    pub dataset: String,
    pub run_id: String,
    pub hla_environment_representation: String,
    pub pn_hla_scope: String,
    pub max_num_ps_values_log2: u32,
    pub max_num_ps_values: u64,
    pub common_fingerprint: String,
    pub qpi_mode_fingerprint: String,
    pub pn_mode_fingerprint: String,
    pub query_input_file: PathBuf,
    pub query_input_sha256: String,
    pub mapping_file: PathBuf,
    pub mapping_sha256: String,
    pub parameter_file: PathBuf,
    pub parameter_file_sha256: String,
    pub mn_tuples_file: PathBuf,
    pub mn_tuples_file_sha256: String,
    pub run_manifest: PathBuf,
    pub qpi_manifest: PathBuf,
    pub pn_manifest: PathBuf,
    pub expected_grid: ExpectedGrid,
    pub sparse_grid: crate::sparse_grid::SparseGrid,
    pub survivor_grid_file: PathBuf,
    pub survivor_grid_sha256: String,
    results_dir: PathBuf,
}

fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let file = fs::File::open(path)
        .with_context(|| format!("Failed to open Stage 2 manifest: {}", path.display()))?;
    serde_json::from_reader(file)
        .with_context(|| format!("Failed to parse Stage 2 manifest: {}", path.display()))
}

fn canonical_file(path: &Path, description: &str) -> Result<PathBuf> {
    let resolved = path
        .canonicalize()
        .with_context(|| format!("{description} is not readable: {}", path.display()))?;
    if !resolved.is_file() {
        bail!("{description} is not a file: {}", resolved.display());
    }
    Ok(resolved)
}

fn canonical_dir(path: &Path, description: &str) -> Result<PathBuf> {
    let resolved = path
        .canonicalize()
        .with_context(|| format!("{description} is not readable: {}", path.display()))?;
    if !resolved.is_dir() {
        bail!("{description} is not a directory: {}", resolved.display());
    }
    Ok(resolved)
}

fn input_for_role<'a>(inputs: &'a [ManifestInput], role: &str) -> Result<&'a ManifestInput> {
    let matches: Vec<_> = inputs.iter().filter(|item| item.role == role).collect();
    if matches.len() != 1 {
        bail!(
            "Expected exactly one manifest input with role '{role}', found {}",
            matches.len()
        );
    }
    Ok(matches[0])
}

pub(crate) fn sha256_file(path: &Path) -> Result<String> {
    for (program, args) in [
        ("sha256sum", Vec::<&str>::new()),
        ("shasum", vec!["-a", "256"]),
    ] {
        let output = Command::new(program).args(args).arg(path).output();
        let Ok(output) = output else { continue };
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8(output.stdout)
            .with_context(|| format!("{program} returned non-UTF-8 output"))?;
        if let Some(value) = stdout.split_whitespace().next() {
            if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Ok(value.to_ascii_lowercase());
            }
        }
    }
    bail!(
        "Unable to compute SHA-256 for {}; install sha256sum or shasum",
        path.display()
    )
}

fn verify_manifest_file(input: &ManifestInput) -> Result<(PathBuf, String)> {
    if input.kind != "file" {
        bail!(
            "Manifest input role '{}' must be a file, got '{}'",
            input.role,
            input.kind
        );
    }
    let path = canonical_file(&input.path, &format!("manifest input '{}'", input.role))?;
    let actual_size = fs::metadata(&path)?.len();
    if actual_size != input.size {
        bail!(
            "Size mismatch for manifest input '{}': expected {}, got {} ({})",
            input.role,
            input.size,
            actual_size,
            path.display()
        );
    }
    let actual_hash = sha256_file(&path)?;
    if actual_hash != input.sha256.to_ascii_lowercase() {
        bail!(
            "SHA-256 mismatch for manifest input '{}': {}",
            input.role,
            path.display()
        );
    }
    Ok((path, actual_hash))
}

fn required_column(headers: &csv::StringRecord, name: &str) -> Result<usize> {
    headers
        .iter()
        .position(|header| header.trim() == name)
        .with_context(|| format!("Required column '{name}' is absent"))
}

fn parse_f32(raw: &str, field: &str, row: usize, path: &Path) -> Result<f32> {
    let value = raw.trim().parse::<f32>().with_context(|| {
        format!(
            "Invalid {field} at row {row} in {}: '{raw}'",
            path.display()
        )
    })?;
    if !value.is_finite() {
        bail!("Non-finite {field} at row {row} in {}", path.display());
    }
    Ok(value)
}

fn read_expected_grid(parameter_file: &Path, mn_file: &Path) -> Result<ExpectedGrid> {
    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_path(parameter_file)
        .with_context(|| {
            format!(
                "Failed to open parameter file: {}",
                parameter_file.display()
            )
        })?;
    let headers = reader.headers()?.clone();
    let d_pos = required_column(&headers, "d_pos")?;
    let d_neg = required_column(&headers, "d_neg")?;
    let steepness_pos = required_column(&headers, "steepness_pos")?;
    let steepness_neg = required_column(&headers, "steepness_neg")?;
    let tau_thymus = required_column(&headers, "tau_thymus")?;
    let mut geometries = HashSet::new();
    let mut taus = HashSet::new();
    let mut parameter_rows = 0usize;
    for (index, record) in reader.records().enumerate() {
        let record = record?;
        let row = index + 2;
        let geometry = (
            parse_f32(
                record.get(d_pos).unwrap_or(""),
                "d_pos",
                row,
                parameter_file,
            )?,
            parse_f32(
                record.get(d_neg).unwrap_or(""),
                "d_neg",
                row,
                parameter_file,
            )?,
            parse_f32(
                record.get(steepness_pos).unwrap_or(""),
                "steepness_pos",
                row,
                parameter_file,
            )?,
            parse_f32(
                record.get(steepness_neg).unwrap_or(""),
                "steepness_neg",
                row,
                parameter_file,
            )?,
        );
        let tau = parse_f32(
            record.get(tau_thymus).unwrap_or(""),
            "tau_thymus",
            row,
            parameter_file,
        )?;
        geometries.insert((
            geometry.0.to_bits(),
            geometry.1.to_bits(),
            geometry.2.to_bits(),
            geometry.3.to_bits(),
        ));
        taus.insert(tau.to_bits());
        parameter_rows += 1;
    }
    if parameter_rows == 0 {
        bail!("Parameter file is empty: {}", parameter_file.display());
    }

    let mut geometry_params: Vec<_> = geometries
        .into_iter()
        .map(|(a, b, c, d)| {
            (
                f32::from_bits(a),
                f32::from_bits(b),
                f32::from_bits(c),
                f32::from_bits(d),
            )
        })
        .collect();
    geometry_params.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.1.total_cmp(&b.1))
            .then(a.2.total_cmp(&b.2))
            .then(a.3.total_cmp(&b.3))
    });
    let mut tau_values: Vec<_> = taus.into_iter().map(f32::from_bits).collect();
    tau_values.sort_by(f32::total_cmp);
    if geometry_params.len() * tau_values.len() != parameter_rows {
        bail!(
            "Parameter file is not a complete geometry × tau grid: {} rows, {} geometries, {} tau values",
            parameter_rows,
            geometry_params.len(),
            tau_values.len()
        );
    }

    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_path(mn_file)
        .with_context(|| format!("Failed to open M/N file: {}", mn_file.display()))?;
    let headers = reader.headers()?.clone();
    let m = required_column(&headers, "at_least_M")?;
    let n = required_column(&headers, "at_most_N")?;
    let mut mn = HashSet::new();
    for (index, record) in reader.records().enumerate() {
        let record = record?;
        let row = index + 2;
        let m_value = record
            .get(m)
            .unwrap_or("")
            .trim()
            .parse::<u32>()
            .with_context(|| format!("Invalid at_least_M at row {row} in {}", mn_file.display()))?;
        let n_value = record
            .get(n)
            .unwrap_or("")
            .trim()
            .parse::<u32>()
            .with_context(|| format!("Invalid at_most_N at row {row} in {}", mn_file.display()))?;
        mn.insert((m_value, n_value));
    }
    let mut mn_tuples: Vec<_> = mn.into_iter().collect();
    mn_tuples.sort_unstable();
    if mn_tuples.is_empty() {
        bail!("M/N file is empty: {}", mn_file.display());
    }

    Ok(ExpectedGrid {
        parameter_rows,
        geometry_params,
        tau_values,
        mn_tuples,
    })
}

fn same_path(left: &Path, right: &Path) -> Result<bool> {
    Ok(canonical_file(left, "path")? == canonical_file(right, "path")?)
}

#[allow(clippy::too_many_arguments)]
pub fn validate_assembly_contract(
    dataset: &str,
    run_id: &str,
    results_dir: &Path,
    query_file: &Path,
    mapping_file: &Path,
    run_manifest_path: &Path,
    qpi_manifest_path: &Path,
    pn_manifest_path: &Path,
) -> Result<AssemblyProvenance> {
    let results_dir = canonical_dir(results_dir, "Stage 2 results directory")?;
    let run_manifest_path = canonical_file(run_manifest_path, "run manifest")?;
    let qpi_manifest_path = canonical_file(qpi_manifest_path, "Q/Pi manifest")?;
    let pn_manifest_path = canonical_file(pn_manifest_path, "P/N manifest")?;
    for manifest in [&run_manifest_path, &qpi_manifest_path, &pn_manifest_path] {
        if manifest.parent() != Some(results_dir.as_path()) {
            bail!(
                "Manifest {} is not directly under results directory {}",
                manifest.display(),
                results_dir.display()
            );
        }
    }

    let common: RunManifest = load_json(&run_manifest_path)?;
    let qpi: ModeManifest = load_json(&qpi_manifest_path)?;
    let pn: ModeManifest = load_json(&pn_manifest_path)?;
    if common.dataset != dataset || qpi.dataset != dataset || pn.dataset != dataset {
        bail!(
            "Dataset mismatch: CLI={dataset}, run={}, qpi={}, pn={}",
            common.dataset,
            qpi.dataset,
            pn.dataset
        );
    }
    if common.run_id != run_id || qpi.run_id != run_id || pn.run_id != run_id {
        bail!(
            "Run-ID mismatch: CLI={run_id}, run={}, qpi={}, pn={}",
            common.run_id,
            qpi.run_id,
            pn.run_id
        );
    }
    if qpi.mode != "qpi" || pn.mode != "pn" {
        bail!(
            "Mode manifests must be qpi and pn, got '{}' and '{}'",
            qpi.mode,
            pn.mode
        );
    }
    if qpi.common_fingerprint != common.common_fingerprint
        || pn.common_fingerprint != common.common_fingerprint
    {
        bail!("Q/Pi or P/N manifest common fingerprint does not match run_manifest.json");
    }
    if qpi.total_params != pn.total_params || qpi.param_chunks != pn.param_chunks {
        bail!("Q/Pi and P/N manifests disagree on parameter geometry");
    }
    if common.hla_environment_representation != "full"
        && common.hla_environment_representation != "mono"
    {
        bail!(
            "Unsupported HLA environment representation '{}'",
            common.hla_environment_representation
        );
    }
    let pn_hla_scope = pn
        .pn_hla_scope
        .clone()
        .context("P/N manifest is missing pn_hla_scope")?;
    if pn_hla_scope != "all" && pn_hla_scope != "focal" {
        bail!("Unsupported P/N HLA scope '{pn_hla_scope}'");
    }
    if common.hla_environment_representation == "mono" && pn_hla_scope != "all" {
        bail!("Mono representation must use P/N HLA scope 'all'");
    }
    let max_num_ps_values_log2 = pn
        .max_num_ps_values_log2
        .context("P/N manifest is missing max_num_ps_values_log2")?;
    let max_num_ps_values = pn
        .max_num_ps_values
        .context("P/N manifest is missing max_num_ps_values")?;
    let expected_top_k = 1u64
        .checked_shl(max_num_ps_values_log2)
        .context("max_num_ps_values_log2 is too large")?;
    if max_num_ps_values != expected_top_k {
        bail!(
            "P/N top-k provenance is inconsistent: log2={} but max_num_ps_values={}",
            max_num_ps_values_log2,
            max_num_ps_values
        );
    }

    let query_file = canonical_file(query_file, "query roster")?;
    if !same_path(&query_file, &common.query_input_file)?
        || !same_path(&query_file, &qpi.query_input_file)?
        || !same_path(&query_file, &pn.query_input_file)?
    {
        bail!("CLI query roster disagrees with one or more Stage 2 manifests");
    }
    let query_input = input_for_role(&common.inputs, "query_peptides")?;
    let (manifest_query, query_input_sha256) = verify_manifest_file(query_input)?;
    if manifest_query != query_file {
        bail!("query_peptides input entry disagrees with query_input_file");
    }

    let parameter_input = input_for_role(&pn.inputs, "parameter_sets")?;
    let mn_input = input_for_role(&pn.inputs, "mn_tuples")?;
    let (parameter_file, parameter_file_sha256) = verify_manifest_file(parameter_input)?;
    let (mn_tuples_file, mn_tuples_file_sha256) = verify_manifest_file(mn_input)?;
    let expected_grid = read_expected_grid(&parameter_file, &mn_tuples_file)?;
    if expected_grid.parameter_rows != pn.total_params {
        bail!(
            "P/N manifest declares {} parameters but its parameter file contains {} rows",
            pn.total_params,
            expected_grid.parameter_rows
        );
    }

    let grid_input = input_for_role(&pn.inputs, "survivor_grid")?;
    let (survivor_grid_file, survivor_grid_sha256) = verify_manifest_file(grid_input)?;
    for inputs in [&common.inputs, &qpi.inputs] {
        let (other, hash) = verify_manifest_file(input_for_role(inputs, "survivor_grid")?)?;
        if other != survivor_grid_file || hash != survivor_grid_sha256 { bail!("run modes disagree on survivor grid"); }
    }
    let sparse_grid: crate::sparse_grid::SparseGrid = load_json(&survivor_grid_file)?;
    sparse_grid.validate(&expected_grid, &common.hla_environment_representation, &pn_hla_scope)?;
    let grid_root = survivor_grid_file.parent().context("grid has no parent")?;
    if crate::sparse_grid::verify_file(grid_root, &sparse_grid.parameter_file)? != parameter_file
        || crate::sparse_grid::verify_file(grid_root, &sparse_grid.mn_union_file)? != mn_tuples_file {
        bail!("grid CSV paths disagree with mode inputs");
    }
    for g in &sparse_grid.geometries {
        let mn_path = crate::sparse_grid::verify_file(grid_root, &g.mn_file)?;
        let local_grid = read_expected_grid(&parameter_file, &mn_path)?;
        let mut pairs = g.mn_pairs.clone(); pairs.sort_unstable();
        if pairs != local_grid.mn_tuples { bail!("per-geometry MN CSV disagrees with sparse grid"); }
    }

    let mapping_file = canonical_file(mapping_file, "assembly mapping")?;
    let mapping_sha256 = sha256_file(&mapping_file)?;

    Ok(AssemblyProvenance {
        dataset: dataset.to_string(),
        run_id: run_id.to_string(),
        hla_environment_representation: common.hla_environment_representation,
        pn_hla_scope,
        max_num_ps_values_log2,
        max_num_ps_values,
        common_fingerprint: common.common_fingerprint,
        qpi_mode_fingerprint: qpi.mode_fingerprint,
        pn_mode_fingerprint: pn.mode_fingerprint,
        query_input_file: query_file,
        query_input_sha256,
        mapping_file,
        mapping_sha256,
        parameter_file,
        parameter_file_sha256,
        mn_tuples_file,
        mn_tuples_file_sha256,
        run_manifest: run_manifest_path,
        qpi_manifest: qpi_manifest_path,
        pn_manifest: pn_manifest_path,
        expected_grid,
        sparse_grid,
        survivor_grid_file,
        survivor_grid_sha256,
        results_dir,
    })
}

fn recorded_outputs(
    provenance: &AssemblyProvenance,
    directory_prefix: &str,
    mode: &str,
    fingerprint: &str,
) -> Result<HashSet<PathBuf>> {
    let mut outputs = HashSet::new();
    let mut task_manifests = 0usize;
    for entry in fs::read_dir(&provenance.results_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir()
            || !entry
                .file_name()
                .to_string_lossy()
                .starts_with(directory_prefix)
        {
            continue;
        }
        for child in fs::read_dir(entry.path())? {
            let child = child?;
            let name = child.file_name();
            let name = name.to_string_lossy();
            if !child.file_type()?.is_file()
                || !name.starts_with("TASK_")
                || !name.ends_with(".done.json")
            {
                continue;
            }
            let task: TaskManifest = load_json(&child.path())?;
            task_manifests += 1;
            if task.run_id != provenance.run_id
                || task.mode != mode
                || task.fingerprint != fingerprint
                || (task.status != "success" && task.status != "zero_input")
            {
                bail!(
                    "Task completion manifest disagrees with the {mode} contract: {}",
                    child.path().display()
                );
            }
            for output in task.output_files {
                let path = canonical_file(
                    &provenance.results_dir.join(output.path),
                    "recorded Stage 2 output",
                )?;
                if !path.starts_with(&provenance.results_dir) {
                    bail!(
                        "Task manifest output escapes the Stage 2 run root: {}",
                        path.display()
                    );
                }
                if fs::metadata(&path)?.len() != output.size || sha256_file(&path)? != output.sha256 {
                    bail!("Task output hash/size mismatch: {}", path.display());
                }
                if !outputs.insert(path.clone()) {
                    bail!(
                        "Stage 2 output is recorded by more than one task manifest: {}",
                        path.display()
                    );
                }
            }
        }
    }
    if task_manifests == 0 {
        bail!("No {mode} task completion manifests were found");
    }
    Ok(outputs)
}

pub fn validate_discovered_outputs(
    provenance: &AssemblyProvenance,
    pn_files: &[PathBuf],
    qpi_files: &[PathBuf],
) -> Result<()> {
    let canonical_set = |paths: &[PathBuf], description: &str| -> Result<HashSet<PathBuf>> {
        paths
            .iter()
            .map(|path| canonical_file(path, description))
            .collect()
    };
    let recorded_pn = recorded_outputs(
        provenance,
        "results_pn_env_id_",
        "pn",
        &provenance.pn_mode_fingerprint,
    )?;
    let recorded_qpi = recorded_outputs(
        provenance,
        "results_qpi_env_id_",
        "qpi",
        &provenance.qpi_mode_fingerprint,
    )?;
    let discovered_pn = canonical_set(pn_files, "discovered P/N CSV")?;
    let discovered_qpi = canonical_set(qpi_files, "discovered Q/Pi CSV")?;
    if discovered_pn != recorded_pn {
        bail!(
            "Discovered P/N CSVs do not exactly match task completion manifests (discovered {}, recorded {})",
            discovered_pn.len(),
            recorded_pn.len()
        );
    }
    if discovered_qpi != recorded_qpi {
        bail!(
            "Discovered Q/Pi CSVs do not exactly match task completion manifests (discovered {}, recorded {})",
            discovered_qpi.len(),
            recorded_qpi.len()
        );
    }
    Ok(())
}

pub fn validate_discovered_grid(
    expected: &ExpectedGrid,
    geometry_params: &[(f32, f32, f32, f32)],
    tau_values: &[f32],
    mn_tuples: &[(u32, u32)],
    n_params: usize,
) -> Result<()> {
    let geometry_bits = |values: &[(f32, f32, f32, f32)]| {
        let mut result: Vec<_> = values
            .iter()
            .map(|value| {
                (
                    value.0.to_bits(),
                    value.1.to_bits(),
                    value.2.to_bits(),
                    value.3.to_bits(),
                )
            })
            .collect();
        result.sort_unstable();
        result
    };
    let float_bits = |values: &[f32]| {
        let mut result: Vec<_> = values.iter().map(|value| value.to_bits()).collect();
        result.sort_unstable();
        result
    };
    let mut actual_mn = mn_tuples.to_vec();
    actual_mn.sort_unstable();
    if n_params != expected.parameter_rows
        || geometry_bits(geometry_params) != geometry_bits(&expected.geometry_params)
        || float_bits(tau_values) != float_bits(&expected.tau_values)
        || actual_mn != expected.mn_tuples
    {
        bail!(
            "Discovered PN grid disagrees with the fingerprinted Stage 2 inputs: expected {} parameters, {} geometries, {} tau values, M/N {:?}; discovered {} parameters, {} geometries, {} tau values, M/N {:?}",
            expected.parameter_rows,
            expected.geometry_params.len(),
            expected.tau_values.len(),
            expected.mn_tuples,
            n_params,
            geometry_params.len(),
            tau_values.len(),
            actual_mn
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn write_json(path: &Path, value: serde_json::Value) {
        fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }

    #[test]
    fn discovered_grid_must_match_fingerprinted_inputs() {
        let expected = ExpectedGrid {
            parameter_rows: 2,
            geometry_params: vec![(15.0, 9.0, 0.5, 0.75)],
            tau_values: vec![1000.0, 50052.0],
            mn_tuples: vec![(1, 1)],
        };
        validate_discovered_grid(
            &expected,
            &[(15.0, 9.0, 0.5, 0.75)],
            &[1000.0, 50052.0],
            &[(1, 1)],
            2,
        )
        .unwrap();
        assert!(validate_discovered_grid(
            &expected,
            &[(15.0, 9.0, 0.5, 0.75)],
            &[1000.0],
            &[(1, 1)],
            1,
        )
        .is_err());
    }

    #[test]
    fn complete_manifest_contract_is_accepted() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "assembly_contract_{}_{}",
            std::process::id(),
            nonce
        ));
        fs::create_dir_all(&root).unwrap();
        let query = root.join("query.csv");
        let mapping = root.join("mapping.csv");
        let parameters = root.join("parameters.csv");
        let mn = root.join("mn.csv");
        fs::write(
            &query,
            "peptide,HLA-RE,PatientID,env_id\nPEPTIDE,A0101,p1,0\n",
        )
        .unwrap();
        fs::write(
            &mapping,
            "patient_id,env_id,long_peptide,nmer,gene\np1,0,LONGPEPTIDE,PEPTIDE,G\n",
        )
        .unwrap();
        fs::write(
            &parameters,
            "d_pos,d_neg,steepness_pos,steepness_neg,tau_thymus\n15,9,0.5,0.75,1000\n15,9,0.5,0.75,50052\n15,9,0.5,0.75,91691\n15,9,0.5,0.75,120604\n15,9,0.5,0.75,156295\n15,9,0.5,0.75,200000\n",
        )
        .unwrap();
        fs::write(&mn, "at_least_M,at_most_N\n1,1\n").unwrap();
        let input = |role: &str, path: &Path| {
            serde_json::json!({
                "role": role,
                "path": path,
                "kind": "file",
                "size": fs::metadata(path).unwrap().len(),
                "sha256": sha256_file(path).unwrap(),
            })
        };
        let grid = root.join("grid.json");
        let grid_record = |path: &Path| serde_json::json!({"path": path.file_name().unwrap().to_str().unwrap(), "sha256": sha256_file(path).unwrap(), "bytes": fs::metadata(path).unwrap().len()});
        write_json(&grid, serde_json::json!({
            "schema_version":1,"kind":"survivor_exact_stage2_grid","construction":"full",
            "hla_environment_representation":"full","pn_hla_scope":"all","n_regimes":1,
            "tau_values_str":["1000","50052","91691","120604","156295","200000"],
            "parameter_file":grid_record(&parameters),"mn_union_file":grid_record(&mn),
            "geometries":[{"geometry_idx":0,"geometry_params_str":["15","9","0.5","0.75"],
                "mn_pairs":[[1,1]],"mn_file":grid_record(&mn)}]
        }));
        let run_manifest = root.join("run_manifest.json");
        let qpi_manifest = root.join("qpi_manifest.json");
        let pn_manifest = root.join("pn_manifest.json");
        write_json(
            &run_manifest,
            serde_json::json!({
                "dataset": "PDAC",
                "run_id": "test_run",
                "hla_environment_representation": "full",
                "query_input_file": query,
                "common_fingerprint": "common",
                "inputs": [input("query_peptides", &query), input("survivor_grid", &grid)],
            }),
        );
        write_json(
            &qpi_manifest,
            serde_json::json!({
                "dataset": "PDAC",
                "run_id": "test_run",
                "mode": "qpi",
                "common_fingerprint": "common",
                "mode_fingerprint": "qpi-fingerprint",
                "query_input_file": query,
                "total_params": 6,
                "param_chunks": 1,
                "inputs": [input("survivor_grid", &grid)],
            }),
        );
        write_json(
            &pn_manifest,
            serde_json::json!({
                "dataset": "PDAC",
                "run_id": "test_run",
                "mode": "pn",
                "common_fingerprint": "common",
                "mode_fingerprint": "pn-fingerprint",
                "query_input_file": query,
                "total_params": 6,
                "param_chunks": 1,
                "pn_hla_scope": "all",
                "max_num_ps_values_log2": 14,
                "max_num_ps_values": 16384,
                "inputs": [
                    input("survivor_grid", &grid),
                    input("parameter_sets", &parameters),
                    input("mn_tuples", &mn),
                ],
            }),
        );
        let contract = validate_assembly_contract(
            "PDAC",
            "test_run",
            &root,
            &query,
            &mapping,
            &run_manifest,
            &qpi_manifest,
            &pn_manifest,
        )
        .unwrap();
        assert_eq!(contract.expected_grid.parameter_rows, 6);
        assert_eq!(contract.expected_grid.mn_tuples, vec![(1, 1)]);

        let pn_dir = root.join("results_pn_env_id_0_1");
        let qpi_dir = root.join("results_qpi_env_id_0_1");
        fs::create_dir_all(&pn_dir).unwrap();
        fs::create_dir_all(&qpi_dir).unwrap();
        let pn_output = pn_dir.join("pn.csv");
        let qpi_output = qpi_dir.join("qpi.csv");
        fs::write(&pn_output, "x\n1\n").unwrap();
        fs::write(&qpi_output, "x\n1\n").unwrap();
        write_json(
            &pn_dir.join("TASK_0.done.json"),
            serde_json::json!({
                "status": "success",
                "run_id": "test_run",
                "fingerprint": "pn-fingerprint",
                "mode": "pn",
                "output_files": [{"path": pn_output.strip_prefix(&root).unwrap(), "size": fs::metadata(&pn_output).unwrap().len(), "sha256": sha256_file(&pn_output).unwrap()}],
            }),
        );
        write_json(
            &qpi_dir.join("TASK_0.done.json"),
            serde_json::json!({
                "status": "success",
                "run_id": "test_run",
                "fingerprint": "qpi-fingerprint",
                "mode": "qpi",
                "output_files": [{"path": qpi_output.strip_prefix(&root).unwrap(), "size": fs::metadata(&qpi_output).unwrap().len(), "sha256": sha256_file(&qpi_output).unwrap()}],
            }),
        );
        validate_discovered_outputs(
            &contract,
            std::slice::from_ref(&pn_output),
            std::slice::from_ref(&qpi_output),
        )
        .unwrap();
        let unrecorded = qpi_dir.join("stale.csv");
        fs::write(&unrecorded, "x\n1\n").unwrap();
        assert!(validate_discovered_outputs(
            &contract,
            std::slice::from_ref(&pn_output),
            &[qpi_output, unrecorded],
        )
        .is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
