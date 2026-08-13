//! Exact linear optimization over a capped simplex, expressed in exclusion mass.
//!
//! For class `y` with design size `n_y` and concentration factor `Gamma`, the
//! admissible weights are
//!
//! ```text
//!     0 <= w_j <= m_j * Gamma / n_y ,      sum_j w_j = 1
//! ```
//!
//! where `m_j` is the multiplicity of case `j`. Writing `w_j = u * x_j` with
//! `u = Gamma / n_y`, the constraint becomes `0 <= x_j <= m_j` with
//! `sum_j x_j = n_y / Gamma`. The variable `x_j` is *included multiplicity
//! mass*, and `e_j = m_j - x_j` is *excluded mass*.
//!
//! Working in exclusion mass is what makes the near-`Gamma = 1` regime cheap:
//! total exclusion mass is `n_y (1 - 1/Gamma)`, which is small exactly when the
//! replacement policy still passes, so the active structure is a short list
//! rather than a vector of length `n_y`.

/// Per-class capped design at one concentration factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CappedClass {
    /// Weight carried by one unit of multiplicity mass, `Gamma / n_y`.
    pub(crate) cap: f64,
    /// Multiplicity mass receiving weight, `n_y / Gamma`.
    pub(crate) include_mass: f64,
    /// Multiplicity mass receiving no weight, `n_y (1 - 1/Gamma)`.
    pub(crate) exclude_mass: f64,
    /// Design size `n_y`, counting repeats.
    pub(crate) total_mass: f64,
}

impl CappedClass {
    pub(crate) fn new(total_mass: f64, gamma: f64) -> Self {
        let include_mass = (total_mass / gamma).min(total_mass);
        Self {
            cap: gamma / total_mass,
            include_mass,
            exclude_mass: (total_mass - include_mass).max(0.0),
            total_mass,
        }
    }

    /// True when one case may absorb the entire unit of weight.
    ///
    /// Requires `m_peak * cap >= 1`, i.e. `Gamma * m_peak >= n_y`.
    pub(crate) fn saturates(&self, peak_multiplicity: u32) -> bool {
        f64::from(peak_multiplicity) * self.cap >= 1.0 - 1e-12
    }
}

/// A vertex of the capped simplex, recorded by what it excludes.
///
/// At most one case carries partial mass; every other listed case is excluded
/// entirely. Cases absent from both fields receive full multiplicity weight.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Exclusion {
    pub(crate) full: Vec<u32>,
    pub(crate) partial: Option<(u32, f64)>,
}

impl Exclusion {
    pub(crate) fn clear(&mut self) {
        self.full.clear();
        self.partial = None;
    }

    /// Iterates `(index, excluded_mass)` over every case carrying exclusion.
    pub(crate) fn iter<'a>(
        &'a self,
        multiplicities: &'a [u32],
    ) -> impl Iterator<Item = (u32, f64)> + 'a {
        self.full
            .iter()
            .map(move |&index| (index, f64::from(multiplicities[index as usize])))
            .chain(self.partial)
    }
}

/// Greedy exclusion fill: the exact minimiser of a linear form.
///
/// Minimising `sum_j x_j c_j` subject to `0 <= x_j <= m_j`,
/// `sum_j x_j = include_mass` is the same as *maximising* the excluded
/// contribution `sum_j e_j c_j` subject to `sum_j e_j = exclude_mass`. Exclusion
/// therefore goes to the largest coefficients first.
///
/// `scratch` is reused across calls to avoid per-iteration allocation.
pub(crate) fn fill_exclusion(
    support: &[u32],
    coefficients: &[f64],
    multiplicities: &[u32],
    design: CappedClass,
    scratch: &mut Vec<u32>,
    out: &mut Exclusion,
) {
    out.clear();
    if design.exclude_mass <= 0.0 {
        return;
    }
    scratch.clear();
    scratch.extend_from_slice(support);
    // Descending by coefficient; ties broken by index so results are
    // reproducible regardless of the sort implementation.
    scratch.sort_unstable_by(|&left, &right| {
        coefficients[right as usize]
            .total_cmp(&coefficients[left as usize])
            .then_with(|| left.cmp(&right))
    });

    let mut remaining = design.exclude_mass;
    for &index in scratch.iter() {
        if remaining <= 0.0 {
            break;
        }
        let mass = f64::from(multiplicities[index as usize]);
        if mass <= remaining {
            out.full.push(index);
            remaining -= mass;
        } else {
            out.partial = Some((index, remaining));
            break;
        }
    }
}

/// Largest attainable `sum_j e_j v_j` with `0 <= e_j <= m_j`, `sum e_j = mass`.
///
/// Used for the exclusion-side lower bound, where each class independently
/// removes whichever cases carry the most favourable marginal.
pub(crate) fn maximum_over_mass(
    support: &[u32],
    values: &[f64],
    multiplicities: &[u32],
    mass: f64,
    scratch: &mut Vec<u32>,
) -> f64 {
    accumulate_extreme(support, values, multiplicities, mass, scratch, true)
}

/// Smallest attainable `sum_j x_j v_j` with `0 <= x_j <= m_j`, `sum x_j = mass`.
///
/// Used for the inclusion-side lower bound, where the retained cases are the
/// ones with the most adverse independent relaxation value.
pub(crate) fn minimum_over_mass(
    support: &[u32],
    values: &[f64],
    multiplicities: &[u32],
    mass: f64,
    scratch: &mut Vec<u32>,
) -> f64 {
    accumulate_extreme(support, values, multiplicities, mass, scratch, false)
}

fn accumulate_extreme(
    support: &[u32],
    values: &[f64],
    multiplicities: &[u32],
    mass: f64,
    scratch: &mut Vec<u32>,
    descending: bool,
) -> f64 {
    if mass <= 0.0 {
        return 0.0;
    }
    scratch.clear();
    scratch.extend_from_slice(support);
    scratch.sort_unstable_by(|&left, &right| {
        let ordering = values[left as usize].total_cmp(&values[right as usize]);
        if descending {
            ordering.reverse().then_with(|| left.cmp(&right))
        } else {
            ordering.then_with(|| left.cmp(&right))
        }
    });
    let mut remaining = mass;
    let mut total = 0.0;
    for &index in scratch.iter() {
        if remaining <= 0.0 {
            break;
        }
        let available = f64::from(multiplicities[index as usize]).min(remaining);
        total += available * values[index as usize];
        remaining -= available;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_class_matches_the_appendix_arithmetic() {
        let design = CappedClass::new(100.0, 1.25);
        assert!((design.cap - 0.0125).abs() < 1e-15);
        assert!((design.include_mass - 80.0).abs() < 1e-12);
        assert!((design.exclude_mass - 20.0).abs() < 1e-12);
        // Gamma = 1 leaves nothing to exclude.
        let uniform = CappedClass::new(100.0, 1.0);
        assert!(uniform.exclude_mass.abs() < 1e-12);
    }

    #[test]
    fn saturation_accounts_for_repeated_cases() {
        // A singleton case needs Gamma >= n to absorb all the weight.
        let design = CappedClass::new(10.0, 9.0);
        assert!(!design.saturates(1));
        assert!(design.saturates(2));
        assert!(CappedClass::new(10.0, 10.0).saturates(1));
    }

    #[test]
    fn exclusion_fill_takes_largest_coefficients_first() {
        let support = [0u32, 1, 2, 3];
        let coefficients = [1.0, 4.0, 3.0, 2.0];
        let multiplicities = [1u32, 1, 1, 1];
        let design = CappedClass::new(4.0, 4.0 / 2.5); // exclude_mass = 1.5
        let mut scratch = Vec::new();
        let mut exclusion = Exclusion::default();
        fill_exclusion(
            &support,
            &coefficients,
            &multiplicities,
            design,
            &mut scratch,
            &mut exclusion,
        );
        assert_eq!(exclusion.full, vec![1]);
        let (index, mass) = exclusion.partial.unwrap();
        assert_eq!(index, 2);
        assert!((mass - 0.5).abs() < 1e-12);
    }

    #[test]
    fn exclusion_fill_consumes_multiplicity_before_moving_on() {
        let support = [0u32, 1];
        let coefficients = [5.0, 1.0];
        let multiplicities = [3u32, 4];
        let design = CappedClass::new(7.0, 7.0 / 3.0); // exclude_mass = 4
        let mut scratch = Vec::new();
        let mut exclusion = Exclusion::default();
        fill_exclusion(
            &support,
            &coefficients,
            &multiplicities,
            design,
            &mut scratch,
            &mut exclusion,
        );
        assert_eq!(exclusion.full, vec![0]);
        let (index, mass) = exclusion.partial.unwrap();
        assert_eq!(index, 1);
        assert!((mass - 1.0).abs() < 1e-12);
        let listed: Vec<_> = exclusion.iter(&multiplicities).collect();
        assert_eq!(listed.len(), 2);
        assert!((listed.iter().map(|&(_, mass)| mass).sum::<f64>() - 4.0).abs() < 1e-12);
    }

    #[test]
    fn extreme_accumulators_respect_capacity_and_budget() {
        let support = [0u32, 1, 2];
        let values = [-1.0, 0.0, 2.0];
        let multiplicities = [2u32, 1, 5];
        let mut scratch = Vec::new();
        // Largest 3 units of mass: 3 copies of value 2.
        let maximum = maximum_over_mass(&support, &values, &multiplicities, 3.0, &mut scratch);
        assert!((maximum - 6.0).abs() < 1e-12);
        // Smallest 3 units: 2 copies of -1 then 1 copy of 0.
        let minimum = minimum_over_mass(&support, &values, &multiplicities, 3.0, &mut scratch);
        assert!((minimum + 2.0).abs() < 1e-12);
        // A zero budget contributes nothing.
        assert_eq!(
            maximum_over_mass(&support, &values, &multiplicities, 0.0, &mut scratch),
            0.0
        );
    }
}
