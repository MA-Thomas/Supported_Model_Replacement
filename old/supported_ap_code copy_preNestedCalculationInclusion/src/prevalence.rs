use serde::{Deserialize, Serialize};

use crate::{Error, Prevalence};

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
                        let value =
                            lower.get() + width * index as f64 / (count.saturating_sub(1)) as f64;
                        Prevalence::new(value).expect("interior interval points are valid")
                    })
                    .collect()
            }
        }
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
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
    pub grid_points: usize,
    pub tolerance: f64,
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
            grid_points: 257,
            tolerance: 1e-8,
            max_iterations: 128,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProfileExtremum {
    pub value: f64,
    pub prevalence: Prevalence,
}

pub(crate) fn minimize<F>(
    set: &TargetPrevalences,
    search: SearchOptions,
    mut evaluate: F,
) -> ProfileExtremum
where
    F: FnMut(Prevalence) -> f64,
{
    match set {
        TargetPrevalences::Finite { values } => values
            .iter()
            .copied()
            .map(|prevalence| ProfileExtremum {
                value: evaluate(prevalence),
                prevalence,
            })
            .min_by(|a, b| a.value.total_cmp(&b.value))
            .expect("finite target set is validated as nonempty"),
        TargetPrevalences::ClosedInterval { lower, upper } => {
            if lower == upper {
                return ProfileExtremum {
                    value: evaluate(*lower),
                    prevalence: *lower,
                };
            }

            let points = set.diagnostic_points(search);
            let values: Vec<f64> = points.iter().copied().map(&mut evaluate).collect();
            let mut best_index = values
                .iter()
                .enumerate()
                .min_by(|left, right| left.1.total_cmp(right.1))
                .map(|(index, _)| index)
                .expect("interval grid is nonempty");
            let mut best = ProfileExtremum {
                value: values[best_index],
                prevalence: points[best_index],
            };

            for index in 1..points.len() - 1 {
                if values[index] <= values[index - 1] && values[index] <= values[index + 1] {
                    let candidate = golden_section_minimum(
                        points[index - 1],
                        points[index + 1],
                        search,
                        &mut evaluate,
                    );
                    if candidate.value < best.value {
                        best = candidate;
                        best_index = index;
                    }
                }
            }

            if best_index == 0 || best_index + 1 == points.len() {
                best
            } else {
                let refined = golden_section_minimum(
                    points[best_index - 1],
                    points[best_index + 1],
                    search,
                    &mut evaluate,
                );
                if refined.value < best.value {
                    refined
                } else {
                    best
                }
            }
        }
    }
}

fn golden_section_minimum<F>(
    lower: Prevalence,
    upper: Prevalence,
    search: SearchOptions,
    evaluate: &mut F,
) -> ProfileExtremum
where
    F: FnMut(Prevalence) -> f64,
{
    const RATIO: f64 = 0.618_033_988_749_894_9;
    let mut left = lower.get();
    let mut right = upper.get();
    let mut c = right - RATIO * (right - left);
    let mut d = left + RATIO * (right - left);
    let mut fc = evaluate(Prevalence::new(c).expect("bracket is inside (0,1)"));
    let mut fd = evaluate(Prevalence::new(d).expect("bracket is inside (0,1)"));
    for _ in 0..search.max_iterations {
        if right - left <= search.tolerance {
            break;
        }
        if fc <= fd {
            right = d;
            d = c;
            fd = fc;
            c = right - RATIO * (right - left);
            fc = evaluate(Prevalence::new(c).expect("bracket is inside (0,1)"));
        } else {
            left = c;
            c = d;
            fc = fd;
            d = left + RATIO * (right - left);
            fd = evaluate(Prevalence::new(d).expect("bracket is inside (0,1)"));
        }
    }
    if fc <= fd {
        ProfileExtremum {
            value: fc,
            prevalence: Prevalence::new(c).expect("bracket is inside (0,1)"),
        }
    } else {
        ProfileExtremum {
            value: fd,
            prevalence: Prevalence::new(d).expect("bracket is inside (0,1)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_search_refines_an_interior_minimum() {
        let set = TargetPrevalences::closed_interval(
            Prevalence::new(0.01).unwrap(),
            Prevalence::new(0.9).unwrap(),
        )
        .unwrap();
        let result = minimize(&set, SearchOptions::default(), |prevalence| {
            (prevalence.get() - 0.371_234).powi(2) + 0.2
        });
        assert!((result.prevalence.get() - 0.371_234).abs() < 1e-7);
        assert!((result.value - 0.2).abs() < 1e-12);
    }
}
