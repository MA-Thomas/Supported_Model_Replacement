// src/bin/assemble_tensor_longpep.rs
//
// Assembles F tensor for the PDAC long-peptide evaluation context.
//
// Differences from assemble_tensor.rs:
//
//   - Labels are at the long-peptide level, not the n-mer level.
//     The input is a single mapping CSV (not separate MT/WT label CSVs):
//
//       patient_id, env_id, long_peptide, nmer, long_peptide_label, gene, cancer_type
//
//     where `long_peptide_label` is 1 (immunogenic) or 0 (non-immunogenic)
//     for the long peptide, repeated on every n-mer row belonging to it.
//     This binary validates that the label column exists, but the label written
//     into the observations parquet is still a placeholder (0). The real
//     long-peptide label is read downstream by evaluate_long_peptides.py from
//     the mapping CSV.
//
//   - Observations are deduplicated scoreable rows at the level
//
//       patient_id × env_id × nmer × HLA
//
//     not at the full contextual long-peptide row level. If the same n-mer
//     appears in multiple long peptides for the same patient/environment, it is
//     scored once and expanded back to all long-peptide contexts downstream by
//     evaluate_long_peptides.py when scored observations are merged back to the
//     full mapping CSV.
//
//   - Tensor PN/Q/Pi rows are keyed only by (nmer, HLA, env_id), because the
//     Stage 2 CSVs do not contain patient_id. Therefore the observation index is
//     a multimap:
//
//       (nmer, HLA, env_id) -> Vec<obs_idx>
//
//     If the same env_id/nmer/HLA scoreable unit is present for multiple
//     patients, the same tensor entry is emitted for each corresponding
//     patient-specific observation. This preserves correctness for PDAC even if
//     env_id is not one-to-one with patient_id.
//
//   - The filtered Stage 2 query CSV is the authoritative HLA-specific candidate
//     roster. Observations are built only from exact joins between mapping rows
//     and query rows on (patient_id, env_id, nmer/peptide), retaining the query
//     row's HLA. Missing PN files therefore cannot silently shrink the expected
//     observation universe, and unsupported nmer × HLA crosses are never made.
//
//   - PN CSV columns: peptide, env_id, M, N, p_pos, p_neg.
//     env_id is read from each CSV row.
//
//   - Q and Pi values are joined from results_qpi_env_id_* files, identical
//     to assemble_tensor.rs. Both are mandatory for every PN row that maps to
//     an observation. Missing, malformed, or non-finite values abort assembly;
//     they are never replaced with zero.
//
//   - Every observation must contain exactly the manifest-required sparse
//     parameter/MN cells. Missing rows and duplicate PN tensor cells abort assembly before
//     any output tensor is written.
//
//   - wt_mt_group_id is a representative placeholder context for the deduped
//     scoreable row. Long-peptide grouping and labels are reconstructed
//     downstream from the mapping CSV.
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
use log::{debug, info};
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

    /// Dataset recorded by the Stage 2 contract (PDAC).
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

    /// Mapping CSV with required columns:
    ///   patient_id, env_id, long_peptide, nmer, long_peptide_label, gene, cancer_type
    #[arg(long)]
    mapping: PathBuf,

    /// Filtered Stage 2 query CSV with required columns:
    ///   peptide, HLA-RE, PatientID, env_id
    /// This file defines the exact HLA-specific candidate roster.
    #[arg(long)]
    query_peptides: PathBuf,

    /// Output Parquet file path (observations written to same stem + .observations.parquet)
    #[arg(long)]
    output: PathBuf,

    /// Number of parallel threads (default: all available)
    #[arg(long)]
    threads: Option<usize>,

}


// =============================================================================
// Mapping CSV record
// =============================================================================
//
// Parsed manually by header name so that column errors are explicit and the
// assembler remains robust to harmless extra columns. For PDAC, all seven core
// columns are required.

#[derive(Debug)]
struct MappingRecord {
    patient_id: String,
    env_id: u32,
    long_peptide: String,
    nmer: String,
    long_peptide_label: u8,
    gene: String,
    cancer_type: String,
}


// =============================================================================
// Load mapping CSV
// =============================================================================

fn load_mapping(path: &Path) -> Result<Vec<MappingRecord>> {
    info!("Loading mapping file from {:?}", path);

    let file = File::open(path)
        .with_context(|| format!("Failed to open mapping file: {:?}", path))?;

    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_reader(BufReader::new(file));

    let headers = reader
        .headers()
        .with_context(|| format!("Failed to read headers from mapping file: {:?}", path))?
        .clone();
    let hidx = HeaderIndex::from_headers(&headers);

    let patient_id_idx          = hidx.get_required("patient_id")?;
    let env_id_idx              = hidx.get_required("env_id")?;
    let long_peptide_idx        = hidx.get_required("long_peptide")?;
    let nmer_idx                = hidx.get_required("nmer")?;
    let long_peptide_label_idx  = hidx.get_required("long_peptide_label")?;
    let gene_idx                = hidx.get_required("gene")?;
    let cancer_type_idx         = hidx.get_required("cancer_type")?;

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

        let label_raw = record.get(long_peptide_label_idx).unwrap_or("").trim();
        let long_peptide_label: u8 = label_raw.parse().with_context(|| {
            format!(
                "Failed to parse long_peptide_label '{}' as 0/1 integer (mapping data row {})",
                label_raw,
                row_no + 1
            )
        })?;
        if long_peptide_label > 1 {
            bail!(
                "Invalid long_peptide_label={} at mapping data row {}; expected 0 or 1",
                long_peptide_label,
                row_no + 1
            );
        }

        records.push(MappingRecord {
            patient_id:          record.get(patient_id_idx).unwrap_or("").trim().to_string(),
            env_id,
            long_peptide:        record.get(long_peptide_idx).unwrap_or("").trim().to_string(),
            nmer:                record.get(nmer_idx).unwrap_or("").trim().to_string(),
            long_peptide_label,
            gene:                record.get(gene_idx).unwrap_or("").trim().to_string(),
            cancer_type:         record.get(cancer_type_idx).unwrap_or("").trim().to_string(),
        });
    }

    info!(
        "Loaded {} mapping rows ({} unique n-mers, {} unique env_ids, {} positive long-peptide rows)",
        records.len(),
        records.iter().map(|r| r.nmer.as_str()).collect::<HashSet<_>>().len(),
        records.iter().map(|r| r.env_id).collect::<HashSet<_>>().len(),
        records.iter().filter(|r| r.long_peptide_label == 1).count(),
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
        .with_context(|| format!("Failed to read headers from filtered query file: {:?}", path))?
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
            format!("Failed to read filtered query row {} from {:?}", row_no + 2, path)
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
        .to_string()
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
    let parsed = value.trim().parse::<f32>().with_context(|| {
        format!(
            "Invalid or missing {field} value {:?} in {}",
            value,
            path.display()
        )
    })?;
    if !parsed.is_finite() || parsed < 0.0 {
        bail!(
            "Expected finite nonnegative {field}, got {parsed} in {}",
            path.display()
        );
    }
    Ok(parsed)
}

fn require_q_pi(
    peptide: &str,
    hla: &str,
    env_id: u32,
    q_store: &QStore,
    pi_store: &PiStore,
    file_path: &Path,
) -> Result<(f32, f32)> {
    let q_value = q_store
        .get(&(peptide.to_string(), hla.to_string(), env_id))
        .copied()
        .with_context(|| {
            format!(
                "Missing required q_value for peptide='{peptide}', hla='{hla}', env_id={env_id} while processing {}",
                file_path.display()
            )
        })?;
    let pi_value = pi_store
        .get(&(peptide.to_string(), hla.to_string()))
        .copied()
        .with_context(|| {
            format!(
                "Missing required pi_value for peptide='{peptide}', hla='{hla}' while processing {}",
                file_path.display()
            )
        })?;
    Ok((q_value, pi_value))
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
                None => { pb.inc(1); return Ok(local); }
            };

            let file = File::open(file_path)
                .with_context(|| format!("Failed to open Q file: {:?}", file_path))?;
            let mut reader = ReaderBuilder::new()
                .has_headers(true)
                .trim(csv::Trim::All)
                .from_reader(BufReader::new(file));

            let headers = reader.headers()
                .with_context(|| format!("Failed to read headers from {:?}", file_path))?
                .clone();
            let hidx = HeaderIndex::from_headers(&headers);
            let peptide_idx = hidx.get_required("peptide")?;
            let env_id_idx  = hidx.get_required("env_id")?;
            let q_idx       = hidx.get_required("q_value")?;

            for result in reader.records() {
                let record = result
                    .with_context(|| format!("Failed to read record from {:?}", file_path))?;

                let peptide = record.get(peptide_idx).unwrap_or("").trim().to_string();
                if peptide.is_empty() { continue; }

                let env_id: u32 = match record.get(env_id_idx).unwrap_or("").trim().parse() {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let q_value = parse_f32_strict(
                    record.get(q_idx).unwrap_or(""),
                    "q_value",
                    file_path,
                )?;
                let key = (peptide, hla.clone(), env_id);

                if let Some(&existing) = local.get(&key) {
                    if (existing - q_value).abs() > 1e-6 {
                        bail!(
                            "Conflicting q_value in {:?}: existing={} new={}",
                            file_path, existing, q_value
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
                    bail!("Conflicting q_value during merge: existing={} new={}", existing, q);
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
                None => { pb.inc(1); return Ok(local); }
            };

            let file = File::open(file_path)
                .with_context(|| format!("Failed to open Pi file: {:?}", file_path))?;
            let mut reader = ReaderBuilder::new()
                .has_headers(true)
                .trim(csv::Trim::All)
                .from_reader(BufReader::new(file));

            let headers = reader.headers()
                .with_context(|| format!("Failed to read headers from {:?}", file_path))?
                .clone();
            let hidx = HeaderIndex::from_headers(&headers);
            let peptide_idx = hidx.get_required("peptide")?;
            let pi_idx       = hidx.get_required("pi_value")?;

            for result in reader.records() {
                let record = result
                    .with_context(|| format!("Failed to read record from {:?}", file_path))?;

                let peptide = record.get(peptide_idx).unwrap_or("").trim().to_string();
                if peptide.is_empty() { continue; }

                let pi_value = parse_f32_strict(
                    record.get(pi_idx).unwrap_or(""),
                    "pi_value",
                    file_path,
                )?;
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

    // Deduplicate to the scoreable row used by evaluate_long_peptides.py:
    // one score per patient_id/env_id/nmer/HLA.  Multiple long-peptide
    // contexts carrying the same n-mer for the same patient/environment are
    // expanded downstream when the mapping CSV is merged back in.
    let mut seen: HashSet<(String, u32, String, String)> = HashSet::new();

    let mut n_duplicate_contexts = 0usize;
    let mut missing_mapping_rows: Vec<String> = Vec::new();
    let mut all_long_peptides: HashSet<(String, u32, String)> = HashSet::new();
    let mut supported_long_peptides: HashSet<(String, u32, String)> = HashSet::new();

    for record in mapping {
        let long_peptide_key = (
            record.patient_id.clone(),
            record.env_id,
            record.long_peptide.clone(),
        );
        all_long_peptides.insert(long_peptide_key.clone());

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
                supported_long_peptides.insert(long_peptide_key);
                hlas
            }
            None => {
                if missing_mapping_rows.len() < 20 {
                    missing_mapping_rows.push(format!(
                        "patient={} env={} nmer={} long_peptide={}",
                        record.patient_id, record.env_id, peptide, record.long_peptide
                    ));
                }
                continue;
            }
        };

        for hla in hlas {
            let dedup_key = (
                record.patient_id.clone(),
                record.env_id,
                peptide.clone(),
                hla.clone(),
            );
            if !seen.insert(dedup_key) {
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
                // Representative context only. Downstream evaluation rebuilds
                // long-peptide groups from the full mapping CSV.
                wt_mt_group_id,
            });
        }
    }

    if !missing_mapping_rows.is_empty() {
        bail!(
            "Mapping contains rows absent from the authoritative filtered HLA-specific query roster. Examples: {}",
            missing_mapping_rows.join("; "),
        );
    }

    let unsupported_long_peptides: Vec<String> = all_long_peptides
        .difference(&supported_long_peptides)
        .take(20)
        .map(|(patient, env, peptide)| {
            format!("patient={patient} env={env} long_peptide={peptide}")
        })
        .collect();
    if !unsupported_long_peptides.is_empty() {
        bail!(
            "At least one mapped long peptide has no exact filtered HLA-specific candidate. Examples: {}",
            unsupported_long_peptides.join("; "),
        );
    }

    info!(
        "Built {} deduplicated patient/env/nmer/HLA observations from {} mapping rows and {} long peptides ({} duplicate exact candidate contexts skipped)",
        observations.len(),
        mapping.len(),
        all_long_peptides.len(),
        n_duplicate_contexts,
    );
    Ok(observations)
}


// =============================================================================
// Observation index
// =============================================================================

/// Stage 2 PN/Q/Pi rows do not carry patient_id, so tensor rows are keyed by
/// (peptide, HLA, env_id).  For PDAC this key can map to more than one
/// patient-specific observation.  We therefore return a multimap and emit the
/// same tensor value for every matching obs_idx.
type ObservationIndex = FxHashMap<(String, String, u32), Vec<usize>>;

fn build_observation_index(
    observations: &[Observation],
) -> ObservationIndex {
    let mut index: ObservationIndex = FxHashMap::default();
    for (i, obs) in observations.iter().enumerate() {
        index
            .entry((obs.peptide.clone(), obs.hla.clone(), obs.env_id))
            .or_insert_with(Vec::new)
            .push(i);
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
    info!("Scanning {} PN files to discover parameter grid...", files.len());

    let mut geometry_keys: BTreeSet<GeometryKey> = BTreeSet::new();
    let mut tau_values: BTreeSet<String> = BTreeSet::new();
    let mut mn_tuples: BTreeSet<(u32, u32)> = BTreeSet::new();

    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] Scanning: [{bar:40.cyan/blue}] {pos}/{len}")
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
                let m_col = match hidx.get_required("M") { Ok(i) => i, Err(_) => continue };
                let n_col = match hidx.get_required("N") { Ok(i) => i, Err(_) => continue };

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
        .map(|g| (g.d_pos.clone(), g.d_neg.clone(), g.steepness_pos.clone(), g.steepness_neg.clone()))
        .collect();

    let mut tau_with_numeric: Vec<(String, f32)> = tau_values
        .into_iter()
        .map(|s| {
            let value = s
                .parse::<f32>()
                .with_context(|| format!("Invalid tau_thymus value '{s}' in PN filename"))?;
            if !value.is_finite() {
                bail!("Non-finite tau_thymus value '{s}' in PN filename");
            }
            Ok((s, value))
        })
        .collect::<Result<Vec<_>>>()?;
    tau_with_numeric.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    let tau_values_str: Vec<String> = tau_with_numeric.iter().map(|(s, _)| s.clone()).collect();
    let tau_values_f32: Vec<f32>   = tau_with_numeric.iter().map(|(_, f)| *f).collect();

    let geometry_params_f32: Vec<(f32, f32, f32, f32)> = geometry_params_str
        .iter()
        .map(|(a, b, c, d)| {
            let parse = |value: &str, name: &str| -> Result<f32> {
                let parsed = value.parse::<f32>().with_context(|| {
                    format!("Invalid {name} value '{value}' in PN filename")
                })?;
                if !parsed.is_finite() {
                    bail!("Non-finite {name} value '{value}' in PN filename");
                }
                Ok(parsed)
            };
            Ok((
                parse(a, "d_pos")?,
                parse(b, "d_neg")?,
                parse(c, "steepness_pos")?,
                parse(d, "steepness_neg")?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;

    let mn_tuples_vec: Vec<(u32, u32)> = mn_tuples.into_iter().collect();
    let n_geometry = geometry_params_str.len();
    let n_tau      = tau_values_str.len();
    let n_mn       = mn_tuples_vec.len();
    let n_params   = n_geometry * n_tau;

    info!("Discovered parameter grid:");
    info!("  {} geometry settings", n_geometry);
    info!("  {} τ values: {:?}", n_tau, tau_values_str);
    info!("  {} (M, N) tuples", n_mn);
    info!("  {} geometry × {} τ = {} param combinations", n_geometry, n_tau, n_params);

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
    tau_to_idx:      FxHashMap<String, u32>,
    mn_to_idx:       FxHashMap<(u32, u32), u32>,
}

fn build_parameter_indices(metadata: &ParameterGridMetadata) -> ParameterIndices {
    let geometry_to_idx = metadata.geometry_params_str.iter().enumerate()
        .map(|(i, g)| (g.clone(), i as u32)).collect();
    let tau_to_idx = metadata.tau_values_str.iter().enumerate()
        .map(|(i, t)| (t.clone(), i as u32)).collect();
    let mn_to_idx = metadata.mn_tuples.iter().enumerate()
        .map(|(i, &mn)| (mn, i as u32)).collect();
    ParameterIndices { geometry_to_idx, tau_to_idx, mn_to_idx }
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

    let geometry_idx = *indices.geometry_to_idx.get(&geometry_key)
        .ok_or_else(|| anyhow!("Unknown geometry key in {:?}", file_path))?;
    let tau_idx = *indices.tau_to_idx.get(&file_params.tau_thymus)
        .ok_or_else(|| anyhow!("Unknown tau value in {:?}", file_path))?;
    let param_idx = geometry_idx * (n_tau as u32) + tau_idx;

    let file = File::open(file_path)
        .with_context(|| format!("Failed to open PN file: {:?}", file_path))?;
    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .trim(csv::Trim::All)
        .from_reader(BufReader::new(file));

    let headers = reader.headers()
        .with_context(|| format!("Failed to read headers from {:?}", file_path))?
        .clone();
    let hidx = HeaderIndex::from_headers(&headers);

    let peptide_idx = hidx.get_required("peptide")?;
    let env_id_idx  = hidx.get_required("env_id")?;
    let m_idx       = hidx.get_required("M")?;
    let n_idx       = hidx.get_required("N")?;
    let p_pos_idx   = hidx.get_required("p_pos")?;
    let p_neg_idx   = hidx.get_required("p_neg")?;

    let mut entries = Vec::new();
    let mut seen_tensor_cells: HashSet<(String, u32, u32, u32)> = HashSet::new();

    for result in reader.records() {
        let record = result
            .with_context(|| format!("Failed to read record from {:?}", file_path))?;

        let peptide = record.get(peptide_idx).unwrap_or("").trim().to_string();
        if peptide.is_empty() { continue; }

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
            None => { debug!("Unknown (M,N)=({},{}) in {:?}", m, n, file_path); continue; }
        };

        let p_pos = parse_f32_strict(
            record.get(p_pos_idx).unwrap_or(""),
            "p_pos",
            file_path,
        )?;
        let p_neg = parse_f32_strict(
            record.get(p_neg_idx).unwrap_or(""),
            "p_neg",
            file_path,
        )?;

        let obs_key = (peptide.clone(), file_params.hla.clone(), env_id);
        if let Some(obs_indices) = obs_index.get(&obs_key) {
            if !seen_tensor_cells.insert((peptide.clone(), env_id, m, n)) {
                bail!(
                    "Duplicate PN tensor cell for peptide='{peptide}', hla='{}', env_id={env_id}, M={m}, N={n} in {}",
                    file_params.hla,
                    file_path.display()
                );
            }
            let (q_value, pi_value) = require_q_pi(
                &peptide,
                &file_params.hla,
                env_id,
                q_store,
                pi_store,
                file_path,
            )?;
            for &obs_idx in obs_indices {
                entries.push((
                    obs_idx,
                    param_idx,
                    mn_idx,
                    q_value,
                    p_pos,
                    p_neg,
                    pi_value,
                ));
            }
        }
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
    // Load mapping CSV and authoritative filtered HLA-specific query roster
    // ------------------------------------------------------------------
    let mapping = load_mapping(&args.mapping)?;
    let query_roster = load_query_roster(&args.query_peptides)?;

    // ------------------------------------------------------------------
    // Discover PN files under results_pn_env_id_*
    // ------------------------------------------------------------------
    let mut pn_files: Vec<PathBuf> = Vec::new();

    for dir in &args.results_dirs {
        let pattern = dir.join("results_pn_env_id_*").join("*.csv");
        let pattern_str = pattern.to_string_lossy();
        info!("Searching for PN files in {}: {}", dir.display(), pattern_str);

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
    info!("Found {} total PN files across all datasets", pn_files.len());
    
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
    let q_store  = collect_q_store(&qpi_files)?;
    let pi_store = collect_pi_store(&qpi_files)?;

    if q_store.is_empty() {
        bail!("No Q values were loaded; refusing to assemble a tensor with missing Q entries");
    }
    if pi_store.is_empty() {
        bail!("No Pi values were loaded; refusing to assemble a tensor with missing Pi entries");
    }

    // ------------------------------------------------------------------
    // Build observations from exact mapping × filtered-query candidate joins.
    // The expected HLA roster never comes from result filenames.
    // ------------------------------------------------------------------
    let all_observations = build_observations(&mapping, &query_roster)?;

    if all_observations.is_empty() {
        bail!(
            "No observations could be built from the exact mapping × filtered-query roster join."
        );
    }

    let obs_index = build_observation_index(&all_observations);
    let indexed_observation_count: usize = obs_index.values().map(|v| v.len()).sum();
    info!(
        "Observation index size: {} tensor keys covering {} observations",
        obs_index.len(),
        indexed_observation_count,
    );
    if indexed_observation_count != all_observations.len() {
        bail!(
            "Internal error: observation index covers {} rows but observations table has {} rows",
            indexed_observation_count,
            all_observations.len(),
        );
    }

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

        let results: Vec<_> = chunk.par_iter()
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

        info!("  Chunk {} complete. Total entries so far: {}", chunk_idx + 1, all_entries.len());
    }

    pb.finish_with_message("File processing complete");
    info!("Collected {} F tensor entries", all_entries.len());

    let expected_rows_per_observation = allowed_cells.len();
    validate_complete_tensor(&all_entries, &all_observations, &allowed_cells)?;
    info!(
        "Validated complete tensor coverage: exactly {} rows for each of {} observations",
        expected_rows_per_observation,
        all_observations.len()
    );

    // ------------------------------------------------------------------
    // Write observations parquet (label is placeholder 0)
    // ------------------------------------------------------------------
    let obs_schema = Schema::new(vec![
        Field::new("obs_idx",        DataType::UInt32, false),
        Field::new("peptide",        DataType::Utf8,   false),
        Field::new("hla",            DataType::Utf8,   false),
        Field::new("env_id",         DataType::UInt32, false),
        Field::new("patient_id",     DataType::Utf8,   false),
        Field::new("label",          DataType::UInt8,  false),
        Field::new("gene",           DataType::Utf8,   false),
        Field::new("cancer_type",    DataType::Utf8,   false),
        Field::new("wt_mt_group_id", DataType::Utf8,   false),
    ]);

    let obs_path = args.output.with_extension("observations.parquet");
    info!("Writing observations to {:?}", obs_path);

    let obs_indices:     Vec<u32>  = (0..all_observations.len() as u32).collect();
    let peptides:        Vec<&str> = all_observations.iter().map(|o| o.peptide.as_str()).collect();
    let hlas:            Vec<&str> = all_observations.iter().map(|o| o.hla.as_str()).collect();
    let env_ids:         Vec<u32>  = all_observations.iter().map(|o| o.env_id).collect();
    let patient_ids:     Vec<&str> = all_observations.iter().map(|o| o.patient_id.as_str()).collect();
    let labels:          Vec<u8>   = all_observations.iter().map(|o| o.label).collect();
    let genes:           Vec<&str> = all_observations.iter().map(|o| o.gene.as_str()).collect();
    let cancer_types:    Vec<&str> = all_observations.iter().map(|o| o.cancer_type.as_str()).collect();
    let wt_mt_group_ids: Vec<&str> = all_observations.iter().map(|o| o.wt_mt_group_id.as_str()).collect();

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
        Field::new("obs_idx",   DataType::UInt32,  false),
        Field::new("param_idx", DataType::UInt32,  false),
        Field::new("mn_idx",    DataType::UInt32,  false),
        Field::new("q_value",   DataType::Float32, false),
        Field::new("pos_prob",  DataType::Float32, false),
        Field::new("neg_prob",  DataType::Float32, false),
        Field::new("pi_eh",     DataType::Float32, false),
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
        let obs_idx:   Vec<u32> = chunk.iter().map(|e| e.0 as u32).collect();
        let param_idx: Vec<u32> = chunk.iter().map(|e| e.1).collect();
        let mn_idx:    Vec<u32> = chunk.iter().map(|e| e.2).collect();
        let q_value:   Vec<f32> = chunk.iter().map(|e| e.3).collect();
        let pos_prob:  Vec<f32> = chunk.iter().map(|e| e.4).collect();
        let neg_prob:  Vec<f32> = chunk.iter().map(|e| e.5).collect();
        let pi_eh:     Vec<f32> = chunk.iter().map(|e| e.6).collect();

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


    #[test]
    fn numeric_inputs_never_fall_back_to_zero() {
        let path = Path::new("input.csv");
        assert_eq!(parse_f32_strict("0", "q_value", path).unwrap(), 0.0);
        assert!(parse_f32_strict("", "q_value", path).is_err());
        assert!(parse_f32_strict("not-a-number", "q_value", path).is_err());
        assert!(parse_f32_strict("NaN", "q_value", path).is_err());
    }

    #[test]
    fn missing_q_or_pi_is_always_an_error() {
        let path = Path::new("pn.csv");
        let mut q_store = QStore::default();
        let mut pi_store = PiStore::default();

        let missing_q = require_q_pi("PEPTIDE", "A0101", 7, &q_store, &pi_store, path)
            .unwrap_err()
            .to_string();
        assert!(missing_q.contains("Missing required q_value"));

        q_store.insert(("PEPTIDE".into(), "A0101".into(), 7), 0.25);
        let missing_pi = require_q_pi("PEPTIDE", "A0101", 7, &q_store, &pi_store, path)
            .unwrap_err()
            .to_string();
        assert!(missing_pi.contains("Missing required pi_value"));

        pi_store.insert(("PEPTIDE".into(), "A0101".into()), 0.5);
        assert_eq!(
            require_q_pi("PEPTIDE", "A0101", 7, &q_store, &pi_store, path).unwrap(),
            (0.25, 0.5)
        );
    }

    #[test]
    fn incomplete_or_duplicated_observation_grids_are_errors() {
        let observations = vec![Observation {
            peptide: "PEPTIDE".into(),
            hla: "A0101".into(),
            env_id: 7,
            patient_id: "P1".into(),
            label: 0,
            gene: "GENE".into(),
            cancer_type: "PDAC".into(),
            wt_mt_group_id: "P1_LONG".into(),
        }];
        let row = (0usize, 0u32, 0u32, 0.25f32, 0.5f32, 0.5f32, 0.5f32);

        assert!(validate_complete_tensor(&[], &observations, &HashSet::from([(0,0)])).is_err());
        assert!(validate_complete_tensor(&[row], &observations, &HashSet::from([(0,0)])).is_ok());
        assert!(validate_complete_tensor(&[row, row], &observations, &HashSet::from([(0,0)])).is_err());
    }

    #[test]
    fn observations_use_only_exact_filtered_hla_candidates() {
        let mapping = vec![MappingRecord {
            patient_id: "P1".into(),
            env_id: 7,
            long_peptide: "LONGPEPTIDE".into(),
            nmer: "PEPTIDEAA".into(),
            long_peptide_label: 1,
            gene: "GENE".into(),
            cancer_type: "PDAC".into(),
        }];
        let mut roster = QueryRoster::new();
        roster.insert(
            ("P1".into(), 7, "PEPTIDEAA".into()),
            HashSet::from(["A0101".into()]),
        );

        let observations = build_observations(&mapping, &roster).unwrap();
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].hla, "A0101");

        let mut unrelated_roster = QueryRoster::new();
        unrelated_roster.insert(
            ("P1".into(), 7, "OTHERPEP".into()),
            HashSet::from(["B0702".into()]),
        );
        assert!(build_observations(&mapping, &unrelated_roster).is_err());
    }
}
