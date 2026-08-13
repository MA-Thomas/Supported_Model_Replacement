use serde::{Deserialize, Serialize};

use crate::{
    ClassCounts, ComputationalReplicationCount, Error, Execution, Prevalence, ResamplingUnit,
    SupportOrder,
};

/// Computational replication design used to project finite evidence from one
/// observed evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedResampling {
    pub replications: ComputationalReplicationCount,
    pub computational_order: SupportOrder,
    pub replication_counts: ClassCounts,
    pub seed: u64,
    pub execution: Execution,
    pub resampling_unit: ResamplingUnit,
}

impl ProjectedResampling {
    pub fn new(
        replications: ComputationalReplicationCount,
        computational_order: SupportOrder,
        replication_counts: ClassCounts,
        seed: u64,
        execution: Execution,
        resampling_unit: ResamplingUnit,
    ) -> Result<Self, Error> {
        let design = Self {
            replications,
            computational_order,
            replication_counts,
            seed,
            execution,
            resampling_unit,
        };
        design.validate()?;
        Ok(design)
    }

    pub(crate) fn validate(self) -> Result<(), Error> {
        if self.computational_order.get() > self.replications.get() {
            Err(Error::SupportOrderExceedsList {
                order: self.computational_order.get(),
                list_length: self.replications.get(),
            })
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProfilePoint {
    pub prevalence: Prevalence,
    pub value: f64,
}
