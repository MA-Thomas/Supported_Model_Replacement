//! Numeric core: Hill breadth, the self-gated adaptive aggregate, empirical
//! average precision / AUROC with threshold-block tie handling, and average
//! ranks. Every routine is a faithful port of the corresponding NumPy/SciPy
//! code in `select_adaptive_hillq_parameters_deprecated.py` /
//! `select_adaptive_hill2_parameters_deprecated.py`, computed in `f64` to match NumPy's
//! default dtype.

use crate::error::{Result, SelectionError};

/// Shared finite score for an uncomputed observation or an empty candidate
/// roster: `ln(1e-12)`. Keeping this finite is part of the analysis contract.
pub const FLOOR: f64 = -27.631_021_115_928_547;

/// Numerically stable logistic sigmoid, matching `scipy.special.expit`.
#[inline]
pub fn expit(x: f64) -> f64 {
    if x >= 0.0 {
        1.0 / (1.0 + (-x).exp())
    } else {
        let ex = x.exp();
        ex / (1.0 + ex)
    }
}

/// Neumaier compensated summation. NumPy code uses `math.fsum` in the hot spots;
/// Neumaier keeps us within a few ULP of that exact sum, which is far below any
/// tolerance that could change a rank or a metric.
pub fn neumaier_sum<I: IntoIterator<Item = f64>>(iter: I) -> f64 {
    let mut sum = 0.0_f64;
    let mut c = 0.0_f64;
    for x in iter {
        let t = sum + x;
        if sum.abs() >= x.abs() {
            c += (sum - t) + x;
        } else {
            c += (x - t) + sum;
        }
        sum = t;
    }
    sum + c
}

/// `max_i z_i + log sum_i exp(z_i - max)`, with the declared floor for an empty
/// roster (mirrors `analyze_adaptive_l2_deprecated.stable_lse`).
pub fn stable_lse(values: &[f64]) -> f64 {
    if values.is_empty() {
        return FLOOR;
    }
    let m = max_of(values);
    m + neumaier_sum(values.iter().map(|&v| (v - m).exp())).ln()
}

#[inline]
pub fn max_of(values: &[f64]) -> f64 {
    values.iter().copied().fold(f64::NEG_INFINITY, f64::max)
}

/// A Hill order `q in [0, inf]`. Kept as a typed value rather than a bare `f64`
/// so the `q = 0`, `q = 1`, and `q = inf` special cases are explicit and the
/// display/JSON encodings form the frozen artifact contract.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HillOrder {
    Finite(f64),
    Infinity,
}

impl HillOrder {
    /// Parse a CLI token such as `"0"`, `"0.5"`, `"inf"`.
    pub fn parse(raw: &str) -> Result<Self> {
        let token = raw.trim().to_ascii_lowercase();
        if matches!(token.as_str(), "inf" | "infinity" | "+inf" | "+infinity") {
            return Ok(HillOrder::Infinity);
        }
        let value: f64 = token
            .parse()
            .map_err(|_| SelectionError::msg(format!("invalid Hill order: {raw}")))?;
        if value.is_nan() || value < 0.0 {
            return Err(SelectionError::msg(format!(
                "Hill orders must be nonnegative or infinity: {raw}"
            )));
        }
        Ok(HillOrder::Finite(value))
    }

    /// Numeric value, with `Infinity -> f64::INFINITY`.
    pub fn value(&self) -> f64 {
        match self {
            HillOrder::Finite(v) => *v,
            HillOrder::Infinity => f64::INFINITY,
        }
    }

    /// Stable identifier: `"qinf"`, else `"q" + format(q)` with `-` -> `m`
    /// and `.` -> `p`.
    pub fn id(&self) -> String {
        match self {
            HillOrder::Infinity => "qinf".to_string(),
            HillOrder::Finite(v) => {
                let g = format!("{v}");
                format!("q{}", g.replace('-', "m").replace('.', "p"))
            }
        }
    }

    /// JSON encoding: the string `"infinity"` or the numeric value.
    pub fn json(&self) -> serde_json::Value {
        match self {
            HillOrder::Infinity => serde_json::Value::String("infinity".to_string()),
            HillOrder::Finite(v) => serde_json::json!(v),
        }
    }

    /// `1 / N_q(p)` for normalized weights `p` over `k` candidates.
    fn inverse_hill(&self, p: &[f64], k: usize) -> f64 {
        match self {
            HillOrder::Finite(q) if *q == 0.0 => 1.0 / (k as f64),
            HillOrder::Finite(q) if *q == 1.0 => {
                // entropy = -sum p*ln(p) over p>0 ; inverse = exp(-entropy)
                let entropy =
                    -neumaier_sum(p.iter().copied().filter(|&x| x > 0.0).map(|x| x * x.ln()));
                (-entropy).exp()
            }
            HillOrder::Infinity => max_of(p),
            HillOrder::Finite(q) => {
                let power_sum = neumaier_sum(p.iter().map(|&x| x.powf(*q)));
                let log_hill = power_sum.ln() / (1.0 - *q);
                (-log_hill).exp()
            }
        }
    }
}

/// Port of `hill_components`: returns `(maxima, admitted_bonus)` where
/// `admitted_bonus = B * (1 - 1/N_q)` per roster. `B = log sum exp(z - max)`.
///
/// Singletons and equal-weight plateaus are order-invariant; empty rosters
/// return `(FLOOR, 0)`.
pub fn hill_components(rosters: &[Vec<f64>], order: HillOrder) -> Result<(Vec<f64>, Vec<f64>)> {
    let mut maxima = Vec::with_capacity(rosters.len());
    let mut admitted = Vec::with_capacity(rosters.len());
    for values in rosters {
        if values.is_empty() {
            maxima.push(FLOOR);
            admitted.push(0.0);
            continue;
        }
        if values.iter().any(|v| !v.is_finite()) {
            return Err(SelectionError::msg(
                "candidate roster contains a nonfinite score",
            ));
        }
        let m = max_of(values);
        let relative: Vec<f64> = values.iter().map(|&v| (v - m).exp()).collect();
        let total = neumaier_sum(relative.iter().copied());
        let probabilities: Vec<f64> = relative.iter().map(|&r| r / total).collect();
        let inverse_hill = order.inverse_hill(&probabilities, values.len());
        let breadth = 1.0 - inverse_hill;
        let bonus = total.ln();
        maxima.push(m);
        admitted.push(breadth * bonus);
    }
    if admitted.iter().any(|&x| x < -1e-13) {
        return Err(SelectionError::msg(
            "adaptive Hill admitted bonus became negative",
        ));
    }
    for x in admitted.iter_mut() {
        if *x < 0.0 {
            *x = 0.0;
        }
    }
    Ok((maxima, admitted))
}

/// Solve the authoritative self-gated equation
///
/// `S = m + C_q * sigmoid((c - S) / kappa)`
///
/// by bisection on the certified interval `[m, m + C_q]`. The residual is
/// strictly increasing, so this interval contains exactly one root whenever
/// `C_q >= 0` and `kappa > 0`.
pub fn self_gated_score(
    maximum: f64,
    corroboration_offer: f64,
    c: f64,
    kappa: f64,
    absolute_tolerance: f64,
    max_iterations: usize,
) -> Result<f64> {
    if !maximum.is_finite()
        || !corroboration_offer.is_finite()
        || corroboration_offer < 0.0
        || !c.is_finite()
        || !kappa.is_finite()
        || kappa <= 0.0
        || !absolute_tolerance.is_finite()
        || absolute_tolerance <= 0.0
        || max_iterations == 0
    {
        return Err(SelectionError::msg("invalid self-gated solver inputs"));
    }
    if corroboration_offer == 0.0 {
        return Ok(maximum);
    }

    let mut lower = maximum;
    let mut upper = maximum + corroboration_offer;
    for _ in 0..max_iterations {
        if upper - lower <= absolute_tolerance {
            break;
        }
        let midpoint = lower + 0.5 * (upper - lower);
        let residual = midpoint - maximum - corroboration_offer * expit((c - midpoint) / kappa);
        if residual < 0.0 {
            lower = midpoint;
        } else {
            upper = midpoint;
        }
    }
    if upper - lower > absolute_tolerance {
        return Err(SelectionError::msg(
            "self-gated bisection did not converge within the declared iteration limit",
        ));
    }
    let score = lower + 0.5 * (upper - lower);
    if score < maximum - absolute_tolerance
        || score > maximum + corroboration_offer + absolute_tolerance
    {
        return Err(SelectionError::msg(
            "self-gated score escaped its certified interval",
        ));
    }
    Ok(score)
}

/// Empirical average precision and AUROC with threshold-block tie handling.
///
/// AP is ordinary empirical average precision (precision at each block =
/// cumulative TP / cumulative selected). AUROC gives half credit for tied
/// positive/negative pairs. Errors if either class is absent.
pub fn ap_auc(labels: &[i8], scores: &[f64]) -> Result<(f64, f64)> {
    let n = labels.len();
    debug_assert_eq!(n, scores.len());
    let positive_count = labels.iter().filter(|&&y| y == 1).count();
    let negative_count = n - positive_count;
    if positive_count == 0 || negative_count == 0 {
        return Err(SelectionError::msg(
            "metric task must contain both label classes",
        ));
    }
    // Stable descending order by score (equivalent to np.argsort(-scores, kind="stable")).
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        scores[b]
            .partial_cmp(&scores[a])
            .expect("scores are finite")
            .then(a.cmp(&b))
    });

    let p = positive_count as f64;
    let neg = negative_count as f64;
    let mut ap = 0.0_f64;
    let mut concordance = 0.0_f64;
    let mut cumulative_positive = 0.0_f64;
    let mut cumulative_negative = 0.0_f64;

    let mut i = 0usize;
    while i < n {
        let mut j = i + 1;
        while j < n && scores[order[j]] == scores[order[i]] {
            j += 1;
        }
        let mut block_positive = 0.0_f64;
        for k in i..j {
            if labels[order[k]] == 1 {
                block_positive += 1.0;
            }
        }
        let block_negative = (j - i) as f64 - block_positive;
        cumulative_positive += block_positive;
        cumulative_negative += block_negative;
        let cumulative_size = cumulative_positive + cumulative_negative;
        if block_positive > 0.0 {
            ap += (block_positive / p) * (cumulative_positive / cumulative_size);
        }
        concordance +=
            block_positive * (neg - cumulative_negative) + 0.5 * block_positive * block_negative;
        i = j;
    }
    let auc = concordance / (p * neg);
    Ok((ap, auc))
}

/// Average ranks with tie averaging, matching `scipy.stats.rankdata(method="average")`.
pub fn rankdata_average(a: &[f64]) -> Vec<f64> {
    let n = a.len();
    if n == 0 {
        return Vec::new();
    }
    let mut sorter: Vec<usize> = (0..n).collect();
    sorter.sort_by(|&i, &j| a[i].partial_cmp(&a[j]).expect("finite").then(i.cmp(&j)));
    let mut inv = vec![0usize; n];
    for (rank, &i) in sorter.iter().enumerate() {
        inv[i] = rank;
    }
    let sorted: Vec<f64> = sorter.iter().map(|&i| a[i]).collect();
    // obs[k] = start of a new tied block among the sorted values
    let mut obs = vec![false; n];
    obs[0] = true;
    for k in 1..n {
        obs[k] = sorted[k] != sorted[k - 1];
    }
    // dense ordinal (1-based) per sorted position
    let mut dense_sorted = vec![0usize; n];
    let mut d = 0usize;
    for k in 0..n {
        if obs[k] {
            d += 1;
        }
        dense_sorted[k] = d;
    }
    // count = flatnonzero(obs) followed by n
    let mut count: Vec<usize> = (0..n).filter(|&k| obs[k]).collect();
    count.push(n);
    let mut out = vec![0.0_f64; n];
    for i in 0..n {
        let dv = dense_sorted[inv[i]];
        out[i] = 0.5 * (count[dv] as f64 + count[dv - 1] as f64 + 1.0);
    }
    out
}

/// Descending fractional rank in `[0, 1]`: `(rankdata(-values) - 1) / (n - 1)`.
pub fn fractional_rank_desc(values: &[f64]) -> Vec<f64> {
    let n = values.len();
    let negated: Vec<f64> = values.iter().map(|&v| -v).collect();
    let ranks = rankdata_average(&negated);
    let denom = (n as f64) - 1.0;
    ranks.iter().map(|&r| (r - 1.0) / denom).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn floor_is_log_one_e_minus_twelve() {
        assert_eq!(FLOOR, 1e-12_f64.ln());
    }

    #[test]
    fn expit_matches_scipy_reference() {
        // spot values
        assert!(close(expit(0.0), 0.5, 0.0));
        assert!(close(expit(-2.152), 0.10406, 1e-4));
    }

    #[test]
    fn hill_components_match_frozen_reference() {
        // iso roster (0, -5, -5) across orders
        let rosters = vec![vec![0.0, -5.0, -5.0]];
        let refs = [
            (HillOrder::Finite(0.0), 0.008923934480965947),
            (HillOrder::Finite(0.5), 0.0033760360309972696),
            (HillOrder::Finite(1.0), 0.0010275437098013352),
            (HillOrder::Finite(2.0), 0.00035242688794046424),
            (HillOrder::Infinity, 0.00017798843932691072),
        ];
        for (order, expected) in refs {
            let (m, ab) = hill_components(&rosters, order).unwrap();
            assert!(close(m[0], 0.0, 0.0));
            assert!(
                close(ab[0], expected, 1e-12),
                "order {order:?}: {} vs {expected}",
                ab[0]
            );
        }
        // uneven roster (0, -1, -2, -4)
        let uneven = vec![vec![0.0, -1.0, -2.0, -4.0]];
        let uneven_refs = [
            (HillOrder::Finite(0.0), 0.3147874847365046),
            (HillOrder::Finite(0.5), 0.27624148333614335),
            (HillOrder::Finite(1.0), 0.2469336478444671),
            (HillOrder::Finite(2.0), 0.210500291078002),
            (HillOrder::Infinity, 0.14386500612466016),
        ];
        for (order, expected) in uneven_refs {
            let (_, ab) = hill_components(&uneven, order).unwrap();
            assert!(close(ab[0], expected, 1e-12), "uneven order {order:?}");
        }
    }

    #[test]
    fn hill_singleton_empty_and_plateau() {
        let rosters = vec![vec![], vec![-2.0], vec![-2.0, -2.0, -2.0], vec![-2.0; 10]];
        for order in [
            HillOrder::Finite(0.0),
            HillOrder::Finite(0.5),
            HillOrder::Finite(1.0),
            HillOrder::Finite(2.0),
            HillOrder::Infinity,
        ] {
            let (m, ab) = hill_components(&rosters, order).unwrap();
            assert_eq!(m[0], FLOOR);
            assert_eq!(ab[0], 0.0); // empty
            assert_eq!(m[1], -2.0);
            assert_eq!(ab[1], 0.0); // singleton
            assert!(close(ab[2], 0.7324081924454064, 1e-12)); // equal-weight plateau
            assert!(close(ab[3], 2.0723265836946414, 1e-12)); // dense equal plateau
        }
    }

    #[test]
    fn self_gated_score_satisfies_fixed_point_and_bounds() {
        let maxima = [0.0, -2.0, -2.0];
        let bonus = [0.0, 0.5277, 0.7324];
        for i in 0..3 {
            let s = self_gated_score(maxima[i], bonus[i], -1.0, 0.5, 1e-12, 64).unwrap();
            let residual = s - maxima[i] - bonus[i] * expit((-1.0 - s) / 0.5);
            assert!(residual.abs() <= 1e-12, "residual={residual}");
            assert!(s >= maxima[i] && s <= maxima[i] + bonus[i]);
        }
    }

    #[test]
    fn self_gated_score_reports_nonconvergence() {
        let err = self_gated_score(-2.0, 0.7, -1.0, 0.5, 1e-15, 1).unwrap_err();
        assert!(err.to_string().contains("did not converge"));
    }

    #[test]
    fn nonempty_all_floor_roster_follows_authoritative_equation() {
        let rosters = vec![vec![FLOOR; 3]];
        let (maxima, offer) = hill_components(&rosters, HillOrder::Finite(2.0)).unwrap();
        assert_eq!(maxima[0], FLOOR);
        assert!(offer[0] > 0.0);
        let score = self_gated_score(maxima[0], offer[0], -12.0, 1.0, 1e-12, 64).unwrap();
        assert!(score > FLOOR);
        assert!(score <= FLOOR + offer[0]);
    }

    #[test]
    fn ap_auc_matches_frozen_reference() {
        let labels = [1i8, 0, 1, 0, 0];
        let rows: [[f64; 5]; 3] = [
            [3.0, 2.0, 1.0, 1.0, 0.0],
            [1.0, 1.0, 1.0, 1.0, 1.0],
            [0.5, 2.0, 0.5, 3.0, 1.0],
        ];
        let expected_ap = [0.75, 0.4, 0.4];
        let expected_auc = [0.75, 0.5, 0.0];
        for r in 0..3 {
            let (ap, auc) = ap_auc(&labels, &rows[r]).unwrap();
            assert!(close(ap, expected_ap[r], 1e-12), "ap row {r}: {ap}");
            assert!(close(auc, expected_auc[r], 1e-12), "auc row {r}: {auc}");
        }
    }

    #[test]
    fn rankdata_matches_scipy() {
        let a = [0.9, 0.8, 0.8, 0.7, 0.9, 0.1];
        assert_eq!(rankdata_average(&a), vec![5.5, 3.5, 3.5, 2.0, 5.5, 1.0]);
        let neg: Vec<f64> = a.iter().map(|&x| -x).collect();
        assert_eq!(rankdata_average(&neg), vec![1.5, 3.5, 3.5, 5.0, 1.5, 6.0]);
    }

    #[test]
    fn hill_id_and_json() {
        assert_eq!(HillOrder::Finite(0.5).id(), "q0p5");
        assert_eq!(HillOrder::Finite(1.0).id(), "q1");
        assert_eq!(HillOrder::Infinity.id(), "qinf");
        assert_eq!(HillOrder::Infinity.json(), serde_json::json!("infinity"));
        assert_eq!(HillOrder::Finite(2.0).json(), serde_json::json!(2.0));
    }
}
