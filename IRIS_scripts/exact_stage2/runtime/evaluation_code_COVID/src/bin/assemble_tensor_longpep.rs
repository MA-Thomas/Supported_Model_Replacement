// src/bin/assemble_tensor_longpep.rs
//
// Assembles F tensor for the long-peptide evaluation context.
//
// Differences from assemble_tensor.rs:
//
//   - Labels are at the long-peptide level, not the n-mer level.
//     The input is a single mapping CSV (not separate MT/WT label CSVs).
//
//     Required columns:
//       patient_id, env_id, long_peptide, nmer, gene
//
//     Optional column:
//       cancer_type   (defaults to "NA" if absent — it is only carried
//                      through to the observations parquet and is not used
//                      in any computation)
//
//     NOTE on labels: this binary does NOT read or require a label column.
//     The label written into the observations parquet is always a
//     placeholder (0); the real label is produced downstream by
//     evaluate_long_peptides.py, which reads `long_peptide_label` from the
//     mapping CSV directly, OR derives it by thresholding a continuous
//     response column (e.g. cd8_IFNg_dmso_adj) via --response-column /
//     --label-threshold. Any extra columns in the mapping CSV (a label
//     column, response columns, a `condition` column, etc.) are ignored
//     here. This keeps a single assembler binary working for the PDAC
//     mapping (which carries long_peptide_label + cancer_type) and for the
//     Covid mappings (which carry a continuous response column and neither
//     of those two fields).
//
//   - Observations are deduplicated scoreable
//     (patient × nmer × HLA × env_id) units.
//     The authoritative filtered Stage 2 query CSV defines the exact HLA set
//     for each patient/environment/n-mer. Mapping rows absent from that roster
//     are intentionally unscoreable and are audited rather than crossed with
//     every HLA seen in an environment. Duplicate long-peptide contexts are
//     expanded downstream by evaluate_long_peptides.py when scored observations
//     are merged back to the full mapping CSV.
//
//   - The observation index key is (nmer, hla, env_id), identical to
//     assemble_tensor.rs so that process_ftensor_file is structurally
//     identical to process_pn_file. The index stores a list so a shared tensor
//     key can populate more than one patient-specific observation.
//
//   - PN CSV columns: peptide, env_id, M, N, p_pos, p_neg
//     env_id is read from each CSV row (not derived from the directory range).
//
//   - Q and Pi values are joined from results_qpi_env_id_* files, identical
//     to assemble_tensor.rs.
//
//   - wt_mt_group_id is a placeholder representative context for the deduped
//     scoreable unit. Long-peptide grouping/labels are reconstructed downstream
//     from the mapping CSV, not from this observations parquet.
//
// Everything else — parameter grid discovery, chunked parallel file
// processing, F tensor writing, metadata JSON writing — is unchanged from
// assemble_tensor.rs.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use csv::ReaderBuilder;
use glob::glob;
use indicatif::{ProgressBar, ProgressStyle};
use log::{debug, info, warn};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use serde::Serialize;

use arrow::array::*;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

use immunogenicity_evaluation::{
    assembly_contract::{
        validate_assembly_contract, validate_discovered_grid, validate_discovered_outputs,
    },
    Observation,
};

// =============================================================================
// CLI
// =============================================================================

#[derive(Parser, Debug)]
#[command(name = "assemble_tensor_longpep")]
#[command(about = "Assemble F tensor for long-peptide evaluation (nmer × HLA observations)")]
struct Args {
    /// Base directories (usually just 1, sometimes 2 if processing additional peptide runs from same dataset) containing results_pn_env_id_* and results_qpi_env_id_*
    /// subdirectories.
    #[arg(long, num_args = 1..)]
    results_dirs: Vec<PathBuf>,

    /// Dataset recorded by the Stage 2 contract (COVID_SPIKE or COVID_NONSPIKE).
    #[arg(long)]
    dataset: String,

    /// Run ID recorded by all three Stage 2 manifests.
    #[arg(long)]
    run_id: String,

    /// Immutable common Stage 2 run manifest.
    #[arg(long)]
    run_manifest: PathBuf,

    /// Immutable Stage 2 Q/Pi mode manifest.
    #[arg(long)]
    qpi_manifest: PathBuf,

    /// Immutable Stage 2 P/N mode manifest.
    #[arg(long)]
    pn_manifest: PathBuf,

    /// Mapping CSV. Required columns: patient_id, env_id, long_peptide, nmer, gene.
    /// Optional: cancer_type (defaults to "NA"). Any other columns (a label column,
    /// continuous response columns, condition, etc.) are ignored.
    #[arg(long)]
    mapping: PathBuf,

    /// Authoritative filtered Stage 2 query CSV. Required columns:
    /// PatientID, env_id, peptide, HLA-RE.
    #[arg(long)]
    query_peptides: PathBuf,

    /// Output Parquet file path (observations written to same stem + .observations.parquet)
    #[arg(long)]
    output: PathBuf,

    /// Number of parallel threads (default: all available)
    #[arg(long)]
    threads: Option<usize>,

    /// Deprecated compatibility flag. Q coverage is always required for retained observations.
    #[arg(long, default_value_t = false)]
    expect_q: bool,

    /// Deprecated compatibility flag. Pi coverage is always required for retained observations.
    #[arg(long, default_value_t = false)]
    expect_pi: bool,
}

// =============================================================================
// Mapping CSV record
// =============================================================================
//
// Parsed manually (by header name) rather than via serde so that the set of
// required columns is small and explicit, extra columns are ignored, and
// cancer_type can be optional.
//
// Required: patient_id, env_id, long_peptide, nmer, gene
// Optional: cancer_type (defaults to "NA")

#[derive(Debug)]
struct MappingRecord {
    patient_id: String,
    env_id: u32,
    long_peptide: String,
    nmer: String,
    gene: String,
    cancer_type: String,
}

// =============================================================================
// Load mapping CSV
// =============================================================================

fn load_mapping(path: &Path) -> Result<Vec<MappingRecord>> {
    info!("Loading mapping file from {:?}", path);

    let file =
        File::open(path).with_context(|| format!("Failed to open mapping file: {:?}", path))?;

    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_reader(BufReader::new(file));

    let headers = reader
        .headers()
        .with_context(|| format!("Failed to read headers from mapping file: {:?}", path))?
        .clone();
    let hidx = HeaderIndex::from_headers(&headers);

    let patient_id_idx = hidx.get_required("patient_id")?;
    let env_id_idx = hidx.get_required("env_id")?;
    let long_peptide_idx = hidx.get_required("long_peptide")?;
    let nmer_idx = hidx.get_required("nmer")?;
    let gene_idx = hidx.get_required("gene")?;
    // cancer_type is optional; if the column is absent we default to "NA".
    let cancer_type_idx = hidx.map.get("cancer_type").copied();

    if cancer_type_idx.is_none() {
        info!("Mapping has no 'cancer_type' column — defaulting cancer_type to \"NA\"");
    }

    let mut records: Vec<MappingRecord> = Vec::new();
    for (row_no, result) in reader.records().enumerate() {
        let record = result
            .with_context(|| format!("Failed to read record from mapping file: {:?}", path))?;

        let env_id_raw = record.get(env_id_idx).unwrap_or("").trim();
        let env_id: u32 = env_id_raw.parse().with_context(|| {
            format!(
                "Failed to parse env_id '{}' as integer (mapping data row {})",
                env_id_raw,
                row_no + 1
            )
        })?;

        let cancer_type = cancer_type_idx
            .and_then(|i| record.get(i))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "NA".to_string());

        records.push(MappingRecord {
            patient_id: record.get(patient_id_idx).unwrap_or("").trim().to_string(),
            env_id,
            long_peptide: record
                .get(long_peptide_idx)
                .unwrap_or("")
                .trim()
                .to_string(),
            nmer: record.get(nmer_idx).unwrap_or("").trim().to_string(),
            gene: record.get(gene_idx).unwrap_or("").trim().to_string(),
            cancer_type,
        });
    }

    info!(
        "Loaded {} mapping rows ({} unique n-mers, {} unique env_ids)",
        records.len(),
        records
            .iter()
            .map(|r| r.nmer.as_str())
            .collect::<HashSet<_>>()
            .len(),
        records
            .iter()
            .map(|r| r.env_id)
            .collect::<HashSet<_>>()
            .len(),
    );
    Ok(records)
}

// =============================================================================
// Load authoritative filtered HLA-specific query roster
// =============================================================================

type QueryRosterKey = (String, u32, String);
type QueryRoster = HashMap<QueryRosterKey, HashSet<String>>;

fn load_query_roster(path: &Path) -> Result<QueryRoster> {
    info!("Loading filtered HLA-specific query roster from {:?}", path);
    let file = File::open(path)
        .with_context(|| format!("Failed to open filtered query file: {:?}", path))?;
    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_reader(BufReader::new(file));

    let headers = reader
        .headers()
        .with_context(|| {
            format!(
                "Failed to read headers from filtered query file: {:?}",
                path
            )
        })?
        .clone();
    let hidx = HeaderIndex::from_headers(&headers);
    let peptide_idx = hidx.get_required("peptide")?;
    let hla_idx = hidx.get_required("HLA-RE")?;
    let patient_idx = hidx.get_required("PatientID")?;
    let env_idx = hidx.get_required("env_id")?;

    let mut roster: QueryRoster = HashMap::new();
    let mut n_rows = 0usize;
    for (row_no, result) in reader.records().enumerate() {
        let record = result.with_context(|| {
            format!(
                "Failed to read filtered query row {} from {:?}",
                row_no + 2,
                path
            )
        })?;
        let peptide = record.get(peptide_idx).unwrap_or("").trim().to_uppercase();
        let hla = normalize_hla(record.get(hla_idx).unwrap_or(""));
        let patient_id = record.get(patient_idx).unwrap_or("").trim().to_string();
        let env_raw = record.get(env_idx).unwrap_or("").trim();
        let env_id: u32 = env_raw.parse().with_context(|| {
            format!(
                "Invalid env_id '{}' in filtered query data row {}",
                env_raw,
                row_no + 2
            )
        })?;
        if peptide.is_empty() || hla.is_empty() || patient_id.is_empty() {
            bail!(
                "Missing peptide, HLA-RE, or PatientID in filtered query data row {}",
                row_no + 2
            );
        }
        roster
            .entry((patient_id, env_id, peptide))
            .or_default()
            .insert(hla);
        n_rows += 1;
    }

    if roster.is_empty() {
        bail!("Filtered query roster is empty: {:?}", path);
    }
    let n_hla_specific: usize = roster.values().map(HashSet::len).sum();
    info!(
        "Loaded {} filtered query rows defining {} patient/env/peptide keys and {} exact HLA-specific candidates",
        n_rows,
        roster.len(),
        n_hla_specific,
    );
    Ok(roster)
}

// =============================================================================
// Filename parsing
// =============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FilenameParams {
    d_pos: String,
    d_neg: String,
    steepness_pos: String,
    steepness_neg: String,
    tau_thymus: String,
    hla: String,
    expr_dataset: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct GeometryKey {
    d_pos: String,
    d_neg: String,
    steepness_pos: String,
    steepness_neg: String,
}

fn normalize_hla(raw: &str) -> String {
    raw.trim()
        .strip_prefix("HLA-")
        .unwrap_or(raw.trim())
        .to_uppercase()
}

/// Parse a PN-style filename.
/// Format: query_peptides_results_dpos_<...>_dneg_<...>_steepness_pos_<...>
///         _steepness_neg_<...>_fft_size_<...>_tau_thymus_<...>_HLA_<...>_expr_<...>.csv
fn parse_pn_filename(filename: &str) -> Option<FilenameParams> {
    let s = filename.strip_suffix(".csv").unwrap_or(filename);
    let rest = s.strip_prefix("query_peptides_results_dpos_")?;

    let mut parts = rest.splitn(2, "_dneg_");
    let d_pos = parts.next()?.to_string();
    let rest = parts.next()?;

    let mut parts = rest.splitn(2, "_steepness_pos_");
    let d_neg = parts.next()?.to_string();
    let rest = parts.next()?;

    let mut parts = rest.splitn(2, "_steepness_neg_");
    let steepness_pos = parts.next()?.to_string();
    let rest = parts.next()?;

    let mut parts = rest.splitn(2, "_fft_size_");
    let steepness_neg = parts.next()?.to_string();
    let rest = parts.next()?;

    let mut parts = rest.splitn(2, "_tau_thymus_");
    let _fft_size = parts.next()?.to_string();
    let rest = parts.next()?;

    let mut parts = rest.splitn(2, "_HLA_");
    let tau_thymus = parts.next()?.to_string();
    let rest = parts.next()?;

    let mut parts = rest.splitn(2, "_expr_");
    let hla = normalize_hla(parts.next()?);
    let expr_dataset = parts.next()?.to_string();

    Some(FilenameParams {
        d_pos,
        d_neg,
        steepness_pos,
        steepness_neg,
        tau_thymus,
        hla,
        expr_dataset,
    })
}

/// Parse a Q-values filename.
/// Format: query_peptides_results_HLA_<hla>_q_values.csv
fn parse_q_filename(filename: &str) -> Option<String> {
    let s = filename.strip_suffix(".csv")?;
    let rest = s.strip_prefix("query_peptides_results_HLA_")?;
    let hla = rest.strip_suffix("_q_values")?;
    Some(normalize_hla(hla))
}

/// Parse a Pi-values filename.
/// Format: query_peptides_results_HLA_<hla>_pi_values.csv
fn parse_pi_filename(filename: &str) -> Option<String> {
    let s = filename.strip_suffix(".csv")?;
    let rest = s.strip_prefix("query_peptides_results_HLA_")?;
    let hla = rest.strip_suffix("_pi_values")?;
    Some(normalize_hla(hla))
}

// =============================================================================
// CSV schema helpers
// =============================================================================

#[derive(Debug, Clone)]
struct HeaderIndex {
    map: FxHashMap<String, usize>,
}

impl HeaderIndex {
    fn from_headers(headers: &csv::StringRecord) -> Self {
        let map = headers
            .iter()
            .enumerate()
            .map(|(i, h)| (h.trim().to_string(), i))
            .collect();
        Self { map }
    }

    fn get_required(&self, name: &str) -> Result<usize> {
        self.map
            .get(name)
            .copied()
            .ok_or_else(|| anyhow!("Missing required column '{}'", name))
    }
}

// =============================================================================
// Q and Pi stores
// =============================================================================

/// Q is keyed by (peptide, hla, env_id) because it varies per environment.
type QKey = (String, String, u32);
type QStore = FxHashMap<QKey, f32>;

/// Pi is keyed by (peptide, hla) because it depends only on peptide and HLA.
type PiKey = (String, String);
type PiStore = FxHashMap<PiKey, f32>;

fn parse_f32_strict(value: &str, field: &str, path: &Path) -> Result<f32> {
    let parsed = value.trim().parse::<f32>().with_context(||
        format!("Invalid or missing {field} value {value:?} in {}", path.display()))?;
    if !parsed.is_finite() || parsed < 0.0 {
        bail!("Expected finite nonnegative {field}, got {parsed} in {}", path.display());
    }
    Ok(parsed)
}

fn collect_q_store(qpi_files: &[PathBuf]) -> Result<QStore> {
    info!("Collecting Q values from {} QPI files...", qpi_files.len());

    let pb = ProgressBar::new(qpi_files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] Collecting Q: [{bar:40.cyan/blue}] {pos}/{len}")
            .unwrap(),
    );

    let partials: Vec<Result<QStore>> = qpi_files
        .par_iter()
        .map(|file_path| {
            let mut local: QStore = FxHashMap::default();

            let filename = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let hla = match parse_q_filename(filename) {
                Some(h) => h,
                None => {
                    pb.inc(1);
                    return Ok(local);
                }
            };

            let file = File::open(file_path)
                .with_context(|| format!("Failed to open Q file: {:?}", file_path))?;
            let mut reader = ReaderBuilder::new()
                .has_headers(true)
                .trim(csv::Trim::All)
                .from_reader(BufReader::new(file));

            let headers = reader
                .headers()
                .with_context(|| format!("Failed to read headers from {:?}", file_path))?
                .clone();
            let hidx = HeaderIndex::from_headers(&headers);
            let peptide_idx = hidx.get_required("peptide")?;
            let env_id_idx = hidx.get_required("env_id")?;
            let q_idx = hidx.get_required("q_value")?;

            for result in reader.records() {
                let record = result
                    .with_context(|| format!("Failed to read record from {:?}", file_path))?;

                let peptide = record.get(peptide_idx).unwrap_or("").trim().to_uppercase();
                if peptide.is_empty() {
                    continue;
                }

                let env_id: u32 = match record.get(env_id_idx).unwrap_or("").trim().parse() {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let q_value = parse_f32_strict(record.get(q_idx).unwrap_or(""), "q_value", file_path)?;
                let key = (peptide, hla.clone(), env_id);

                if let Some(&existing) = local.get(&key) {
                    if (existing - q_value).abs() > 1e-6 {
                        bail!(
                            "Conflicting q_value in {:?}: existing={} new={}",
                            file_path,
                            existing,
                            q_value
                        );
                    }
                } else {
                    local.insert(key, q_value);
                }
            }

            pb.inc(1);
            Ok(local)
        })
        .collect();

    pb.finish_with_message("Q collection complete");

    let mut merged: QStore = FxHashMap::default();
    for part in partials {
        let store = part?;
        for (key, q) in store {
            if let Some(&existing) = merged.get(&key) {
                if (existing - q).abs() > 1e-6 {
                    bail!(
                        "Conflicting q_value during merge: existing={} new={}",
                        existing,
                        q
                    );
                }
            } else {
                merged.insert(key, q);
            }
        }
    }

    info!("Collected {} Q entries", merged.len());
    Ok(merged)
}

fn collect_pi_store(qpi_files: &[PathBuf]) -> Result<PiStore> {
    info!("Collecting Pi values from {} QPI files...", qpi_files.len());

    let pb = ProgressBar::new(qpi_files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] Collecting Pi: [{bar:40.cyan/blue}] {pos}/{len}")
            .unwrap(),
    );

    let partials: Vec<Result<PiStore>> = qpi_files
        .par_iter()
        .map(|file_path| {
            let mut local: PiStore = FxHashMap::default();

            let filename = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let hla = match parse_pi_filename(filename) {
                Some(h) => h,
                None => {
                    pb.inc(1);
                    return Ok(local);
                }
            };

            let file = File::open(file_path)
                .with_context(|| format!("Failed to open Pi file: {:?}", file_path))?;
            let mut reader = ReaderBuilder::new()
                .has_headers(true)
                .trim(csv::Trim::All)
                .from_reader(BufReader::new(file));

            let headers = reader
                .headers()
                .with_context(|| format!("Failed to read headers from {:?}", file_path))?
                .clone();
            let hidx = HeaderIndex::from_headers(&headers);
            let peptide_idx = hidx.get_required("peptide")?;
            let pi_idx = hidx.get_required("pi_value")?;

            for result in reader.records() {
                let record = result
                    .with_context(|| format!("Failed to read record from {:?}", file_path))?;

                let peptide = record.get(peptide_idx).unwrap_or("").trim().to_uppercase();
                if peptide.is_empty() {
                    continue;
                }

                let pi_value = parse_f32_strict(record.get(pi_idx).unwrap_or(""), "pi_value", file_path)?;
                local.insert((peptide, hla.clone()), pi_value);
            }

            pb.inc(1);
            Ok(local)
        })
        .collect();

    pb.finish_with_message("Pi collection complete");

    let mut merged: PiStore = FxHashMap::default();
    for part in partials {
        merged.extend(part?);
    }

    info!("Collected {} Pi entries", merged.len());
    Ok(merged)
}

// =============================================================================
// Build deduplicated observations from exact filtered query candidates
// =============================================================================

fn build_observations(
    mapping: &[MappingRecord],
    query_roster: &QueryRoster,
) -> Result<Vec<Observation>> {
    let mut observations: Vec<Observation> = Vec::new();
    let mut seen: HashSet<(String, u32, String, String)> = HashSet::new();
    let mut n_mapping_rows_outside_query_roster = 0usize;
    let mut n_duplicate_contexts = 0usize;
    let mut mapped_endpoints: HashSet<(String, String)> = HashSet::new();
    let mut supported_endpoints: HashSet<(String, String)> = HashSet::new();

    for record in mapping {
        let endpoint_key = (record.patient_id.clone(), record.long_peptide.clone());
        mapped_endpoints.insert(endpoint_key.clone());

        let peptide = record.nmer.trim().to_uppercase();
        if peptide.is_empty() {
            bail!(
                "Mapping contains an empty nmer for patient={} env={} long_peptide={}",
                record.patient_id,
                record.env_id,
                record.long_peptide,
            );
        }

        let roster_key = (record.patient_id.clone(), record.env_id, peptide.clone());
        let hlas = match query_roster.get(&roster_key) {
            Some(hlas) => {
                supported_endpoints.insert(endpoint_key);
                hlas
            }
            None => {
                debug!(
                    "Mapping row is outside the filtered query roster: patient={} env={} nmer={}",
                    record.patient_id, record.env_id, peptide,
                );
                n_mapping_rows_outside_query_roster += 1;
                continue;
            }
        };

        for hla in hlas {
            let observation_identity = (
                record.patient_id.clone(),
                record.env_id,
                peptide.clone(),
                hla.clone(),
            );
            if !seen.insert(observation_identity) {
                n_duplicate_contexts += 1;
                continue;
            }

            let wt_mt_group_id = format!("{}_{}", record.patient_id, record.long_peptide);

            observations.push(Observation {
                peptide: peptide.clone(),
                hla: hla.clone(),
                env_id: record.env_id,
                patient_id: record.patient_id.clone(),
                // Placeholder: long_peptide_label is at the long-peptide level
                // and is read from the mapping CSV by evaluate_long_peptides.py.
                label: 0,
                gene: record.gene.clone(),
                cancer_type: record.cancer_type.clone(),
                // Representative context only.  Downstream COVID evaluation
                // reconstructs long-peptide groups from the mapping CSV.
                wt_mt_group_id,
            });
        }
    }

    info!(
        "Built {} exact patient/env/nmer/HLA observations from {} mapping rows and {} endpoints ({} endpoints have at least one filtered candidate; {} mapping rows were outside the filtered query roster; {} duplicate contexts skipped)",
        observations.len(),
        mapping.len(),
        mapped_endpoints.len(),
        supported_endpoints.len(),
        n_mapping_rows_outside_query_roster,
        n_duplicate_contexts,
    );
    Ok(observations)
}

// =============================================================================
// Observation index
// =============================================================================

type ObservationIndex = FxHashMap<(String, String, u32), Vec<usize>>;

fn build_observation_index(observations: &[Observation]) -> ObservationIndex {
    let mut index: ObservationIndex = FxHashMap::default();
    for (obs_idx, observation) in observations.iter().enumerate() {
        index
            .entry((
                observation.peptide.clone(),
                observation.hla.clone(),
                observation.env_id,
            ))
            .or_default()
            .push(obs_idx);
    }
    index
}

// =============================================================================
// Parameter grid
// =============================================================================

#[derive(Debug, Clone, Serialize)]
struct ParameterGridMetadata {
    tau_values_str: Vec<String>,
    tau_values: Vec<f32>,
    n_geometry: usize,
    n_tau: usize,
    n_mn: usize,
    n_params: usize,
    geometry_params_str: Vec<(String, String, String, String)>,
    geometry_params: Vec<(f32, f32, f32, f32)>,
    mn_tuples: Vec<(u32, u32)>,
}

fn discover_parameter_grid(files: &[PathBuf]) -> Result<ParameterGridMetadata> {
    info!(
        "Scanning {} PN files to discover parameter grid...",
        files.len()
    );

    let mut geometry_keys: BTreeSet<GeometryKey> = BTreeSet::new();
    let mut tau_values: BTreeSet<String> = BTreeSet::new();
    let mut mn_tuples: BTreeSet<(u32, u32)> = BTreeSet::new();

    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] Scanning: [{bar:40.cyan/blue}] {pos}/{len}",
            )
            .unwrap(),
    );

    for file_path in files {
        let filename = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if let Some(params) = parse_pn_filename(filename) {
            geometry_keys.insert(GeometryKey {
                d_pos: params.d_pos,
                d_neg: params.d_neg,
                steepness_pos: params.steepness_pos,
                steepness_neg: params.steepness_neg,
            });
            tau_values.insert(params.tau_thymus);

            if let Ok(file) = File::open(file_path) {
                let mut reader = ReaderBuilder::new()
                    .has_headers(true)
                    .trim(csv::Trim::All)
                    .from_reader(BufReader::new(file));

                let headers = match reader.headers() {
                    Ok(h) => h.clone(),
                    Err(_) => continue,
                };
                let hidx = HeaderIndex::from_headers(&headers);
                let m_col = match hidx.get_required("M") {
                    Ok(i) => i,
                    Err(_) => continue,
                };
                let n_col = match hidx.get_required("N") {
                    Ok(i) => i,
                    Err(_) => continue,
                };

                for result in reader.records() {
                    if let Ok(record) = result {
                        if let (Ok(m), Ok(n)) = (
                            record.get(m_col).unwrap_or("").trim().parse::<u32>(),
                            record.get(n_col).unwrap_or("").trim().parse::<u32>(),
                        ) {
                            mn_tuples.insert((m, n));
                        }
                    }
                }
            }
        }
        pb.inc(1);
    }
    pb.finish_with_message("Scan complete");

    let geometry_params_str: Vec<(String, String, String, String)> = geometry_keys
        .iter()
        .map(|g| {
            (
                g.d_pos.clone(),
                g.d_neg.clone(),
                g.steepness_pos.clone(),
                g.steepness_neg.clone(),
            )
        })
        .collect();

    let mut tau_with_numeric: Vec<(String, f32)> = tau_values
        .into_iter()
        .map(|s| {
            let f = s.parse::<f32>().unwrap_or(0.0);
            (s, f)
        })
        .collect();
    tau_with_numeric.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    let tau_values_str: Vec<String> = tau_with_numeric.iter().map(|(s, _)| s.clone()).collect();
    let tau_values_f32: Vec<f32> = tau_with_numeric.iter().map(|(_, f)| *f).collect();

    let geometry_params_f32: Vec<(f32, f32, f32, f32)> = geometry_params_str
        .iter()
        .map(|(a, b, c, d)| {
            (
                a.parse().unwrap_or(0.0),
                b.parse().unwrap_or(0.0),
                c.parse().unwrap_or(0.0),
                d.parse().unwrap_or(0.0),
            )
        })
        .collect();

    let mn_tuples_vec: Vec<(u32, u32)> = mn_tuples.into_iter().collect();
    let n_geometry = geometry_params_str.len();
    let n_tau = tau_values_str.len();
    let n_mn = mn_tuples_vec.len();
    let n_params = n_geometry * n_tau;

    info!("Discovered parameter grid:");
    info!("  {} geometry settings", n_geometry);
    info!("  {} τ values: {:?}", n_tau, tau_values_str);
    info!("  {} (M, N) tuples", n_mn);
    info!(
        "  {} geometry × {} τ = {} param combinations",
        n_geometry, n_tau, n_params
    );

    Ok(ParameterGridMetadata {
        tau_values_str,
        tau_values: tau_values_f32,
        n_geometry,
        n_tau,
        n_mn,
        n_params,
        geometry_params_str,
        geometry_params: geometry_params_f32,
        mn_tuples: mn_tuples_vec,
    })
}

struct ParameterIndices {
    geometry_to_idx: FxHashMap<(String, String, String, String), u32>,
    tau_to_idx: FxHashMap<String, u32>,
    mn_to_idx: FxHashMap<(u32, u32), u32>,
}

fn build_parameter_indices(metadata: &ParameterGridMetadata) -> ParameterIndices {
    let geometry_to_idx = metadata
        .geometry_params_str
        .iter()
        .enumerate()
        .map(|(i, g)| (g.clone(), i as u32))
        .collect();
    let tau_to_idx = metadata
        .tau_values_str
        .iter()
        .enumerate()
        .map(|(i, t)| (t.clone(), i as u32))
        .collect();
    let mn_to_idx = metadata
        .mn_tuples
        .iter()
        .enumerate()
        .map(|(i, &mn)| (mn, i as u32))
        .collect();
    ParameterIndices {
        geometry_to_idx,
        tau_to_idx,
        mn_to_idx,
    }
}

// =============================================================================
// PN file processing
// =============================================================================

/// Process one PN file and produce tensor entries.
///
/// env_id is read from each CSV row. Q is looked up by (peptide, hla, env_id).
/// Pi is looked up by (peptide, hla).
///
/// Returns Vec<(obs_idx, param_idx, mn_idx, q_value, pos_prob, neg_prob, pi_value)>
fn process_ftensor_file(
    file_path: &Path,
    file_params: &FilenameParams,
    indices: &ParameterIndices,
    obs_index: &ObservationIndex,
    q_store: &QStore,
    pi_store: &PiStore,
    n_tau: usize,
) -> Result<Vec<(usize, u32, u32, f32, f32, f32, f32)>> {
    let geometry_key = (
        file_params.d_pos.clone(),
        file_params.d_neg.clone(),
        file_params.steepness_pos.clone(),
        file_params.steepness_neg.clone(),
    );

    let geometry_idx = *indices
        .geometry_to_idx
        .get(&geometry_key)
        .ok_or_else(|| anyhow!("Unknown geometry key in {:?}", file_path))?;
    let tau_idx = *indices
        .tau_to_idx
        .get(&file_params.tau_thymus)
        .ok_or_else(|| anyhow!("Unknown tau value in {:?}", file_path))?;
    let param_idx = geometry_idx * (n_tau as u32) + tau_idx;

    let file = File::open(file_path)
        .with_context(|| format!("Failed to open PN file: {:?}", file_path))?;
    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_reader(BufReader::new(file));

    let headers = reader
        .headers()
        .with_context(|| format!("Failed to read headers from {:?}", file_path))?
        .clone();
    let hidx = HeaderIndex::from_headers(&headers);

    let peptide_idx = hidx.get_required("peptide")?;
    let env_id_idx = hidx.get_required("env_id")?;
    let m_idx = hidx.get_required("M")?;
    let n_idx = hidx.get_required("N")?;
    let p_pos_idx = hidx.get_required("p_pos")?;
    let p_neg_idx = hidx.get_required("p_neg")?;

    let mut entries = Vec::new();

    // Invariant guard: a single PN file must hold at most one row per
    // (peptide, env_id, M, N).  Duplicate rows are the fingerprint of a roster
    // that was not deduplicated on its compute key before Stage 2 (e.g.
    // homozygous allele copies), and they inflate the tensor with duplicate
    // (obs_idx, param_idx, mn_idx) cells.  Collapse numerically-identical
    // duplicates and fail loudly on genuine value conflicts.
    let mut seen_rows: FxHashMap<(String, u32, u32, u32), (f32, f32)> = FxHashMap::default();
    let mut n_collapsed = 0usize;

    for result in reader.records() {
        let record =
            result.with_context(|| format!("Failed to read record from {:?}", file_path))?;

        let peptide = record.get(peptide_idx).unwrap_or("").trim().to_uppercase();
        if peptide.is_empty() {
            continue;
        }

        let env_id: u32 = match record.get(env_id_idx).unwrap_or("").trim().parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let m: u32 = match record.get(m_idx).unwrap_or("").trim().parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let n: u32 = match record.get(n_idx).unwrap_or("").trim().parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let mn_idx = match indices.mn_to_idx.get(&(m, n)) {
            Some(&idx) => idx,
            None => {
                debug!("Unknown (M,N)=({},{}) in {:?}", m, n, file_path);
                continue;
            }
        };

        let p_pos = parse_f32_strict(record.get(p_pos_idx).unwrap_or(""), "pos_prob", file_path)?;
        let p_neg = parse_f32_strict(record.get(p_neg_idx).unwrap_or(""), "neg_prob", file_path)?;

        let obs_key = (peptide.clone(), file_params.hla.clone(), env_id);
        if let Some(obs_indices) = obs_index.get(&obs_key) {
            let q_value = q_store
                .get(&(peptide.clone(), file_params.hla.clone(), env_id))
                .copied()
                .with_context(|| {
                    format!(
                        "Missing required q_value for peptide='{}', hla='{}', env_id={} in {:?}",
                        peptide, file_params.hla, env_id, file_path
                    )
                })?;
            let pi_value = pi_store
                .get(&(peptide.clone(), file_params.hla.clone()))
                .copied()
                .with_context(|| {
                    format!(
                        "Missing required pi_value for peptide='{}', hla='{}' in {:?}",
                        peptide, file_params.hla, file_path
                    )
                })?;
            let row_key = (peptide.clone(), env_id, m, n);
            if let Some(&(prev_pos, prev_neg)) = seen_rows.get(&row_key) {
                if (prev_pos - p_pos).abs() > 1e-6 || (prev_neg - p_neg).abs() > 1e-6 {
                    bail!(
                        "Conflicting duplicate PN row in {:?}: peptide='{}' env_id={} \
                         M={} N={} previously (p_pos={}, p_neg={}) now (p_pos={}, p_neg={})",
                        file_path,
                        peptide,
                        env_id,
                        m,
                        n,
                        prev_pos,
                        prev_neg,
                        p_pos,
                        p_neg
                    );
                }
                // Identical duplicate — collapse it so the tensor holds one cell.
                n_collapsed += 1;
                continue;
            }
            seen_rows.insert(row_key, (p_pos, p_neg));
            for &obs_idx in obs_indices {
                entries.push((obs_idx, param_idx, mn_idx, q_value, p_pos, p_neg, pi_value));
            }
        }
    }

    if n_collapsed > 0 {
        warn!(
            "Collapsed {} duplicate PN row(s) with identical values in {:?}. A roster \
             deduplicated on (peptide, HLA, env_id, expr) should produce none — this file \
             predates the roster fix or was generated from a non-unique query CSV.",
            n_collapsed, file_path
        );
    }

    Ok(entries)
}

fn validate_complete_tensor(
    entries: &[(usize, u32, u32, f32, f32, f32, f32)],
    observations: &[Observation],
    allowed: &HashSet<(u32,u32)>,
) -> Result<()> {
    immunogenicity_evaluation::sparse_grid::validate_cells(entries, observations.len(), allowed)
}

// =============================================================================
// Main
// =============================================================================

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    if args.results_dirs.len() != 1 {
        bail!(
            "Manifest-bound assembly requires exactly one --results-dirs value, got {}",
            args.results_dirs.len()
        );
    }

    if let Some(threads) = args.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .context("Failed to configure thread pool")?;
    }

    let provenance = validate_assembly_contract(
        &args.dataset,
        &args.run_id,
        &args.results_dirs[0],
        &args.query_peptides,
        &args.mapping,
        &args.run_manifest,
        &args.qpi_manifest,
        &args.pn_manifest,
    )?;

    info!(
        "Validated Stage 2 contract: dataset={} run_id={} representation={} pn_hla_scope={} max_num_ps_values={}",
        provenance.dataset,
        provenance.run_id,
        provenance.hla_environment_representation,
        provenance.pn_hla_scope,
        provenance.max_num_ps_values,
    );

    // ------------------------------------------------------------------
    // Load mapping CSV
    // ------------------------------------------------------------------
    let mapping = load_mapping(&args.mapping)?;
    let query_roster = load_query_roster(&args.query_peptides)?;

    if !args.expect_q || !args.expect_pi {
        warn!(
            "Q and Pi coverage are now always required; --expect-q and --expect-pi are retained only for CLI compatibility"
        );
    }

    // ------------------------------------------------------------------
    // Discover PN files under results_pn_env_id_*
    // ------------------------------------------------------------------
    let mut pn_files: Vec<PathBuf> = Vec::new();

    for dir in &args.results_dirs {
        let pattern = dir.join("results_pn_env_id_*").join("*.csv");
        let pattern_str = pattern.to_string_lossy();
        info!(
            "Searching for PN files in {}: {}",
            dir.display(),
            pattern_str
        );

        let mut files: Vec<PathBuf> = glob(&pattern_str)
            .context("Failed to read PN glob pattern")?
            .filter_map(|r| r.ok())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| parse_pn_filename(n).is_some())
                    .unwrap_or(false)
            })
            .collect();

        pn_files.append(&mut files);
    }

    pn_files.sort();
    info!(
        "Found {} total PN files across all datasets",
        pn_files.len()
    );

    if pn_files.is_empty() {
        bail!(
            "No PN files found in any of the provided results_dirs:\n{:?}\n\
            Expected subdirectories matching: results_pn_env_id_*/",
            args.results_dirs
        );
    }
    // let pn_pattern = args.results_dir.join("results_pn_env_id_*").join("*.csv");
    // let pn_pattern_str = pn_pattern.to_string_lossy();
    // info!("Searching for PN files matching: {}", pn_pattern_str);

    // let mut pn_files: Vec<PathBuf> = glob(&pn_pattern_str)
    //     .context("Failed to read PN glob pattern")?
    //     .filter_map(|r| r.ok())
    //     .filter(|p| {
    //         p.file_name()
    //             .and_then(|n| n.to_str())
    //             .map(|n| parse_pn_filename(n).is_some())
    //             .unwrap_or(false)
    //     })
    //     .collect();

    // pn_files.sort();
    // info!("Found {} PN files", pn_files.len());

    // if pn_files.is_empty() {
    //     bail!("No PN files found matching pattern: {}", pn_pattern_str);
    // }

    // ------------------------------------------------------------------
    // Discover QPI files under results_qpi_env_id_*
    // ------------------------------------------------------------------
    let mut qpi_files: Vec<PathBuf> = Vec::new();

    for dir in &args.results_dirs {
        let pattern = dir.join("results_qpi_env_id_*").join("*.csv");
        let pattern_str = pattern.to_string_lossy();
        info!(
            "Searching for QPI files in {}: {}",
            dir.display(),
            pattern_str
        );

        let mut files: Vec<PathBuf> = glob(&pattern_str)
            .context("Failed to read QPI glob pattern")?
            .filter_map(|r| r.ok())
            .collect();

        qpi_files.append(&mut files);
    }

    qpi_files.sort();
    info!(
        "Found {} total QPI files across all datasets",
        qpi_files.len()
    );
    validate_discovered_outputs(&provenance, &pn_files, &qpi_files)?;
    info!("Validated exact Stage 2 output inventory against task manifests");
    // let qpi_pattern = args.results_dir.join("results_qpi_env_id_*").join("*.csv");
    // let qpi_pattern_str = qpi_pattern.to_string_lossy();
    // info!("Searching for QPI files matching: {}", qpi_pattern_str);

    // let qpi_files: Vec<PathBuf> = glob(&qpi_pattern_str)
    //     .context("Failed to read QPI glob pattern")?
    //     .filter_map(|r| r.ok())
    //     .collect();

    // info!("Found {} QPI files", qpi_files.len());

    // ------------------------------------------------------------------
    // Collect Q and Pi stores
    // ------------------------------------------------------------------
    let q_store = collect_q_store(&qpi_files)?;
    let pi_store = collect_pi_store(&qpi_files)?;

    if q_store.is_empty() {
        bail!("No Q values were found in the Stage 2 QPI outputs");
    }
    if pi_store.is_empty() {
        bail!("No Pi values were found in the Stage 2 QPI outputs");
    }

    // ------------------------------------------------------------------
    // Build only the exact HLA-specific candidates retained by Stage 2
    // ------------------------------------------------------------------
    let all_observations = build_observations(&mapping, &query_roster)?;

    if all_observations.is_empty() {
        bail!(
            "No observations could be built. Check that patient IDs, env_ids, and n-mers in \
             the mapping CSV match the authoritative filtered Stage 2 query roster."
        );
    }

    let obs_index = build_observation_index(&all_observations);
    let indexed_observations: usize = obs_index.values().map(Vec::len).sum();
    if indexed_observations != all_observations.len() {
        bail!(
            "Observation index retained {indexed_observations}/{} observations",
            all_observations.len()
        );
    }
    info!(
        "Observation index contains {} tensor keys covering {} observations",
        obs_index.len(),
        indexed_observations,
    );

    // ------------------------------------------------------------------
    // Parameter grid discovery
    // ------------------------------------------------------------------
    let grid_metadata = discover_parameter_grid(&pn_files)?;
    validate_discovered_grid(
        &provenance.expected_grid,
        &grid_metadata.geometry_params,
        &grid_metadata.tau_values,
        &grid_metadata.mn_tuples,
        grid_metadata.n_params,
    )?;
    let indices = build_parameter_indices(&grid_metadata);
    let allowed_cells = provenance.sparse_grid.cells(&grid_metadata.geometry_params,
        grid_metadata.n_tau, &grid_metadata.mn_tuples)?;

    // ------------------------------------------------------------------
    // Chunked parallel PN file processing
    // ------------------------------------------------------------------
    info!("Processing PN files to extract F tensor entries...");

    let pb = ProgressBar::new(pn_files.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] Processing: [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
        .unwrap()
        .progress_chars("#>-"));

    let chunk_size = 10_000;
    let mut all_entries: Vec<(usize, u32, u32, f32, f32, f32, f32)> = Vec::new();

    for (chunk_idx, chunk) in pn_files.chunks(chunk_size).enumerate() {
        info!(
            "Processing chunk {} / {} ({} files)",
            chunk_idx + 1,
            (pn_files.len() + chunk_size - 1) / chunk_size,
            chunk.len(),
        );

        let results: Vec<_> = chunk
            .par_iter()
            .map(|file_path| {
                let filename = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let result = match parse_pn_filename(filename) {
                    Some(file_params) => process_ftensor_file(
                        file_path,
                        &file_params,
                        &indices,
                        &obs_index,
                        &q_store,
                        &pi_store,
                        grid_metadata.n_tau,
                    ),
                    None => Ok(vec![]),
                };
                pb.inc(1);
                result
            })
            .collect();

        for result in results {
            match result {
                Ok(entries) => all_entries.extend(entries),
                Err(e) => return Err(e),
            }
        }

        info!(
            "  Chunk {} complete. Total entries so far: {}",
            chunk_idx + 1,
            all_entries.len()
        );
    }

    pb.finish_with_message("File processing complete");
    info!("Collected {} F tensor entries", all_entries.len());

    validate_complete_tensor(
        &all_entries,
        &all_observations,
        &allowed_cells,
    )?;
    info!(
        "Validated exact tensor coverage: {} observations × {} required cells",
        all_observations.len(),
        allowed_cells.len(),
    );

    // ------------------------------------------------------------------
    // Write observations parquet (label is placeholder 0)
    // ------------------------------------------------------------------
    let obs_schema = Schema::new(vec![
        Field::new("obs_idx", DataType::UInt32, false),
        Field::new("peptide", DataType::Utf8, false),
        Field::new("hla", DataType::Utf8, false),
        Field::new("env_id", DataType::UInt32, false),
        Field::new("patient_id", DataType::Utf8, false),
        Field::new("label", DataType::UInt8, false),
        Field::new("gene", DataType::Utf8, false),
        Field::new("cancer_type", DataType::Utf8, false),
        Field::new("wt_mt_group_id", DataType::Utf8, false),
    ]);

    let obs_path = args.output.with_extension("observations.parquet");
    info!("Writing observations to {:?}", obs_path);

    let obs_indices: Vec<u32> = (0..all_observations.len() as u32).collect();
    let peptides: Vec<&str> = all_observations
        .iter()
        .map(|o| o.peptide.as_str())
        .collect();
    let hlas: Vec<&str> = all_observations.iter().map(|o| o.hla.as_str()).collect();
    let env_ids: Vec<u32> = all_observations.iter().map(|o| o.env_id).collect();
    let patient_ids: Vec<&str> = all_observations
        .iter()
        .map(|o| o.patient_id.as_str())
        .collect();
    let labels: Vec<u8> = all_observations.iter().map(|o| o.label).collect();
    let genes: Vec<&str> = all_observations.iter().map(|o| o.gene.as_str()).collect();
    let cancer_types: Vec<&str> = all_observations
        .iter()
        .map(|o| o.cancer_type.as_str())
        .collect();
    let wt_mt_group_ids: Vec<&str> = all_observations
        .iter()
        .map(|o| o.wt_mt_group_id.as_str())
        .collect();

    let obs_batch = RecordBatch::try_new(
        std::sync::Arc::new(obs_schema),
        vec![
            std::sync::Arc::new(UInt32Array::from(obs_indices)),
            std::sync::Arc::new(StringArray::from(peptides)),
            std::sync::Arc::new(StringArray::from(hlas)),
            std::sync::Arc::new(UInt32Array::from(env_ids)),
            std::sync::Arc::new(StringArray::from(patient_ids)),
            std::sync::Arc::new(UInt8Array::from(labels)),
            std::sync::Arc::new(StringArray::from(genes)),
            std::sync::Arc::new(StringArray::from(cancer_types)),
            std::sync::Arc::new(StringArray::from(wt_mt_group_ids)),
        ],
    )?;

    let props = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();

    let obs_file = File::create(&obs_path)?;
    let mut obs_writer = ArrowWriter::try_new(obs_file, obs_batch.schema(), Some(props.clone()))?;
    obs_writer.write(&obs_batch)?;
    obs_writer.close()?;

    // ------------------------------------------------------------------
    // Write F tensor parquet
    // ------------------------------------------------------------------
    let tensor_schema = Schema::new(vec![
        Field::new("obs_idx", DataType::UInt32, false),
        Field::new("param_idx", DataType::UInt32, false),
        Field::new("mn_idx", DataType::UInt32, false),
        Field::new("q_value", DataType::Float32, false),
        Field::new("pos_prob", DataType::Float32, false),
        Field::new("neg_prob", DataType::Float32, false),
        Field::new("pi_eh", DataType::Float32, false),
    ]);

    info!("Writing F tensor to {:?}", args.output);
    let tensor_file = File::create(&args.output)?;
    let mut tensor_writer = ArrowWriter::try_new(
        tensor_file,
        std::sync::Arc::new(tensor_schema.clone()),
        Some(props.clone()),
    )?;

    let batch_size = 1_000_000;
    for chunk in all_entries.chunks(batch_size) {
        let obs_idx: Vec<u32> = chunk.iter().map(|e| e.0 as u32).collect();
        let param_idx: Vec<u32> = chunk.iter().map(|e| e.1).collect();
        let mn_idx: Vec<u32> = chunk.iter().map(|e| e.2).collect();
        let q_value: Vec<f32> = chunk.iter().map(|e| e.3).collect();
        let pos_prob: Vec<f32> = chunk.iter().map(|e| e.4).collect();
        let neg_prob: Vec<f32> = chunk.iter().map(|e| e.5).collect();
        let pi_eh: Vec<f32> = chunk.iter().map(|e| e.6).collect();

        let batch = RecordBatch::try_new(
            std::sync::Arc::new(tensor_schema.clone()),
            vec![
                std::sync::Arc::new(UInt32Array::from(obs_idx)),
                std::sync::Arc::new(UInt32Array::from(param_idx)),
                std::sync::Arc::new(UInt32Array::from(mn_idx)),
                std::sync::Arc::new(Float32Array::from(q_value)),
                std::sync::Arc::new(Float32Array::from(pos_prob)),
                std::sync::Arc::new(Float32Array::from(neg_prob)),
                std::sync::Arc::new(Float32Array::from(pi_eh)),
            ],
        )?;
        tensor_writer.write(&batch)?;
    }
    tensor_writer.close()?;

    // ------------------------------------------------------------------
    // Write metadata JSON
    // ------------------------------------------------------------------
    let metadata_path = args.output.with_extension("metadata.json");
    info!("Writing parameter grid and run provenance metadata to {:?}", metadata_path);
    let metadata_file = File::create(&metadata_path)?;
    let mut metadata = serde_json::to_value(&grid_metadata)?;
    let object = metadata
        .as_object_mut()
        .context("Serialized parameter metadata was not a JSON object")?;
    let string = |value: String| serde_json::Value::String(value);
    object.insert("assembly_schema_version".into(), string("stage3_survivor_sparse_v2".into()));
    let mut required_cells: Vec<_> = allowed_cells.iter().copied().collect();
    required_cells.sort_unstable();
    object.insert("required_parameter_mn_cells".into(), serde_json::to_value(required_cells)?);
    object.insert("n_required_regimes".into(), serde_json::Value::from(provenance.sparse_grid.n_regimes));
    object.insert("survivor_grid_file".into(), string(provenance.survivor_grid_file.display().to_string()));
    object.insert("survivor_grid_sha256".into(), string(provenance.survivor_grid_sha256));
    object.insert("dataset".into(), string(provenance.dataset));
    object.insert("run_id".into(), string(provenance.run_id));
    object.insert(
        "hla_environment_representation".into(),
        string(provenance.hla_environment_representation),
    );
    object.insert("pn_hla_scope".into(), string(provenance.pn_hla_scope));
    object.insert(
        "max_num_ps_values_log2".into(),
        serde_json::Value::from(provenance.max_num_ps_values_log2),
    );
    object.insert(
        "max_num_ps_values".into(),
        serde_json::Value::from(provenance.max_num_ps_values),
    );
    object.insert("common_fingerprint".into(), string(provenance.common_fingerprint));
    object.insert("qpi_mode_fingerprint".into(), string(provenance.qpi_mode_fingerprint));
    object.insert("pn_mode_fingerprint".into(), string(provenance.pn_mode_fingerprint));
    object.insert(
        "query_input_file".into(),
        string(provenance.query_input_file.display().to_string()),
    );
    object.insert("query_input_sha256".into(), string(provenance.query_input_sha256));
    object.insert(
        "mapping_file".into(),
        string(provenance.mapping_file.display().to_string()),
    );
    object.insert("mapping_sha256".into(), string(provenance.mapping_sha256));
    object.insert(
        "parameter_file".into(),
        string(provenance.parameter_file.display().to_string()),
    );
    object.insert(
        "parameter_file_sha256".into(),
        string(provenance.parameter_file_sha256),
    );
    object.insert(
        "mn_tuples_file".into(),
        string(provenance.mn_tuples_file.display().to_string()),
    );
    object.insert(
        "mn_tuples_file_sha256".into(),
        string(provenance.mn_tuples_file_sha256),
    );
    object.insert(
        "run_manifest".into(),
        string(provenance.run_manifest.display().to_string()),
    );
    object.insert(
        "qpi_manifest".into(),
        string(provenance.qpi_manifest.display().to_string()),
    );
    object.insert(
        "pn_manifest".into(),
        string(provenance.pn_manifest.display().to_string()),
    );
    object.insert(
        "n_observations".into(),
        serde_json::Value::from(all_observations.len()),
    );
    object.insert(
        "n_tensor_rows".into(),
        serde_json::Value::from(all_entries.len()),
    );
    serde_json::to_writer_pretty(BufWriter::new(metadata_file), &metadata)
        .context("Failed to write metadata JSON")?;

    info!("Assembly complete!");
    info!("  Observations: {:?}", obs_path);
    info!("  F tensor:     {:?}", args.output);
    info!("  Metadata:     {:?}", metadata_path);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_components_reject_corruption_and_accept_zero() {
        let path = Path::new("tensor-input.csv");
        assert_eq!(parse_f32_strict("0", "q_value", path).unwrap(), 0.0);
        assert_eq!(parse_f32_strict("0.25", "q_value", path).unwrap(), 0.25);
        for value in ["", "bad", "NaN", "inf", "-0.1"] {
            let error = format!("{:#}", parse_f32_strict(value, "q_value", path).unwrap_err());
            assert!(error.contains("q_value") && error.contains("tensor-input.csv"));
        }
    }


    fn mapping_record(
        patient_id: &str,
        env_id: u32,
        long_peptide: &str,
        nmer: &str,
    ) -> MappingRecord {
        MappingRecord {
            patient_id: patient_id.to_string(),
            env_id,
            long_peptide: long_peptide.to_string(),
            nmer: nmer.to_string(),
            gene: "GENE".to_string(),
            cancer_type: "COVID".to_string(),
        }
    }

    fn observation(patient_id: &str) -> Observation {
        Observation {
            peptide: "PEPTIDEAA".to_string(),
            hla: "A0101".to_string(),
            env_id: 7,
            patient_id: patient_id.to_string(),
            label: 0,
            gene: "GENE".to_string(),
            cancer_type: "COVID".to_string(),
            wt_mt_group_id: format!("{patient_id}_LONGPEPTIDE"),
        }
    }

    #[test]
    fn observations_follow_exact_query_hlas_and_skip_unrostered_mapping_rows() {
        let mapping = vec![
            mapping_record("P1", 7, "LONG_A", "peptideaa"),
            // A second long-peptide context for the same scoreable unit must
            // not duplicate its tensor observation.
            mapping_record("P1", 7, "LONG_B", "PEPTIDEAA"),
            mapping_record("P1", 7, "FILTERED", "FILTEREDAA"),
        ];
        let mut roster = QueryRoster::new();
        roster.insert(
            ("P1".to_string(), 7, "PEPTIDEAA".to_string()),
            HashSet::from(["A0101".to_string(), "B0702".to_string()]),
        );

        let observations = build_observations(&mapping, &roster).unwrap();

        assert_eq!(observations.len(), 2);
        assert!(observations.iter().all(|obs| obs.peptide == "PEPTIDEAA"));
        assert_eq!(
            observations
                .iter()
                .map(|obs| obs.hla.as_str())
                .collect::<HashSet<_>>(),
            HashSet::from(["A0101", "B0702"]),
        );
    }

    #[test]
    fn observation_index_preserves_patients_that_share_a_tensor_key() {
        let observations = vec![observation("P1"), observation("P2")];
        let index = build_observation_index(&observations);

        assert_eq!(index.len(), 1);
        assert_eq!(
            index.get(&("PEPTIDEAA".to_string(), "A0101".to_string(), 7)),
            Some(&vec![0, 1]),
        );
    }

    #[test]
    fn tensor_validation_requires_every_unique_grid_cell() {
        let observations = vec![observation("P1")];
        let complete = vec![
            (0, 0, 0, 0.1, 0.2, 0.3, 0.4),
            (0, 0, 1, 0.1, 0.2, 0.3, 0.4),
            (0, 1, 0, 0.1, 0.2, 0.3, 0.4),
            (0, 1, 1, 0.1, 0.2, 0.3, 0.4),
        ];

        validate_complete_tensor(&complete, &observations, &HashSet::from([(0,0),(0,1),(1,0),(1,1)])).unwrap();

        let missing_error =
            validate_complete_tensor(&complete[..3], &observations, &HashSet::from([(0,0),(0,1),(1,0),(1,1)])).unwrap_err();
        assert!(missing_error.to_string().contains("coverage is incomplete"));

        let mut duplicate = complete.clone();
        duplicate.push(complete[0]);
        let duplicate_error =
            validate_complete_tensor(&duplicate, &observations, &HashSet::from([(0,0),(0,1),(1,0),(1,1)])).unwrap_err();
        assert!(duplicate_error
            .to_string()
            .contains("Duplicate tensor cell"));
    }
}
