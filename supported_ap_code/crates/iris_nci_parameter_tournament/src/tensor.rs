use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result, bail};
use arrow::array::{Array, Float32Array, StringArray, UInt8Array, UInt32Array};
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde_json::Value;

use crate::candidates::Candidate;
use crate::config::ModelConfig;
use crate::io::read_json;

#[derive(Clone, Debug)]
pub struct Observation {
    pub obs_idx: usize,
    pub peptide: String,
    pub hla: String,
    pub env_id: u32,
    pub patient_id: String,
    pub label: bool,
    pub gene: String,
    pub cancer_type: String,
    pub wt_mt_group_id: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct ObservationKey {
    peptide: String,
    hla: String,
    patient_id: String,
    label: bool,
    gene: String,
    cancer_type: String,
    wt_mt_group_id: String,
}

impl From<&Observation> for ObservationKey {
    fn from(value: &Observation) -> Self {
        Self {
            peptide: value.peptide.clone(),
            hla: value.hla.clone(),
            patient_id: value.patient_id.clone(),
            label: value.label,
            gene: value.gene.clone(),
            cancer_type: value.cancer_type.clone(),
            wt_mt_group_id: value.wt_mt_group_id.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Metadata {
    pub n_params: usize,
    pub n_mn: usize,
    pub n_tau: usize,
    pub geometry_params: Vec<[f64; 4]>,
    pub mn_tuples: Vec<[u32; 2]>,
}

#[derive(Clone, Debug)]
pub struct ScoredModel {
    pub observations: Vec<Observation>,
    pub scores_by_regime: BTreeMap<usize, Vec<f64>>,
}

fn number(value: &Value) -> Result<f64> {
    if let Some(value) = value.as_f64() {
        Ok(value)
    } else if let Some(value) = value.as_str() {
        Ok(value.parse()?)
    } else {
        bail!("expected number or number string")
    }
}

pub fn load_metadata(path: &Path) -> Result<Metadata> {
    let value: Value = read_json(path)?;
    let n_params = value["n_params"].as_u64().context("metadata n_params")? as usize;
    let n_mn = value["n_mn"].as_u64().context("metadata n_mn")? as usize;
    let n_tau = value["n_tau"].as_u64().context("metadata n_tau")? as usize;
    let geometry_params = value["geometry_params"]
        .as_array()
        .context("metadata geometry_params")?
        .iter()
        .map(|row| {
            let row = row.as_array().context("geometry row")?;
            if row.len() != 4 {
                bail!("geometry row has {} values, expected four", row.len());
            }
            Ok([
                number(&row[0])?,
                number(&row[1])?,
                number(&row[2])?,
                number(&row[3])?,
            ])
        })
        .collect::<Result<Vec<_>>>()?;
    let mn_tuples = value["mn_tuples"]
        .as_array()
        .context("metadata mn_tuples")?
        .iter()
        .map(|row| {
            let row = row.as_array().context("M/N row")?;
            if row.len() != 2 {
                bail!("M/N row has {} values, expected two", row.len());
            }
            Ok([
                row[0].as_u64().context("M")? as u32,
                row[1].as_u64().context("N")? as u32,
            ])
        })
        .collect::<Result<Vec<_>>>()?;
    if geometry_params.len() * n_tau != n_params
        || mn_tuples.len() != n_mn
        || n_tau == 0
        || n_mn == 0
    {
        bail!("metadata grid dimensions are inconsistent");
    }
    Ok(Metadata {
        n_params,
        n_mn,
        n_tau,
        geometry_params,
        mn_tuples,
    })
}

// Validate external column validity and scalar domains before any value() access.
// Only required columns are checked: unrelated nullable metadata may be present.
fn validate_input_batch(
    batch: &RecordBatch,
    path: &Path,
    batch_index: usize,
    required: &[&str],
) -> Result<()> {
    for &name in required {
        let index = batch.schema().index_of(name).with_context(|| {
            format!(
                "{}: batch {batch_index}, missing required column {name}",
                path.display()
            )
        })?;
        let column = batch.column(index);
        if column.null_count() != 0 {
            let row = (0..column.len())
                .find(|&row| column.is_null(row))
                .expect("nonzero null count");
            bail!(
                "{}: batch {batch_index}, row {row}, column {name}: required value is null",
                path.display()
            );
        }
        if name == "label" {
            let labels = column
                .as_any()
                .downcast_ref::<UInt8Array>()
                .with_context(|| {
                    format!(
                        "{}: batch {batch_index}, column label must be uint8",
                        path.display()
                    )
                })?;
            for (row, &label) in labels.values().iter().enumerate() {
                if label > 1 {
                    bail!(
                        "{}: batch {batch_index}, row {row}, column label: expected 0 or 1, got {label}",
                        path.display()
                    );
                }
            }
        }
        if matches!(name, "q_value" | "pos_prob" | "neg_prob" | "pi_eh") {
            let values = column
                .as_any()
                .downcast_ref::<Float32Array>()
                .with_context(|| {
                    format!(
                        "{}: batch {batch_index}, column {name} must be float32",
                        path.display()
                    )
                })?;
            for (row, &value) in values.values().iter().enumerate() {
                if !value.is_finite() || value < 0.0 {
                    bail!(
                        "{}: batch {batch_index}, row {row}, column {name}: expected finite nonnegative value, got {value}",
                        path.display()
                    );
                }
            }
        }
    }
    Ok(())
}

fn column_index(batch: &RecordBatch, name: &str) -> Result<usize> {
    batch
        .schema()
        .index_of(name)
        .with_context(|| format!("missing column {name}"))
}

fn u32_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a UInt32Array> {
    batch
        .column(column_index(batch, name)?)
        .as_any()
        .downcast_ref::<UInt32Array>()
        .with_context(|| format!("column {name} is not uint32"))
}

fn u8_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a UInt8Array> {
    batch
        .column(column_index(batch, name)?)
        .as_any()
        .downcast_ref::<UInt8Array>()
        .with_context(|| format!("column {name} is not uint8"))
}

fn f32_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a Float32Array> {
    batch
        .column(column_index(batch, name)?)
        .as_any()
        .downcast_ref::<Float32Array>()
        .with_context(|| format!("column {name} is not float32"))
}

fn string_column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a StringArray> {
    batch
        .column(column_index(batch, name)?)
        .as_any()
        .downcast_ref::<StringArray>()
        .with_context(|| format!("column {name} is not string"))
}

pub fn load_observations(path: &Path) -> Result<Vec<Observation>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut observations = Vec::new();
    for (batch_index, batch) in ParquetRecordBatchReaderBuilder::try_new(file)?
        .build()?
        .enumerate()
    {
        let batch = batch?;
        validate_input_batch(
            &batch,
            path,
            batch_index,
            &[
                "obs_idx",
                "peptide",
                "hla",
                "env_id",
                "patient_id",
                "label",
                "gene",
                "cancer_type",
                "wt_mt_group_id",
            ],
        )?;
        let obs_idx = u32_column(&batch, "obs_idx")?;
        let peptide = string_column(&batch, "peptide")?;
        let hla = string_column(&batch, "hla")?;
        let env_id = u32_column(&batch, "env_id")?;
        let patient_id = string_column(&batch, "patient_id")?;
        let label = u8_column(&batch, "label")?;
        let gene = string_column(&batch, "gene")?;
        let cancer_type = string_column(&batch, "cancer_type")?;
        let wt_mt_group_id = string_column(&batch, "wt_mt_group_id")?;
        for index in 0..batch.num_rows() {
            let label = label.value(index);
            observations.push(Observation {
                obs_idx: obs_idx.value(index) as usize,
                peptide: peptide.value(index).to_owned(),
                hla: hla.value(index).to_owned(),
                env_id: env_id.value(index),
                patient_id: patient_id.value(index).to_owned(),
                label: label == 1,
                gene: gene.value(index).to_owned(),
                cancer_type: cancer_type.value(index).to_owned(),
                wt_mt_group_id: wt_mt_group_id.value(index).to_owned(),
            });
        }
    }
    observations.sort_by_key(|row| row.obs_idx);
    for (expected, row) in observations.iter().enumerate() {
        if row.obs_idx != expected {
            bail!(
                "observations must be contiguous and zero-based; expected {expected}, got {}",
                row.obs_idx
            );
        }
    }
    if observations.is_empty()
        || observations.iter().all(|row| row.label)
        || observations.iter().all(|row| !row.label)
    {
        bail!("NCI observations must contain both classes");
    }
    Ok(observations)
}

fn load_constant_q(path: &Path, observation_count: usize) -> Result<Vec<f32>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut values = vec![None::<f32>; observation_count];
    for (batch_index, batch) in ParquetRecordBatchReaderBuilder::try_new(file)?
        .build()?
        .enumerate()
    {
        let batch = batch?;
        validate_input_batch(&batch, path, batch_index, &["obs_idx", "q_value"])?;
        let obs_idx = u32_column(&batch, "obs_idx")?;
        let q = f32_column(&batch, "q_value")?;
        for index in 0..batch.num_rows() {
            let obs = obs_idx.value(index) as usize;
            if obs >= observation_count {
                bail!(
                    "{}: batch {batch_index}, row {index}: Q source obs_idx {obs} is out of range",
                    path.display()
                );
            }
            let value = q.value(index);
            match values[obs] {
                None => values[obs] = Some(value),
                Some(previous) if previous.to_bits() != value.to_bits() => {
                    bail!(
                        "{}: batch {batch_index}, row {index}: Q is parameter-dependent for obs_idx {obs}",
                        path.display()
                    );
                }
                Some(_) => {}
            }
        }
    }
    values
        .into_iter()
        .enumerate()
        .map(|(obs, value)| {
            value.with_context(|| {
                format!(
                    "{}: Q source contains no value for obs_idx {obs}",
                    path.display()
                )
            })
        })
        .collect()
}

fn build_q_override(model: &ModelConfig, target: &[Observation]) -> Result<Option<Vec<f32>>> {
    let Some(source) = &model.query_q else {
        return Ok(None);
    };
    let source_observations = load_observations(&source.observations.path)?;
    let source_q = load_constant_q(&source.tensor.path, source_observations.len())?;
    let mut by_key = HashMap::<ObservationKey, usize>::new();
    for observation in &source_observations {
        if by_key
            .insert(ObservationKey::from(observation), observation.obs_idx)
            .is_some()
        {
            bail!("Q-source biological observation identities are not unique");
        }
    }
    let mut used = BTreeSet::new();
    let values = target
        .iter()
        .map(|observation| {
            let source_idx = *by_key
                .get(&ObservationKey::from(observation))
                .with_context(|| {
                    format!("no Q-source match for obs_idx {}", observation.obs_idx)
                })?;
            used.insert(source_idx);
            Ok(source_q[source_idx])
        })
        .collect::<Result<Vec<_>>>()?;
    if used.len() != source_observations.len() || values.len() != source_observations.len() {
        bail!("Q-source join is not exhaustive and one-to-one");
    }
    Ok(Some(values))
}

fn log_score(q: f64, p_pos: f64, p_neg: f64, pi: f64, epsilon: f64) -> f32 {
    let log_epsilon = epsilon.ln();
    if q <= 0.0 || p_pos <= 0.0 || p_neg <= 0.0 || pi <= 0.0 {
        return log_epsilon as f32;
    }
    let log_f = q.ln() + p_pos.ln() + p_neg.ln() + pi.ln();
    if log_f >= log_epsilon {
        (log_f + (log_epsilon - log_f).exp().ln_1p()) as f32
    } else {
        (log_epsilon + (log_f - log_epsilon).exp().ln_1p()) as f32
    }
}

fn validate_candidate(candidate: &Candidate, metadata: &Metadata) -> Result<()> {
    let row = &candidate.parameters;
    if row.geometry_idx >= metadata.geometry_params.len() || row.mn_idx >= metadata.n_mn {
        bail!(
            "candidate {} indexes outside tensor metadata",
            candidate.system_id
        );
    }
    if row.regime_idx != row.geometry_idx * metadata.n_mn + row.mn_idx
        || row.param_idx != row.geometry_idx
    {
        bail!(
            "candidate {} has inconsistent regime coordinates",
            candidate.system_id
        );
    }
    let geometry = metadata.geometry_params[row.geometry_idx];
    let mn = metadata.mn_tuples[row.mn_idx];
    let exact = geometry[0].to_bits() == row.d_pos.to_bits()
        && geometry[1].to_bits() == row.d_neg.to_bits()
        && geometry[2].to_bits() == row.steepness_pos.to_bits()
        && geometry[3].to_bits() == row.steepness_neg.to_bits()
        && mn == [row.m, row.n];
    if !exact {
        bail!(
            "candidate {} disagrees with tensor metadata",
            candidate.system_id
        );
    }
    Ok(())
}

/// Full-roster selection must cover the tensor's geometry × M/N grid, not
/// merely all rows in a potentially truncated discovery metrics table.
pub(crate) fn validate_complete_regime_grid(
    candidates: &[Candidate],
    metadata: &Metadata,
) -> Result<()> {
    let expected = metadata
        .geometry_params
        .len()
        .checked_mul(metadata.n_mn)
        .context("tensor regime count overflow")?;
    if candidates.len() != expected {
        bail!(
            "all-regimes roster has {} candidates; tensor metadata declares {expected} regimes",
            candidates.len()
        );
    }
    let mut regimes = BTreeSet::new();
    for candidate in candidates {
        validate_candidate(candidate, metadata)?;
        if !regimes.insert(candidate.parameters.regime_idx) {
            bail!(
                "all-regimes roster duplicates regime {}",
                candidate.parameters.regime_idx
            );
        }
    }
    if !regimes.into_iter().eq(0..expected) {
        bail!("all-regimes roster does not cover the complete tensor grid");
    }
    Ok(())
}

pub fn score_model_candidates(
    model: &ModelConfig,
    candidates: &[Candidate],
    epsilon: f64,
) -> Result<ScoredModel> {
    if candidates.is_empty() {
        bail!("cannot score an empty candidate roster");
    }
    let observations = load_observations(&model.primary.observations.path)?;
    let metadata = load_metadata(&model.primary.metadata.path)?;
    let q_override = build_q_override(model, &observations)?;
    let mut unique = BTreeMap::new();
    for candidate in candidates {
        validate_candidate(candidate, &metadata)?;
        unique
            .entry(candidate.parameters.regime_idx)
            .or_insert_with(|| candidate.clone());
    }
    let coordinates: HashMap<(usize, usize), usize> = unique
        .values()
        .map(|candidate| {
            (
                (
                    candidate.parameters.geometry_idx,
                    candidate.parameters.mn_idx,
                ),
                candidate.parameters.regime_idx,
            )
        })
        .collect();
    if coordinates.len() != unique.len() {
        bail!("distinct selected regimes share tensor coordinates");
    }
    let mut scores: BTreeMap<usize, Vec<f32>> = unique
        .keys()
        .map(|&regime| (regime, vec![f32::NEG_INFINITY; observations.len()]))
        .collect();
    let coverage_len = observations
        .len()
        .checked_mul(metadata.n_tau)
        .context("selected tensor coverage size overflow")?;
    let mut seen: BTreeMap<usize, Vec<bool>> = unique
        .keys()
        .map(|&regime| (regime, vec![false; coverage_len]))
        .collect();

    let file = File::open(&model.primary.tensor.path)
        .with_context(|| format!("opening {}", model.primary.tensor.path.display()))?;
    for (batch_index, batch) in ParquetRecordBatchReaderBuilder::try_new(file)?
        .build()?
        .enumerate()
    {
        let batch = batch?;
        validate_input_batch(
            &batch,
            &model.primary.tensor.path,
            batch_index,
            &[
                "obs_idx",
                "param_idx",
                "mn_idx",
                "q_value",
                "pos_prob",
                "neg_prob",
                "pi_eh",
            ],
        )?;
        let obs_idx = u32_column(&batch, "obs_idx")?;
        let param_idx = u32_column(&batch, "param_idx")?;
        let mn_idx = u32_column(&batch, "mn_idx")?;
        let q = f32_column(&batch, "q_value")?;
        let pos = f32_column(&batch, "pos_prob")?;
        let neg = f32_column(&batch, "neg_prob")?;
        let pi = f32_column(&batch, "pi_eh")?;
        for index in 0..batch.num_rows() {
            let obs = obs_idx.value(index) as usize;
            let param = param_idx.value(index) as usize;
            let mn = mn_idx.value(index) as usize;
            if obs >= observations.len() || param >= metadata.n_params || mn >= metadata.n_mn {
                bail!(
                    "{}: batch {batch_index}, row {index}: tensor index out of range (obs_idx={obs}, param_idx={param}, mn_idx={mn})",
                    model.primary.tensor.path.display()
                );
            }
            let geometry = param / metadata.n_tau;
            let Some(&regime) = coordinates.get(&(geometry, mn)) else {
                continue;
            };
            let tau = param % metadata.n_tau;
            let covered =
                &mut seen.get_mut(&regime).expect("registered regime")[obs * metadata.n_tau + tau];
            if *covered {
                bail!(
                    "{}: batch {batch_index}, row {index}: duplicate tensor cell (obs_idx={obs}, regime_idx={regime}, tau={tau})",
                    model.primary.tensor.path.display()
                );
            }
            *covered = true;
            let q_value = q_override
                .as_ref()
                .map_or(q.value(index), |values| values[obs]);
            let value = log_score(
                q_value as f64,
                pos.value(index) as f64,
                neg.value(index) as f64,
                pi.value(index) as f64,
                epsilon,
            );
            if !value.is_finite() {
                bail!("nonfinite score for regime {regime}, obs_idx {obs}");
            }
            let slot = &mut scores.get_mut(&regime).expect("registered regime")[obs];
            if value > *slot {
                *slot = value;
            }
        }
    }
    for (&regime, coverage) in &seen {
        if let Some(cell) = coverage.iter().position(|&present| !present) {
            let obs = cell / metadata.n_tau;
            let tau = cell % metadata.n_tau;
            bail!(
                "{}: missing tensor cell (obs_idx={obs}, regime_idx={regime}, tau={tau})",
                model.primary.tensor.path.display()
            );
        }
    }
    Ok(ScoredModel {
        observations,
        scores_by_regime: scores
            .into_iter()
            .map(|(regime, values)| (regime, values.into_iter().map(f64::from).collect()))
            .collect(),
    })
}
