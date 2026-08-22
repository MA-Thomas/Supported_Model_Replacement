//! End-to-end exercise of the data loader against a synthetic but internally
//! consistent dataset: two endpoints (one multi-candidate positive, one
//! singleton negative), with committed `max`/`logsumexp` baselines computed the
//! same way `stable_lse` reconstructs them, so the audit must pass. Also checks
//! the authoritative-endpoint join and label validation.

use std::fs;
use std::path::Path;

use select_adaptive_hillq::data::{authoritative_endpoint_table, load_task, validate_task_labels};

fn write(path: &Path, body: &str) {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(path, body).unwrap();
}

#[test]
fn loads_rosters_and_passes_baseline_audit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let source_root = root.join("full_roster_transfers");
    let transfer = source_root.join("pdac/full_hla/pr");
    let mapping_path = root.join("normalized_inputs/mapping.csv");

    // E1 candidates: -0.5 (HLA A), -1.5 (HLA B) -> max -0.5, lse computed below.
    // E2 candidate: -2.0 (singleton) -> max/lse -2.0.
    let floor = 1e-12_f64.ln();
    let lse_e1 = -0.5_f64 + ((0.0_f64).exp() + (-1.0_f64).exp() + (floor + 0.5).exp()).ln();

    // predictions: four L2 variants for both endpoints.
    let predictions = format!(
        "patient_id,mutation,long_peptide,l2_variant,score,label\n\
         P1,M1,LP1,max,-0.5,1\n\
         P1,M2,LP2,max,-2.0,0\n\
         P1,M1,LP1,logsumexp,{lse_e1},1\n\
         P1,M2,LP2,logsumexp,-2.0,0\n\
         P1,M1,LP1,logmeanexp,-0.9,1\n\
         P1,M2,LP2,logmeanexp,-2.0,0\n\
         P1,M1,LP1,top_frac_mean_frac0p05,-0.5,1\n\
         P1,M2,LP2,top_frac_mean_frac0p05,-2.0,0\n"
    );
    write(&transfer.join("long_peptide_predictions.csv"), &predictions);

    let tau = "patient_id,env_id,peptide,hla,score,obs_idx\n\
        P1,10,AAAAAAAAA,A,-0.5,1\n\
        P1,10,BBBBBBBBB,B,-1.5,2\n\
        P1,20,CCCCCCCCC,A,-2.0,3\n";
    write(
        &transfer.join("target_tau_selection_by_observation.csv"),
        tau,
    );

    let mapping = "patient_id,mutation,long_peptide,env_id,nmer,HLA-RE,mapping_status\n\
        P1,M1,LP1,10,AAAAAAAAA,A,scoreable\n\
        P1,M1,LP1,10,AAAAAAAAA,HLA-A,scoreable\n\
        P1,M1,LP1,10,BBBBBBBBB,B,scoreable\n\
        P1,M1,LP1,11,DDDDDDDDD,C,floor\n\
        P1,M2,LP2,20,CCCCCCCCC,A,scoreable\n\
        P1,M2,LP2,20,DDDDDDDD,A,scoreable\n"; // 8-mer -> filtered out
    write(&mapping_path, mapping);

    let summary =
        "{\"mapping\": \"/unavailable/original/normalized_inputs/mapping.csv\"}".to_string();
    write(&transfer.join("summary.json"), &summary);

    let (task, audit) = load_task(&source_root, "pdac", "full_hla", "pr").unwrap();
    assert_eq!(task.endpoints.len(), 2);
    assert_eq!(task.raw_candidates[0], vec![-0.5, -1.5, floor]);
    assert_eq!(task.raw_candidates[1], vec![-2.0]);
    assert_eq!(task.labels(), vec![1i8, 0]);
    assert!(audit.max_reconstruction_max_abs_error < 1e-12);
    assert!(audit.logsumexp_reconstruction_max_abs_error < 1e-9);
    assert_eq!(audit.n_unique_candidates, 4);
    assert_eq!(audit.n_scoreable_candidates, 3);
    assert_eq!(audit.n_floor_candidates, 1);
    assert_eq!(audit.n_exact_duplicates_removed, 1);
    assert_eq!(audit.source_variant_column, "l2_variant");
    assert_eq!(audit.n_positive, 1);

    // authoritative bundle + label validation
    let eval = root.join("bundle/pr/evaluations/pdac");
    write(
        &eval.join("endpoints.csv"),
        "endpoint_id,label\nep1,1\nep2,0\n",
    );
    write(
        &eval.join("endpoint_identities.csv"),
        "endpoint_id,patient_id,mutation,long_peptide\nep1,P1,M1,LP1\nep2,P1,M2,LP2\n",
    );
    let authority = authoritative_endpoint_table(&root.join("bundle"), "pdac").unwrap();
    let ids = validate_task_labels(&task, &authority).unwrap();
    assert_eq!(ids, vec!["ep1".to_string(), "ep2".to_string()]);
}

#[test]
fn audit_fails_on_inconsistent_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let transfer = root.join("pdac/full_hla/pr");
    let mapping_path = root.join("normalized_inputs/mapping.csv");

    // committed max is wrong (-9.0) relative to the roster max (-0.5).
    let predictions = "patient_id,mutation,long_peptide,l2_variant,score,label\n\
        P1,M1,LP1,max,-9.0,1\n\
        P1,M2,LP2,max,-2.0,0\n\
        P1,M1,LP1,logsumexp,-0.1,1\n\
        P1,M2,LP2,logsumexp,-2.0,0\n\
        P1,M1,LP1,logmeanexp,-0.9,1\n\
        P1,M2,LP2,logmeanexp,-2.0,0\n\
        P1,M1,LP1,top_frac_mean_frac0p05,-0.5,1\n\
        P1,M2,LP2,top_frac_mean_frac0p05,-2.0,0\n";
    write(&transfer.join("long_peptide_predictions.csv"), predictions);
    write(
        &transfer.join("target_tau_selection_by_observation.csv"),
        "patient_id,env_id,peptide,hla,score,obs_idx\n\
         P1,10,AAAAAAAAA,A,-0.5,1\n\
         P1,20,CCCCCCCCC,A,-2.0,3\n",
    );
    write(
        &mapping_path,
        "patient_id,mutation,long_peptide,env_id,nmer,HLA-RE,mapping_status\n\
         P1,M1,LP1,10,AAAAAAAAA,A,scoreable\n\
         P1,M2,LP2,20,CCCCCCCCC,A,scoreable\n",
    );
    write(
        &transfer.join("summary.json"),
        &format!("{{\"mapping\": \"{}\"}}", mapping_path.display()),
    );

    let result = load_task(root, "pdac", "full_hla", "pr");
    assert!(result.is_err(), "expected baseline reconstruction failure");
}
