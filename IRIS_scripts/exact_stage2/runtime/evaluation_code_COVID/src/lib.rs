// src/lib.rs - Shared types and utilities for evaluation pipeline

pub mod assembly_contract;
pub mod sparse_grid;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Observation: a single (peptide, HLA, env_id) tuple with its label
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub peptide: String,
    pub hla: String,
    pub env_id: u32,
    pub patient_id: String,
    pub label: u8,  // 1 = immunogenic (MT), 0 = non-immunogenic (WT)
    pub gene: String,
    pub cancer_type: String,
    pub wt_mt_group_id: String,  // patient_id + gene (groups one MT with its WT windows for per-event loss)
}

/// Result of grid search: AUC for each parameter combination
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridSearchResult {
    pub param_idx: u32,
    pub mn_idx: u32,
    pub auc: f64,
    pub n_positive: usize,
    pub n_negative: usize,
}

/// Parse scientific notation string back to f32
pub fn parse_scientific(s: &str) -> Result<f32, std::num::ParseFloatError> {
    s.trim().parse::<f32>()
}

/// Configuration for the evaluation pipeline
#[derive(Debug, Clone)]
pub struct EvalConfig {
    pub results_base_dir: PathBuf,
    pub labels_mt_path: PathBuf,
    pub labels_wt_path: PathBuf,
    pub output_dir: PathBuf,
}

/// Patient-level cross-validation fold assignment
/// Returns a vector of fold indices (0..k) for each observation
pub fn assign_patient_folds(
    observations: &[Observation],
    k: usize,
    seed: u64,
) -> Vec<usize> {
    use std::collections::BTreeMap;
    
    // Group observations by patient_id (LaTeX ground truth: patient-level CV)
    // NOTE: env_id may encode other experimental context; we intentionally group by patient_id.
    let mut patient_obs: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, obs) in observations.iter().enumerate() {
        patient_obs.entry(obs.patient_id.clone()).or_default().push(i);
    }
    
    // Assign patients to folds (simple round-robin for reproducibility)
    // For stratification, we could sort by dominant HLA supertype first
    let patients: Vec<String> = patient_obs.keys().cloned().collect();
    let mut patient_fold: HashMap<String, usize> = HashMap::new();
    
    // Simple deterministic assignment with seed-based shuffle
    let mut shuffled_patients = patients.clone();
    // Use a simple LCG for reproducible shuffle
    let mut rng_state = seed;
    for i in (1..shuffled_patients.len()).rev() {
        rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let j = (rng_state as usize) % (i + 1);
        shuffled_patients.swap(i, j);
    }
    
    for (i, patient) in shuffled_patients.iter().enumerate() {
        patient_fold.insert(patient.clone(), i % k);
    }
    
    // Assign each observation the fold of its patient
    observations
        .iter()
        .map(|obs| *patient_fold.get(&obs.patient_id).unwrap())
        .collect()
}
