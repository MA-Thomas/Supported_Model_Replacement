#![forbid(unsafe_code)]
//! Finite-evidence model-replacement assessments following manuscript V17.
//!
//! Projected assessments use computational replications of one observed
//! evaluation. Observed assessments use separately conducted full evaluations.
//! Nested assessments combine several empirical evaluations with independent
//! within-row computational challenges while preserving every row identity.
//! The observed empirical gate precedes any declared computational challenge,
//! and every computational challenge retains the observed effect as an anchor.

/// Cargo package version embedded in scientific result provenance.
pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Manuscript/report contract implemented by this crate.
pub const MANUSCRIPT_VERSION: &str = "v17";

mod auroc;
mod decision;
mod error;
mod execution;
mod metric;
mod nested;
mod paired;
mod prevalence;
mod projected;
mod random;
mod resample;
mod support;
mod survival;
mod types;

pub use auroc::{
    AuRocBackend, AuRocBreakdown, AuRocBreakdownEstimate, AuRocEvidence, AuRocOptimizationOptions,
    AuRocPolicy, AuRocProfilePoint, ConcentrationFactor, ConcentrationSearchOptions, FirstFailure,
    NestedAuRocAssessment, NestedAuRocBreakdownEstimate, NestedAuRocProfilePoint,
    ObservedAuRocAssessment, OptimizationCertificate, OptimizationStatus, PolicyVerdict,
    ProjectedAuRocAssessment, estimate_nested_auroc_breakdown, estimate_observed_auroc_breakdown,
    estimate_projected_auroc_breakdown, paired_auroc_difference, worst_case_auroc_difference,
};
pub use decision::{
    FiniteEvidenceVerdict, MagnitudeThreshold, ReplacementPolicy, SurvivalFloor,
    SurvivalRequirement,
};
pub use error::Error;
pub use nested::{
    AnchoredEffectRow, AnchoredNestedSupport, AnchoredRowSurvival, NestedLiteralSurvival,
    anchored_nested_support,
};
pub use paired::{
    ApEvidence, DirectionalFiniteEvidence, DirectionalNestedFiniteEvidence, EvaluationProfile,
    MarginalProfilePoint, NestedApAssessment, NestedRetainedEffectRow, NestedRowResampling,
    ObservedApAssessment, ObservedApResult, PairedEvaluation, PairedResamplingTrace,
    ProjectedApAssessment, ReferenceAssessment, RetainedEffect, StagedDirectionalNestedAp,
    StagedDirectionalProjectedAp, StagedNestedApResult, StagedProjectedApResult,
    assess_observed_ap, assess_staged_nested_ap, assess_staged_projected_ap,
};
pub use prevalence::{ProfileExtremum, SearchOptions, TargetPrevalences};
pub use projected::{ProfilePoint, ProjectedResampling};
pub use support::{monte_carlo_standard_error, support};
pub use survival::{LiteralSurvival, literal_survival};
pub use types::{
    ClassCounts, ComputationalReplicationCount, Execution, Prevalence, ResamplingUnit,
    ScoreTransportAssumption, SupportOrder,
};
