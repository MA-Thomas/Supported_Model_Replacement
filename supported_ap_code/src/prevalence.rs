use serde::{Deserialize, Serialize};

use crate::{Error, Prevalence, enclosure::Enclosure, prevalence_profile::PreparedDifference};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TargetPrevalences {
    Finite {
        values: Vec<Prevalence>,
    },
    ClosedInterval {
        lower: Prevalence,
        upper: Prevalence,
    },
}

impl TargetPrevalences {
    pub fn finite(values: impl IntoIterator<Item = Prevalence>) -> Result<Self, Error> {
        let mut values: Vec<_> = values.into_iter().collect();
        if values.is_empty() {
            return Err(Error::EmptyPrevalenceSet);
        }
        values.sort_unstable_by(|a, b| a.get().total_cmp(&b.get()));
        values.dedup_by(|a, b| a.get() == b.get());
        Ok(Self::Finite { values })
    }

    pub fn closed_interval(lower: Prevalence, upper: Prevalence) -> Result<Self, Error> {
        if lower.get() <= upper.get() {
            Ok(Self::ClosedInterval { lower, upper })
        } else {
            Err(Error::InvalidPrevalenceInterval)
        }
    }

    pub fn diagnostic_points(&self, search: SearchOptions) -> Vec<Prevalence> {
        match self {
            Self::Finite { values } => values.clone(),
            Self::ClosedInterval { lower, upper } => {
                let count = search.grid_points;
                let width = upper.get() - lower.get();
                (0..count)
                    .map(|index| {
                        let value = if index == 0 {
                            lower.get()
                        } else if index + 1 == count {
                            upper.get()
                        } else {
                            lower.get() + width * (index as f64 / (count - 1) as f64)
                        };
                        Prevalence::new(value).expect("interior interval points are valid")
                    })
                    .collect()
            }
        }
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        // The bounded rational profile requires prevalence strictly in (0,1).
        // Recheck at this boundary because deserialized transparent wrappers
        // need not have passed through Prevalence::new.
        match self {
            Self::Finite { values } => {
                for value in values {
                    Prevalence::new(value.get())?;
                }
            }
            Self::ClosedInterval { lower, upper } => {
                Prevalence::new(lower.get())?;
                Prevalence::new(upper.get())?;
            }
        }
        match self {
            Self::Finite { values } if values.is_empty() => Err(Error::EmptyPrevalenceSet),
            Self::ClosedInterval { lower, upper } if lower.get() > upper.get() => {
                Err(Error::InvalidPrevalenceInterval)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SearchOptions {
    /// Initial grid (also used for diagnostics); subsequent points are adaptive.
    pub grid_points: usize,
    /// Absolute error in the objective value, not in prevalence coordinates.
    pub tolerance: f64,
    /// Maximum total interval bisections, shared by both directions.
    pub max_iterations: usize,
}

impl SearchOptions {
    pub fn new(grid_points: usize, tolerance: f64, max_iterations: usize) -> Result<Self, Error> {
        if grid_points < 3 {
            return Err(Error::TooFewGridPoints);
        }
        if !(tolerance.is_finite() && tolerance > 0.0) {
            return Err(Error::InvalidSearchTolerance);
        }
        if max_iterations == 0 {
            return Err(Error::InvalidSearchIterations);
        }
        Ok(Self {
            grid_points,
            tolerance,
            max_iterations,
        })
    }

    pub(crate) fn validate(self) -> Result<(), Error> {
        Self::new(self.grid_points, self.tolerance, self.max_iterations).map(|_| ())
    }
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            grid_points: 3,
            tolerance: 1e-8,
            max_iterations: 128,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrevalenceSearchStop {
    AccuracyReached,
    BudgetExhausted,
    FloatingPointLimit,
}

/// Bounds enclose the true interval minimum, including numerical rounding.
/// Counts are for the shared forward/reverse search, not per direction.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PrevalenceSearchCertificate {
    pub lower_bound: f64,
    pub upper_bound: f64,
    pub sampled_value: f64,
    /// Distinct prevalence point evaluations; interval bounds are additional work.
    pub evaluations: usize,
    pub iterations: usize,
    pub stop_reason: PrevalenceSearchStop,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProfileExtremum {
    /// For an interval, the conservative lower bound used by decisions.
    pub value: f64,
    /// Best evaluated witness; it need not attain the conservative bound.
    pub prevalence: Prevalence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<PrevalenceSearchCertificate>,
}

#[derive(Clone, Copy)]
struct Sample {
    prevalence: Prevalence,
    value: Enclosure,
}

struct Segment {
    left: Sample,
    right: Sample,
    range: Enclosure,
}

impl Segment {
    fn new(profile: &PreparedDifference, left: Sample, right: Sample) -> Self {
        Self {
            range: profile.range(
                left.prevalence.get(),
                right.prevalence.get(),
                left.value,
                right.value,
            ),
            left,
            right,
        }
    }

    fn midpoint(&self) -> Option<Prevalence> {
        let left = self.left.prevalence.get();
        let right = self.right.prevalence.get();
        let mid = left + (right - left) * 0.5;
        (mid > left && mid < right).then(|| Prevalence::new(mid).unwrap())
    }
}

pub(crate) fn interval_extrema(
    set: &TargetPrevalences,
    search: SearchOptions,
    profile: &PreparedDifference,
) -> (ProfileExtremum, ProfileExtremum, Vec<f64>) {
    let points = set.diagnostic_points(search);
    let mut samples: Vec<Sample> = Vec::with_capacity(points.len());
    let mut diagnostics = Vec::with_capacity(points.len());
    for prevalence in points {
        if samples.last().is_none_or(|s| s.prevalence != prevalence) {
            samples.push(Sample {
                prevalence,
                value: profile.point(prevalence.get()),
            });
        }
        diagnostics.push(samples.last().unwrap().value.midpoint());
    }
    let mut best_min = *samples
        .iter()
        .min_by(|a, b| a.value.hi.total_cmp(&b.value.hi))
        .unwrap();
    let mut best_max = *samples
        .iter()
        .max_by(|a, b| a.value.lo.total_cmp(&b.value.lo))
        .unwrap();
    let mut segments: Vec<_> = samples
        .windows(2)
        .map(|w| Segment::new(profile, w[0], w[1]))
        .collect();
    let mut evaluations = samples.len();
    let mut iterations = 0;
    let mut stop = PrevalenceSearchStop::AccuracyReached;
    let (lower, upper) = loop {
        let lower = segments
            .iter()
            .map(|s| s.range.lo)
            .fold(best_min.value.lo, f64::min);
        let upper = segments
            .iter()
            .map(|s| s.range.hi)
            .fold(best_max.value.hi, f64::max);
        if gap(lower, best_min.value.hi) <= search.tolerance
            && gap(best_max.value.lo, upper) <= search.tolerance
        {
            break (lower, upper);
        }
        if iterations == search.max_iterations {
            stop = PrevalenceSearchStop::BudgetExhausted;
            break (lower, upper);
        }
        // A region is relevant if it can improve either directional minimum.
        // Stable tie breaking makes a budgeted run independent of scheduling.
        let next = segments
            .iter()
            .enumerate()
            .filter(|(_, segment)| segment.midpoint().is_some())
            .map(|(index, segment)| {
                (
                    index,
                    gap(segment.range.lo, best_min.value.hi)
                        .max(gap(best_max.value.lo, segment.range.hi)),
                )
            })
            .filter(|(_, gap)| *gap > search.tolerance)
            .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(&a.0)));
        let Some((index, _)) = next else {
            stop = PrevalenceSearchStop::FloatingPointLimit;
            break (lower, upper);
        };
        let parent = segments.swap_remove(index);
        let prevalence = parent.midpoint().unwrap();
        let sample = Sample {
            prevalence,
            value: profile.point(prevalence.get()),
        };
        evaluations += 1;
        iterations += 1;
        if sample.value.hi < best_min.value.hi {
            best_min = sample;
        }
        if sample.value.lo > best_max.value.lo {
            best_max = sample;
        }
        // Intersect with the parent so increasing the budget can only tighten
        // bounds, even when independent rounding enclosures vary slightly.
        for mut child in [
            Segment::new(profile, parent.left, sample),
            Segment::new(profile, sample, parent.right),
        ] {
            child.range = child.range.intersect(parent.range);
            segments.push(child);
        }
    };
    let result = |sample: Sample, bounds: Enclosure, reverse: bool| {
        let sampled_value = if reverse {
            -sample.value.midpoint()
        } else {
            sample.value.midpoint()
        };
        ProfileExtremum {
            value: bounds.lo,
            prevalence: sample.prevalence,
            search: Some(PrevalenceSearchCertificate {
                lower_bound: bounds.lo,
                upper_bound: bounds.hi,
                sampled_value,
                evaluations,
                iterations,
                stop_reason: if gap(bounds.lo, bounds.hi) <= search.tolerance {
                    PrevalenceSearchStop::AccuracyReached
                } else {
                    stop
                },
            }),
        }
    };
    (
        result(
            best_min,
            Enclosure {
                lo: lower,
                hi: best_min.value.hi,
            },
            false,
        ),
        result(
            best_max,
            Enclosure {
                lo: -upper,
                hi: -best_max.value.lo,
            },
            true,
        ),
        diagnostics,
    )
}

fn gap(lower: f64, upper: f64) -> f64 {
    if lower == upper {
        0.0
    } else {
        (Enclosure::point(upper) - Enclosure::point(lower)).hi
    }
}
