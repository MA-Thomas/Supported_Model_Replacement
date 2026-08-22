#![forbid(unsafe_code)]
//! Deterministic orchestration for complete, single-metric supported-evidence tournaments.
//!
//! This crate owns validation, planning, execution, artifact integrity, and verdict-only
//! reduction. All AP, CNAP, AUROC, resampling, support, survival, and optimization mathematics
//! remain in `supported-ap`.

pub mod artifact;
pub mod audit;
pub mod error;
pub mod graph;
pub mod identity;
pub mod input;
pub mod judge;
pub mod plan;
pub mod provenance;
pub mod reduce;
pub mod revision;
pub mod spec;
pub mod system;

pub use artifact::{MatchArtifact, run_shard};
pub use audit::{CompletenessAudit, audit_results, status_results};
pub use error::{Error, Result};
pub use input::{LoadedBundle, load_bundle, load_bundle_for_hashing};
pub use plan::{TournamentPlan, plan_with_match_target, plan_with_shard_count};
pub use provenance::{BuildProvenance, capture_build_provenance};
pub use reduce::{
    EvaluationMaximalSet, InducedReductionView, Reduction, induce_reduction_view,
    load_reduction_for, reduce_tournament, write_induced_reduction_view, write_reduction,
};
pub use revision::{
    RevisionComposition, TournamentReductionSource, TournamentSource,
    audit_revision_composition_for, compose_revision, compose_revision_from_reductions,
    write_revision_composition,
};
pub use spec::{DirectedVerdict, MetricKind, SelectionStrategy, TournamentSpec};
