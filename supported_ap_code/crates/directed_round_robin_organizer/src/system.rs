use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::spec::{TournamentSpec, validate_id};
use crate::{Error, Result};

/// An opaque contestant registered by an external score provider.
///
/// The organizer interprets only `system_id` and `score_column`. Domain-specific
/// structure may be retained in `annotations`, but it never affects validation,
/// scheduling, judgment, graph construction, or selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemRecord {
    pub system_id: String,
    pub display_label: String,
    pub score_column: String,
    #[serde(default)]
    pub annotations: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemRegistry {
    pub schema_version: u32,
    pub systems: Vec<SystemRecord>,
}

impl SystemRegistry {
    pub fn validate(&self, spec: &TournamentSpec) -> Result<()> {
        if self.schema_version != 2 {
            return Err(Error::InvalidBundle(format!(
                "unsupported systems schema version {}",
                self.schema_version
            )));
        }
        if self.systems.len() < 2 {
            return Err(Error::InvalidBundle(
                "at least two systems are required".into(),
            ));
        }
        let mut system_ids = BTreeSet::new();
        let mut columns = BTreeSet::new();
        for system in &self.systems {
            for (field, value) in [
                ("system_id", &system.system_id),
                ("score_column", &system.score_column),
            ] {
                validate_id(field, value).map_err(|error| {
                    Error::InvalidBundle(format!("system {}: {error}", system.system_id))
                })?;
            }
            if system.display_label.trim().is_empty() {
                return Err(Error::InvalidBundle(format!(
                    "system {} has an empty display label",
                    system.system_id
                )));
            }
            for key in system.annotations.keys() {
                validate_id("annotation key", key).map_err(|error| {
                    Error::InvalidBundle(format!("system {}: {error}", system.system_id))
                })?;
            }
            if !system_ids.insert(&system.system_id) {
                return Err(Error::InvalidBundle(format!(
                    "duplicate system ID {}",
                    system.system_id
                )));
            }
            if !columns.insert(&system.score_column) {
                return Err(Error::InvalidBundle(format!(
                    "duplicate score column {}",
                    system.score_column
                )));
            }
        }
        if let Some(order) = &spec.operational_tie_break {
            if order.len() != self.systems.len()
                || order.iter().collect::<BTreeSet<_>>().len() != order.len()
                || order
                    .iter()
                    .any(|system_id| !system_ids.contains(system_id))
            {
                return Err(Error::InvalidBundle(
                    "operational tie-break must be a complete permutation of system IDs".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn sorted_systems(&self) -> Vec<&SystemRecord> {
        let mut systems: Vec<_> = self.systems.iter().collect();
        systems.sort_by(|left, right| left.system_id.cmp(&right.system_id));
        systems
    }

    pub fn by_id(&self) -> BTreeMap<&str, &SystemRecord> {
        self.systems
            .iter()
            .map(|system| (system.system_id.as_str(), system))
            .collect()
    }
}
