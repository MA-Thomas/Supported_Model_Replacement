//! Self-gated power-mean-anchor Hill-q L2 aggregation and complete-F parameter
//! selection. Joint `(alpha, q, c, kappa)` tuples are selected separately for
//! the PDAC-only and equal-weight all-context policies.

pub mod cluster;
pub mod cnap;
pub mod config;
pub mod data;
pub mod error;
pub mod grid;
pub mod manifest;
pub mod numeric;
pub mod output;
pub mod select;

pub use error::{Result, SelectionError};
