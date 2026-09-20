//! Certificate-based candidate-conservative selection. This module never
//! constructs a `Reduction` or a `CompletenessAudit` from partial results.
mod certificate;
mod plan;
mod scheduler;

pub use certificate::{
    AuditedSelection, ComparisonEvidence, ExclusionWitness, MatchReference, SelectionCertificate,
    SurvivorCertificate, audit_selection, read_certificate,
};
pub use plan::{
    AcceleratedOptions, AcceleratedPlan, ContextExecution, OutputScope, plan_accelerated,
    read_accelerated_plan, validate_accelerated_plan, write_accelerated_plan,
};
pub use scheduler::{AcceleratedRunSummary, run_accelerated};

mod distributed;
pub use distributed::{
    finish_distributed, merge_target_shards, run_completion_shard, run_target_shard,
};
