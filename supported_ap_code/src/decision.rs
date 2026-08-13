use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize};

use crate::Error;

macro_rules! validated_f64 {
    ($name:ident, $error:ident, $predicate:expr) => {
        #[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(f64);

        impl $name {
            pub fn new(value: f64) -> Result<Self, Error> {
                if ($predicate)(value) {
                    Ok(Self(value))
                } else {
                    Err(Error::$error(value))
                }
            }

            #[inline]
            pub const fn get(self) -> f64 {
                self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = f64::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let parsed = value.parse::<f64>().map_err(|_| Error::$error(f64::NAN))?;
                Self::new(parsed)
            }
        }
    };
}

validated_f64!(
    MagnitudeThreshold,
    InvalidMagnitudeThreshold,
    |value: f64| value.is_finite() && value >= 0.0
);
validated_f64!(SurvivalFloor, InvalidSurvivalFloor, |value: f64| value
    .is_finite()
    && value >= 0.0);
validated_f64!(
    SurvivalRequirement,
    InvalidSurvivalRequirement,
    |value: f64| value.is_finite() && value > 0.0 && value < 1.0
);

/// The two prespecified thresholds in the V17 finite-evidence replacement rule.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplacementPolicy {
    pub magnitude_threshold: MagnitudeThreshold,
    pub survival_floor: SurvivalFloor,
    pub survival_requirement: SurvivalRequirement,
}

impl ReplacementPolicy {
    pub const fn new(
        magnitude_threshold: MagnitudeThreshold,
        survival_floor: SurvivalFloor,
        survival_requirement: SurvivalRequirement,
    ) -> Self {
        Self {
            magnitude_threshold,
            survival_floor,
            survival_requirement,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FiniteEvidenceVerdict {
    SupportedReplacement,
    NoVerdict,
}

impl ReplacementPolicy {
    pub(crate) fn verdict(
        self,
        supported_magnitude: f64,
        literal_survival_fraction: f64,
    ) -> FiniteEvidenceVerdict {
        if supported_magnitude > self.magnitude_threshold.get()
            && literal_survival_fraction > self.survival_requirement.get()
        {
            FiniteEvidenceVerdict::SupportedReplacement
        } else {
            FiniteEvidenceVerdict::NoVerdict
        }
    }
}
