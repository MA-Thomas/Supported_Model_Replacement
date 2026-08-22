//! Fixed domain constants, mirroring `analyze_adaptive_l2_deprecated` / the selector.

/// Component models, in the canonical order used for grouping and output.
pub const MODELS: [&str; 5] = [
    "full_hla",
    "focal_hla",
    "old_monoallelic",
    "mono_q_full_pn",
    "full_q_mono_pn",
];

/// Metric branches.
pub const BRANCHES: [&str; 2] = ["pr", "roc"];

/// Selection cohorts (equal-weighted).
pub const SELECTION_COHORTS: [&str; 3] = ["pdac", "covid_spike", "covid_nonspike"];

/// Number of component/metric groups.
pub const N_GROUPS: usize = MODELS.len() * BRANCHES.len();

/// Name of the branch's selection metric.
pub fn selection_metric(branch: &str) -> &'static str {
    match branch {
        "pr" => "paired_supported_cnap_vs_model_matched_max",
        "roc" => "auroc",
        _ => "unknown",
    }
}

/// Default source root (matches `analyze_adaptive_l2_deprecated.DEFAULT_SOURCE`).
pub const DEFAULT_SOURCE: &str = "/Users/thomm15/Work_Data/IRIS_scripts/Single_Parameter_Set_Evaluation/downstream_analyses_and_plots/outputs/external_validation_five_model_supported_ap_auroc_story";
