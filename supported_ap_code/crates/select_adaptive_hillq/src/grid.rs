//! The frozen selection grid: a dense uniform `c` axis crossed with a
//! log-uniform `kappa` axis, and the fixed list of Hill orders. The joint
//! `(q, c, kappa)` lattice is represented implicitly as
//! `j = q_block * base_len + base_index`, with blocks concatenated in declared
//! q order and each block in base-grid order.

use crate::error::{Result, SelectionError};
use crate::numeric::HillOrder;

/// Ranges and resolution for the `(c, kappa)` base grid.
#[derive(Debug, Clone)]
pub struct GridSpec {
    pub c_min: f64,
    pub c_max: f64,
    pub c_step: f64,
    pub kappa_min: f64,
    pub kappa_max: f64,
    pub kappa_points: usize,
}

/// The materialized base grid. `c[i]` / `kappa[i]` are aligned so that grid
/// index `i` has `c = c_values[i / kappa_len]`, `kappa = kappa_values[i % kappa_len]`
/// (NumPy `repeat` of c, `tile` of kappa).
#[derive(Debug, Clone)]
pub struct BaseGrid {
    pub c_values: Vec<f64>,
    pub kappa_values: Vec<f64>,
    pub c: Vec<f64>,
    pub kappa: Vec<f64>,
}

impl BaseGrid {
    pub fn len(&self) -> usize {
        self.c.len()
    }
    pub fn is_empty(&self) -> bool {
        self.c.is_empty()
    }
    pub fn c_min(&self) -> f64 {
        *self.c_values.first().unwrap()
    }
    pub fn c_max(&self) -> f64 {
        *self.c_values.last().unwrap()
    }
    pub fn kappa_min(&self) -> f64 {
        self.kappa_values
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min)
    }
    pub fn kappa_max(&self) -> f64 {
        self.kappa_values
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max)
    }
}

fn round12(x: f64) -> f64 {
    // np.round(x, 12): the c lattice values are exact multiples of the step, so
    // the rounding only strips floating dust.
    (x * 1e12).round() / 1e12
}

/// Port of `make_grid`. Fails closed on a non-divisible or degenerate range.
pub fn make_grid(spec: &GridSpec) -> Result<BaseGrid> {
    let steps = (spec.c_max - spec.c_min) / spec.c_step;
    let finite = spec.c_min.is_finite() && spec.c_max.is_finite() && spec.c_step.is_finite();
    if !finite
        || spec.c_step <= 0.0
        || spec.c_max <= spec.c_min
        || (steps - steps.round()).abs() > 1e-9
    {
        return Err(SelectionError::msg(
            "c range must be finite and exactly divisible by c-step",
        ));
    }
    if !(spec.kappa_min.is_finite() && spec.kappa_max.is_finite())
        || spec.kappa_min <= 0.0
        || spec.kappa_max <= spec.kappa_min
        || spec.kappa_points < 2
    {
        return Err(SelectionError::msg("invalid kappa range or point count"));
    }
    let n_c = steps.round() as usize + 1;
    let c_values: Vec<f64> = (0..n_c)
        .map(|i| round12(spec.c_min + (i as f64) * spec.c_step))
        .collect();

    let log_min = spec.kappa_min.ln();
    let log_max = spec.kappa_max.ln();
    let denom = (spec.kappa_points - 1) as f64;
    let kappa_values: Vec<f64> = (0..spec.kappa_points)
        .map(|i| (log_min + (i as f64) * (log_max - log_min) / denom).exp())
        .collect();

    let n_k = kappa_values.len();
    let mut c = Vec::with_capacity(n_c * n_k);
    let mut kappa = Vec::with_capacity(n_c * n_k);
    for &cv in &c_values {
        for &kv in &kappa_values {
            c.push(cv);
            kappa.push(kv);
        }
    }
    Ok(BaseGrid {
        c_values,
        kappa_values,
        c,
        kappa,
    })
}

/// Parse and de-duplicate the frozen Hill-order list (port of `parse_q_values`).
pub fn parse_q_values(tokens: &[String]) -> Result<Vec<HillOrder>> {
    let mut parsed: Vec<HillOrder> = Vec::new();
    for raw in tokens {
        let order = HillOrder::parse(raw)?;
        let is_dup = parsed.iter().any(|existing| match (existing, &order) {
            (HillOrder::Infinity, HillOrder::Infinity) => true,
            (HillOrder::Finite(a), HillOrder::Finite(b)) => a == b,
            _ => false,
        });
        if is_dup {
            return Err(SelectionError::msg(format!("duplicate Hill order: {raw}")));
        }
        parsed.push(order);
    }
    if parsed.is_empty() {
        return Err(SelectionError::msg("at least one Hill order is required"));
    }
    Ok(parsed)
}

/// Default frozen order set: `0, 0.5, 1, 1.5, 2, 3, 4, inf`.
pub fn default_q_tokens() -> Vec<String> {
    ["0", "0.5", "1", "1.5", "2", "3", "4", "inf"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_spec() -> GridSpec {
        GridSpec {
            c_min: -12.0,
            c_max: 2.0,
            c_step: 0.05,
            kappa_min: 1e-4,
            kappa_max: 4.0,
            kappa_points: 113,
        }
    }

    #[test]
    fn grid_extents_match_frozen_contract() {
        let g = make_grid(&default_spec()).unwrap();
        assert_eq!(g.len(), 31753);
        assert_eq!(g.c[0], -12.0);
        assert_eq!(*g.c.last().unwrap(), 2.0);
        assert!((g.kappa[0] - 0.00010000000000000009).abs() < 1e-18);
        assert!((*g.kappa.last().unwrap() - 4.0).abs() < 1e-12);
        // grid_index 100 falls in the first c block; kappa index 100
        assert_eq!(g.c[100], -12.0);
        assert!((g.kappa[100] - 1.2852337902249547).abs() < 1e-12);
    }

    #[test]
    fn q_values_parse_and_reject_duplicates() {
        let q = parse_q_values(&default_q_tokens()).unwrap();
        assert_eq!(q.len(), 8);
        assert_eq!(q[0], HillOrder::Finite(0.0));
        assert_eq!(q[7], HillOrder::Infinity);
        assert!(parse_q_values(&["1".into(), "1.0".into()]).is_err());
        assert!(parse_q_values(&["-0.5".into()]).is_err());
    }
}
