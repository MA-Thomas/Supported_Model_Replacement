#![forbid(unsafe_code)]
//! Domain-specific construction and operational reduction of IRIS NCI
//! parameter tournaments. Pairwise judgment and graph reduction remain owned
//! by `directed_round_robin_organizer`.

pub mod bundle;
pub mod candidates;
pub mod config;
pub mod io;
pub mod tensor;
pub mod version2;

pub use bundle::{PrepareSummary, audit_prepared, audit_prepared_for_config, prepare};
pub use config::{PipelineConfig, RosterMode};
pub use version2::{
    Version2Selection, audit_version2, finalize_version2, finalize_version2_accelerated,
};

pub mod finalists;
pub mod package;
