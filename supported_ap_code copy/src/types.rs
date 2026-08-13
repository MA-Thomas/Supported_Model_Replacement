use std::num::NonZeroUsize;

use serde::{Deserialize, Serialize};

use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Prevalence(f64);

impl Prevalence {
    pub fn new(value: f64) -> Result<Self, Error> {
        if value.is_finite() && value > 0.0 && value < 1.0 {
            Ok(Self(value))
        } else {
            Err(Error::InvalidPrevalence(value))
        }
    }

    #[inline]
    pub const fn get(self) -> f64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SupportOrder(NonZeroUsize);

impl SupportOrder {
    pub fn new(value: usize) -> Result<Self, Error> {
        NonZeroUsize::new(value)
            .map(Self)
            .ok_or(Error::InvalidSupportOrder)
    }

    #[inline]
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ComputationalReplicationCount(NonZeroUsize);

impl ComputationalReplicationCount {
    pub fn new(value: usize) -> Result<Self, Error> {
        NonZeroUsize::new(value)
            .map(Self)
            .ok_or(Error::InvalidComputationalReplicationCount)
    }

    #[inline]
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassCounts {
    positive: NonZeroUsize,
    negative: NonZeroUsize,
}

impl ClassCounts {
    pub fn new(positive: usize, negative: usize) -> Result<Self, Error> {
        match (NonZeroUsize::new(positive), NonZeroUsize::new(negative)) {
            (Some(positive), Some(negative)) => Ok(Self { positive, negative }),
            _ => Err(Error::InvalidClassCounts),
        }
    }

    pub const fn positive(self) -> usize {
        self.positive.get()
    }

    pub const fn negative(self) -> usize {
        self.negative.get()
    }

    pub const fn total(self) -> usize {
        self.positive.get() + self.negative.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Execution {
    Sequential,
    #[default]
    Parallel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResamplingUnit {
    IndependentObservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreTransportAssumption {
    pub justification: String,
}

impl ScoreTransportAssumption {
    pub fn new(justification: impl Into<String>) -> Result<Self, Error> {
        let justification = justification.into();
        if justification.trim().is_empty() {
            Err(Error::MissingTransportJustification)
        } else {
            Ok(Self { justification })
        }
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.justification.trim().is_empty() {
            Err(Error::MissingTransportJustification)
        } else {
            Ok(())
        }
    }
}
