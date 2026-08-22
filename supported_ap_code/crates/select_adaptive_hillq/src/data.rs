//! Full-roster transfer loading plus authoritative endpoint and label-contract
//! validation.
//!
//! Each task reconstructs the exact 9--12-mer `(nmer, normalized HLA)` roster.
//! Scoreable rows must resolve to the tau table, floor rows must not resolve,
//! and exact duplicate biological candidates are removed before aggregation.
//!
//! The Rust-native transfer package is consumed directly and its package
//! manifest is validated by the CLI before any task is loaded.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::error::{Result, SelectionError};
use crate::numeric::{FLOOR, stable_lse};

const BASELINES: [&str; 2] = ["max", "logsumexp"];

/// One labeled long-peptide endpoint.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub patient_id: String,
    pub mutation: String,
    pub long_peptide: String,
    pub label: i64,
}

impl Endpoint {
    pub fn key(&self) -> (String, String, String) {
        (
            self.patient_id.clone(),
            self.mutation.clone(),
            self.long_peptide.clone(),
        )
    }
}

/// A loaded (cohort, model, branch) task.
#[derive(Debug, Clone)]
pub struct Task {
    pub cohort: String,
    pub model: String,
    pub branch: String,
    pub endpoints: Vec<Endpoint>,
    pub raw_candidates: Vec<Vec<f64>>,
    pub source_files: Vec<PathBuf>,
    pub variant_column: String,
}

impl Task {
    pub fn task_id(&self) -> String {
        format!("{}__{}__{}", self.cohort, self.model, self.branch)
    }
    pub fn labels(&self) -> Vec<i8> {
        self.endpoints.iter().map(|e| e.label as i8).collect()
    }
}

/// Baseline-reconstruction audit for one task.
#[derive(Debug, Clone)]
pub struct Audit {
    pub task_id: String,
    pub source_variant_column: String,
    pub n_endpoints: usize,
    pub n_positive: usize,
    pub n_patients: usize,
    pub n_unique_candidates: usize,
    pub n_scoreable_candidates: usize,
    pub n_floor_candidates: usize,
    pub n_exact_duplicates_removed: usize,
    pub max_reconstruction_max_abs_error: f64,
    pub logsumexp_reconstruction_max_abs_error: f64,
    /// Populated by the PR selector after alignment to the authoritative
    /// directed-round-robin bundle's model-matched maximum.
    pub reference_system_id: Option<String>,
    pub reference_score_vector_hash: Option<String>,
    pub bundle_max_reconstruction_max_abs_error: Option<f64>,
}

/// One authoritative bundle endpoint.
#[derive(Debug, Clone)]
pub struct AuthEndpoint {
    pub endpoint_id: String,
    pub patient_id: String,
    pub mutation: String,
    pub long_peptide: String,
    pub label: i64,
}

fn io_err(path: &Path, source: std::io::Error) -> SelectionError {
    SelectionError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn csv_err(path: &Path, source: csv::Error) -> SelectionError {
    SelectionError::Csv {
        path: path.to_path_buf(),
        source,
    }
}

fn read_records(path: &Path) -> Result<(csv::StringRecord, Vec<csv::StringRecord>)> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_path(path)
        .map_err(|e| csv_err(path, e))?;
    let headers = reader.headers().map_err(|e| csv_err(path, e))?.clone();
    let mut records = Vec::new();
    for record in reader.records() {
        records.push(record.map_err(|e| csv_err(path, e))?);
    }
    Ok((headers, records))
}

fn column(headers: &csv::StringRecord, name: &str, path: &Path) -> Result<usize> {
    headers.iter().position(|h| h == name).ok_or_else(|| {
        SelectionError::msg(format!("missing column '{name}' in {}", path.display()))
    })
}

fn optional_column(headers: &csv::StringRecord, name: &str) -> Option<usize> {
    headers.iter().position(|h| h == name)
}

fn field(record: &csv::StringRecord, index: usize) -> &str {
    record.get(index).unwrap_or("")
}

fn parse_i64(value: &str, path: &Path) -> Result<i64> {
    value
        .trim()
        .parse::<i64>()
        .or_else(|_| value.trim().parse::<f64>().map(|f| f as i64))
        .map_err(|_| {
            SelectionError::msg(format!(
                "cannot parse integer '{value}' in {}",
                path.display()
            ))
        })
}

fn parse_f64(value: &str, path: &Path) -> Result<f64> {
    let parsed = value.trim().parse::<f64>().map_err(|_| {
        SelectionError::msg(format!(
            "cannot parse float '{value}' in {}",
            path.display()
        ))
    })?;
    if !parsed.is_finite() {
        return Err(SelectionError::msg(format!(
            "nonfinite float '{value}' in {}",
            path.display()
        )));
    }
    Ok(parsed)
}

/// Mutation normalization: absent/empty -> "".
fn norm_mutation(record: &csv::StringRecord, index: Option<usize>) -> String {
    match index {
        Some(i) => field(record, i).to_string(),
        None => String::new(),
    }
}

fn norm_hla(value: &str) -> String {
    let upper = value.trim().to_ascii_uppercase();
    upper
        .strip_prefix("HLA-")
        .unwrap_or(&upper)
        .replace(['*', ':'], "")
}

fn resolve_mapping_path(
    source_root: &Path,
    summary_path: &Path,
    recorded: &Path,
) -> Result<PathBuf> {
    let direct = if recorded.is_absolute() {
        recorded.to_path_buf()
    } else {
        summary_path.parent().unwrap_or(source_root).join(recorded)
    };
    if direct.is_file() {
        return Ok(direct);
    }

    if let (Some(production_root), Some(bundle_name), Some(file_name)) = (
        source_root.parent(),
        recorded.parent().and_then(Path::file_name),
        recorded.file_name(),
    ) {
        let relocated = production_root.join(bundle_name).join(file_name);
        if relocated.is_file() {
            return Ok(relocated);
        }
    }

    Err(SelectionError::msg(format!(
        "transfer mapping path no longer resolves: {}",
        recorded.display()
    )))
}

struct BaselineRow {
    patient_id: String,
    mutation: String,
    long_peptide: String,
    label: i64,
    score: f64,
}

/// Load one task's endpoints, candidate rosters, and baseline audit.
pub fn load_task(
    source_root: &Path,
    cohort: &str,
    model: &str,
    branch: &str,
) -> Result<(Task, Audit)> {
    let transfer = source_root.join(cohort).join(model).join(branch);
    let prediction_path = transfer.join("long_peptide_predictions.csv");
    let tau_path = transfer.join("target_tau_selection_by_observation.csv");
    let summary_path = transfer.join("summary.json");

    let summary_text =
        std::fs::read_to_string(&summary_path).map_err(|e| io_err(&summary_path, e))?;
    let summary: serde_json::Value =
        serde_json::from_str(&summary_text).map_err(|e| SelectionError::Json {
            path: summary_path.clone(),
            source: e,
        })?;
    let raw_mapping_path = PathBuf::from(summary["mapping"].as_str().ok_or_else(|| {
        SelectionError::msg(format!(
            "summary missing 'mapping' in {}",
            summary_path.display()
        ))
    })?);
    let mapping_path = resolve_mapping_path(source_root, &summary_path, &raw_mapping_path)?;

    // --- predictions: endpoint roster + baseline scores ---
    let (headers, records) = read_records(&prediction_path)?;
    let patient_col = column(&headers, "patient_id", &prediction_path)?;
    let mutation_col = optional_column(&headers, "mutation");
    let peptide_col = column(&headers, "long_peptide", &prediction_path)?;
    let variant_name = "l2_variant";
    let variant_col = column(&headers, variant_name, &prediction_path)?;
    let score_col = column(&headers, "score", &prediction_path)?;
    let label_col = column(&headers, "label", &prediction_path)?;

    let mut endpoint_frame: Option<Vec<Endpoint>> = None;
    let mut baseline_scores: HashMap<&str, Vec<f64>> = HashMap::new();

    for baseline in BASELINES {
        let mut rows: Vec<BaselineRow> = Vec::new();
        for record in &records {
            if field(record, variant_col) != baseline {
                continue;
            }
            rows.push(BaselineRow {
                patient_id: field(record, patient_col).to_string(),
                mutation: norm_mutation(record, mutation_col),
                long_peptide: field(record, peptide_col).to_string(),
                label: parse_i64(field(record, label_col), &prediction_path)?,
                score: parse_f64(field(record, score_col), &prediction_path)?,
            });
        }
        rows.sort_by(|a, b| {
            a.patient_id
                .cmp(&b.patient_id)
                .then_with(|| a.mutation.cmp(&b.mutation))
                .then_with(|| a.long_peptide.cmp(&b.long_peptide))
        });
        if rows.windows(2).any(|pair| {
            pair[0].patient_id == pair[1].patient_id
                && pair[0].mutation == pair[1].mutation
                && pair[0].long_peptide == pair[1].long_peptide
        }) {
            return Err(SelectionError::msg(format!(
                "duplicate endpoint rows for baseline {baseline} in {}",
                prediction_path.display()
            )));
        }

        let identity: Vec<Endpoint> = rows
            .iter()
            .map(|r| Endpoint {
                patient_id: r.patient_id.clone(),
                mutation: r.mutation.clone(),
                long_peptide: r.long_peptide.clone(),
                label: r.label,
            })
            .collect();
        let scores: Vec<f64> = rows.iter().map(|r| r.score).collect();

        match &endpoint_frame {
            None => endpoint_frame = Some(identity),
            Some(existing) => {
                let matches = existing.len() == identity.len()
                    && existing.iter().zip(&identity).all(|(a, b)| {
                        a.patient_id == b.patient_id
                            && a.mutation == b.mutation
                            && a.long_peptide == b.long_peptide
                            && a.label == b.label
                    });
                if !matches {
                    return Err(SelectionError::msg(format!(
                        "endpoint roster mismatch in {} for {baseline}",
                        prediction_path.display()
                    )));
                }
            }
        }
        baseline_scores.insert(baseline, scores);
    }
    let endpoint_frame = endpoint_frame.filter(|f| !f.is_empty()).ok_or_else(|| {
        SelectionError::msg(format!("no baseline rows in {}", prediction_path.display()))
    })?;
    let endpoint_keys: HashSet<_> = endpoint_frame.iter().map(Endpoint::key).collect();

    // --- tau: candidate scores keyed by (patient, env_id, peptide, hla) ---
    let (tau_headers, tau_records) = read_records(&tau_path)?;
    let t_patient = column(&tau_headers, "patient_id", &tau_path)?;
    let t_env = column(&tau_headers, "env_id", &tau_path)?;
    let t_peptide = column(&tau_headers, "peptide", &tau_path)?;
    let t_hla = column(&tau_headers, "hla", &tau_path)?;
    let t_score = column(&tau_headers, "score", &tau_path)?;
    let t_obs = column(&tau_headers, "obs_idx", &tau_path)?;

    // many_to_one: right (tau) keys must be unique.
    let mut tau_lookup: HashMap<(String, i64, String, String), (f64, String)> = HashMap::new();
    for record in &tau_records {
        let key = (
            field(record, t_patient).to_string(),
            parse_i64(field(record, t_env), &tau_path)?,
            field(record, t_peptide).to_string(),
            norm_hla(field(record, t_hla)),
        );
        let value = (
            parse_f64(field(record, t_score), &tau_path)?,
            field(record, t_obs).to_string(),
        );
        if tau_lookup.insert(key, value).is_some() {
            return Err(SelectionError::msg(format!(
                "tau selection table is not many-to-one (duplicate key) in {}",
                tau_path.display()
            )));
        }
    }

    // --- mapping: complete scoreable/floor 9..=12-mer candidate roster ---
    let (map_headers, map_records) = read_records(&mapping_path)?;
    let m_patient = column(&map_headers, "patient_id", &mapping_path)?;
    let m_mutation = optional_column(&map_headers, "mutation");
    let m_peptide = column(&map_headers, "long_peptide", &mapping_path)?;
    let m_env = column(&map_headers, "env_id", &mapping_path)?;
    let m_nmer = column(&map_headers, "nmer", &mapping_path)?;
    let m_hla = column(&map_headers, "HLA-RE", &mapping_path)?;
    let m_status = column(&map_headers, "mapping_status", &mapping_path)?;

    #[derive(Clone)]
    struct CandidateValue {
        status: String,
        score: f64,
    }
    // Exact biological identity: endpoint + nmer + normalized HLA. Environment
    // and observation ids are provenance, not candidate identity.
    let mut seen: HashMap<(String, String, String, String, String), CandidateValue> =
        HashMap::new();
    let mut grouped: HashMap<(String, String, String), Vec<f64>> = HashMap::new();
    let mut n_scoreable = 0usize;
    let mut n_floor = 0usize;
    let mut n_duplicates = 0usize;
    for record in &map_records {
        let nmer = field(record, m_nmer);
        let len = nmer.chars().count();
        if !(9..=12).contains(&len) {
            continue;
        }
        let status = field(record, m_status).trim().to_ascii_lowercase();
        if status != "scoreable" && status != "floor" {
            return Err(SelectionError::msg(format!(
                "unsupported mapping_status '{status}' in {}; expected scoreable or floor",
                mapping_path.display()
            )));
        }
        let env_id = parse_i64(field(record, m_env), &mapping_path)?;
        let hla = norm_hla(field(record, m_hla));
        if hla.is_empty() {
            return Err(SelectionError::msg(format!(
                "blank HLA in full-roster mapping {}",
                mapping_path.display()
            )));
        }
        let join_key = (
            field(record, m_patient).to_string(),
            env_id,
            nmer.to_string(),
            hla.clone(),
        );
        let candidate_score = match (status.as_str(), tau_lookup.get(&join_key)) {
            ("scoreable", Some((score, _obs_idx))) => *score,
            ("scoreable", None) => {
                return Err(SelectionError::msg(format!(
                    "scoreable mapping row has no tau score in {}: {:?}",
                    mapping_path.display(),
                    join_key
                )));
            }
            ("floor", None) => FLOOR,
            ("floor", Some(_)) => {
                return Err(SelectionError::msg(format!(
                    "floor mapping row unexpectedly resolves to a tau score in {}: {:?}",
                    mapping_path.display(),
                    join_key
                )));
            }
            _ => unreachable!(),
        };
        let patient = field(record, m_patient).to_string();
        let mutation = norm_mutation(record, m_mutation);
        let long_peptide = field(record, m_peptide).to_string();
        let endpoint_key = (patient.clone(), mutation.clone(), long_peptide.clone());
        if !endpoint_keys.contains(&endpoint_key) {
            return Err(SelectionError::msg(format!(
                "mapping contains a candidate outside the transfer endpoint roster in {}: {:?}",
                mapping_path.display(),
                endpoint_key
            )));
        }
        let dedup_key = (
            patient.clone(),
            mutation.clone(),
            long_peptide.clone(),
            nmer.to_string(),
            hla,
        );
        let value = CandidateValue {
            status: status.clone(),
            score: candidate_score,
        };
        if let Some(previous) = seen.get(&dedup_key) {
            if previous.status != value.status || previous.score != value.score {
                return Err(SelectionError::msg(format!(
                    "conflicting duplicate biological candidate in {}: {:?}",
                    mapping_path.display(),
                    dedup_key
                )));
            }
            n_duplicates += 1;
            continue;
        }
        seen.insert(dedup_key, value);
        if status == "scoreable" {
            n_scoreable += 1;
        } else {
            n_floor += 1;
        }
        grouped
            .entry(endpoint_key)
            .or_default()
            .push(candidate_score);
    }

    // --- reconstruct rosters in endpoint order + baseline audit ---
    let empty: Vec<f64> = Vec::new();
    let mut raw_candidates: Vec<Vec<f64>> = Vec::with_capacity(endpoint_frame.len());
    let mut max_error = 0.0_f64;
    let mut lse_error = 0.0_f64;
    let committed_max = &baseline_scores["max"];
    let committed_lse = &baseline_scores["logsumexp"];
    for (i, endpoint) in endpoint_frame.iter().enumerate() {
        let raw = grouped.get(&endpoint.key()).unwrap_or(&empty).clone();
        let reconstructed_max = if raw.is_empty() {
            FLOOR
        } else {
            raw.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        };
        let reconstructed_lse = stable_lse(&raw);
        max_error = max_error.max((reconstructed_max - committed_max[i]).abs());
        lse_error = lse_error.max((reconstructed_lse - committed_lse[i]).abs());
        raw_candidates.push(raw);
    }
    if max_error > 3e-5 || lse_error > 3e-5 {
        return Err(SelectionError::msg(format!(
            "baseline reconstruction failed for {cohort}/{model}/{branch}: max={max_error}, logsumexp={lse_error}"
        )));
    }

    let n_positive = endpoint_frame.iter().filter(|e| e.label == 1).count();
    let mut patients: Vec<&str> = endpoint_frame
        .iter()
        .map(|e| e.patient_id.as_str())
        .collect();
    patients.sort_unstable();
    patients.dedup();

    let task = Task {
        cohort: cohort.to_string(),
        model: model.to_string(),
        branch: branch.to_string(),
        endpoints: endpoint_frame.clone(),
        raw_candidates,
        source_files: vec![prediction_path, tau_path, summary_path, mapping_path],
        variant_column: variant_name.to_string(),
    };
    let audit = Audit {
        task_id: task.task_id(),
        source_variant_column: variant_name.to_string(),
        n_endpoints: endpoint_frame.len(),
        n_positive,
        n_patients: patients.len(),
        n_unique_candidates: n_scoreable + n_floor,
        n_scoreable_candidates: n_scoreable,
        n_floor_candidates: n_floor,
        n_exact_duplicates_removed: n_duplicates,
        max_reconstruction_max_abs_error: max_error,
        logsumexp_reconstruction_max_abs_error: lse_error,
        reference_system_id: None,
        reference_score_vector_hash: None,
        bundle_max_reconstruction_max_abs_error: None,
    };
    Ok((task, audit))
}

/// Port of `authoritative_endpoint_table`: merge the bundle's endpoints.csv
/// with endpoint_identities.csv (inner, one-to-one on endpoint_id).
pub fn authoritative_endpoint_table(bundle_root: &Path, cohort: &str) -> Result<Vec<AuthEndpoint>> {
    let evaluation = bundle_root.join("pr").join("evaluations").join(cohort);
    let endpoints_path = evaluation.join("endpoints.csv");
    let identities_path = evaluation.join("endpoint_identities.csv");
    if !endpoints_path.is_file() || !identities_path.is_file() {
        return Err(SelectionError::msg(format!(
            "missing authoritative endpoint files for {cohort}"
        )));
    }

    let (ep_headers, ep_records) = read_records(&endpoints_path)?;
    let ep_id = column(&ep_headers, "endpoint_id", &endpoints_path)?;
    let ep_label = column(&ep_headers, "label", &endpoints_path)?;
    let mut labels: HashMap<String, i64> = HashMap::new();
    for record in &ep_records {
        if labels
            .insert(
                field(record, ep_id).to_string(),
                parse_i64(field(record, ep_label), &endpoints_path)?,
            )
            .is_some()
        {
            return Err(SelectionError::msg(format!(
                "duplicate endpoint_id in {}",
                endpoints_path.display()
            )));
        }
    }

    let (id_headers, id_records) = read_records(&identities_path)?;
    let i_id = column(&id_headers, "endpoint_id", &identities_path)?;
    let i_patient = column(&id_headers, "patient_id", &identities_path)?;
    let i_mutation = optional_column(&id_headers, "mutation");
    let i_peptide = column(&id_headers, "long_peptide", &identities_path)?;

    let mut out = Vec::new();
    let mut seen_identities: HashMap<String, ()> = HashMap::new();
    let mut label_classes: [bool; 2] = [false, false];
    for record in &id_records {
        let endpoint_id = field(record, i_id).to_string();
        if seen_identities.insert(endpoint_id.clone(), ()).is_some() {
            return Err(SelectionError::msg(format!(
                "duplicate endpoint_id in {}",
                identities_path.display()
            )));
        }
        let Some(&label) = labels.get(&endpoint_id) else {
            continue; // inner join
        };
        if label == 0 {
            label_classes[0] = true;
        } else if label == 1 {
            label_classes[1] = true;
        }
        out.push(AuthEndpoint {
            endpoint_id,
            patient_id: field(record, i_patient).to_string(),
            mutation: norm_mutation(record, i_mutation),
            long_peptide: field(record, i_peptide).to_string(),
            label,
        });
    }
    if out.len() != ep_records.len() || !(label_classes[0] && label_classes[1]) {
        return Err(SelectionError::msg(format!(
            "invalid authoritative endpoint roster for {cohort}"
        )));
    }
    Ok(out)
}

/// Port of `validate_task_labels`: check every task endpoint is present with a
/// matching label, and return endpoint ids in task order.
pub fn validate_task_labels(task: &Task, authority: &[AuthEndpoint]) -> Result<Vec<String>> {
    let mut by_key: HashMap<(String, String, String), (String, i64)> = HashMap::new();
    for endpoint in authority {
        if by_key
            .insert(
                (
                    endpoint.patient_id.clone(),
                    endpoint.mutation.clone(),
                    endpoint.long_peptide.clone(),
                ),
                (endpoint.endpoint_id.clone(), endpoint.label),
            )
            .is_some()
        {
            return Err(SelectionError::msg(
                "authoritative bundle contains duplicate biological endpoint identities",
            ));
        }
    }
    let mut endpoint_ids = Vec::with_capacity(task.endpoints.len());
    for endpoint in &task.endpoints {
        let key = endpoint.key();
        let Some((endpoint_id, label)) = by_key.get(&key) else {
            return Err(SelectionError::msg(format!(
                "task endpoint is absent from authoritative bundle: {}/{:?}",
                task.task_id(),
                key
            )));
        };
        if *label != endpoint.label {
            return Err(SelectionError::msg(format!(
                "task label differs from authoritative bundle: {}/{:?}",
                task.task_id(),
                key
            )));
        }
        endpoint_ids.push(endpoint_id.clone());
    }
    let mut unique = endpoint_ids.clone();
    unique.sort();
    unique.dedup();
    if endpoint_ids.len() != by_key.len() || unique.len() != endpoint_ids.len() {
        return Err(SelectionError::msg(format!(
            "task endpoint roster differs from authoritative bundle: {}",
            task.task_id()
        )));
    }
    Ok(endpoint_ids)
}

/// Port of `validate_label_contract`: check the frozen COVID label contracts in
/// both branch bundles and return them for the manifest.
pub fn validate_label_contract(bundle_root: &Path) -> Result<serde_json::Value> {
    let mut specs: Vec<(serde_json::Value, serde_json::Value)> = Vec::new();
    for branch in ["pr", "roc"] {
        let path = bundle_root.join(branch).join("tournament_spec.json");
        let text = std::fs::read_to_string(&path).map_err(|e| io_err(&path, e))?;
        let spec: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| SelectionError::Json {
                path: path.clone(),
                source: e,
            })?;
        let contract = &spec["annotations"]["scientific_contract"];
        let spike = contract["covid_spike_label_specification"].clone();
        let nonspike = contract["evaluation_label_contracts"]["covid_nonspike"].clone();

        let spike_ok = spike["id"] == serde_json::json!("threshold_zero")
            && spike["response_field"] == serde_json::json!("cd8_IFNg_dmso_adj")
            && spike["comparison_operator"] == serde_json::json!(">")
            && spike["threshold"].as_f64() == Some(0.0);
        let nonspike_ok = nonspike["response_field"] == serde_json::json!("cd8_TNFa_IFNg_dmso_adj")
            && nonspike["comparison_operator"] == serde_json::json!(">")
            && nonspike["threshold"].as_f64() == Some(0.0);
        if !spike_ok || !nonspike_ok {
            return Err(SelectionError::msg(format!(
                "unexpected COVID label contract in {}",
                path.display()
            )));
        }
        specs.push((spike, nonspike));
    }
    if specs[0].0 != specs[1].0 || specs[0].1 != specs[1].1 {
        return Err(SelectionError::msg(
            "PR and ROC bundles disagree on COVID label contracts",
        ));
    }
    Ok(serde_json::json!({
        "covid_spike": specs[0].0,
        "covid_nonspike": specs[0].1,
    }))
}
