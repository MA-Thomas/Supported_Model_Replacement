//! Differential and property tests for the combinatorial AUROC solver.
//!
//! The combinatorial backend is checked against two independent references:
//! the HiGHS mixed-integer linearisation, and exhaustive enumeration of the
//! capped-simplex vertices. Agreement with both is the only reason to trust it
//! at a scale where neither reference can run.
//!
//! Cases deliberately covered:
//!
//! * `Gamma` just above one, where the exclusion-side relaxation is tight;
//! * mid-range `Gamma`, where neither relaxation is tight and branch and bound
//!   does the work;
//! * large `Gamma`, where residual weight is a material fraction of the mass —
//!   the regime where an earlier prototype produced an invalid bound by
//!   silently dropping the residual term;
//! * tie-heavy scores, since the gain matrix is built from half credits.

use supported_ap::{
    AuRocBackend, AuRocOptimizationOptions, ConcentrationFactor, PairedEvaluation,
    paired_auroc_difference, worst_case_auroc_difference,
};

const GAMMAS: [f64; 9] = [1.0, 1.05, 1.2, 1.5, 2.0, 2.5, 3.0, 4.0, 6.0];

fn options(backend: AuRocBackend) -> AuRocOptimizationOptions {
    AuRocOptimizationOptions {
        backend,
        ..AuRocOptimizationOptions::default()
    }
}

fn value(evaluation: &PairedEvaluation, gamma: f64, backend: AuRocBackend) -> f64 {
    worst_case_auroc_difference(
        evaluation,
        ConcentrationFactor::new(gamma).unwrap(),
        options(backend),
    )
    .unwrap()
    .point_estimate()
}

/// An evaluation together with the raw scores, since `PairedEvaluation` keeps
/// its score vectors crate-private and the reference implementation needs them.
struct Case {
    evaluation: PairedEvaluation,
    scores_a: Vec<f64>,
    scores_b: Vec<f64>,
    positives: usize,
    negatives: usize,
}

/// Deterministic pseudo-random evaluations; no dev-dependency on a RNG crate.
fn generated(seed: u64, positives: usize, negatives: usize, buckets: u64) -> Case {
    let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        // Coarse buckets deliberately produce ties.
        ((state >> 33) % buckets) as f64 / buckets as f64
    };
    let total = positives + negatives;
    let scores_a: Vec<f64> = (0..total).map(|_| next()).collect();
    let scores_b: Vec<f64> = (0..total).map(|_| next()).collect();
    let labels: Vec<bool> = (0..total).map(|index| index < positives).collect();
    let evaluation = PairedEvaluation::new(&scores_a, &scores_b, &labels).unwrap();
    Case {
        evaluation,
        scores_a,
        scores_b,
        positives,
        negatives,
    }
}

// ---------------------------------------------------------------------------
// Reference: exhaustive vertex enumeration
// ---------------------------------------------------------------------------

fn vertices(dimension: usize, gamma: f64) -> Vec<Vec<f64>> {
    let cap = (gamma / dimension as f64).min(1.0);
    let reciprocal = 1.0 / cap;
    let nearest = reciprocal.round();
    let full = if (reciprocal - nearest).abs() <= 1e-12 * reciprocal.abs().max(1.0) {
        nearest as usize
    } else {
        reciprocal.floor() as usize
    }
    .min(dimension);
    let residual = (1.0 - full as f64 * cap).max(0.0);
    let residual = if residual <= 1e-12 { 0.0 } else { residual };

    let mut out = Vec::new();
    let mut selected = Vec::new();
    fn walk(
        dimension: usize,
        full: usize,
        cap: f64,
        residual: f64,
        start: usize,
        selected: &mut Vec<usize>,
        out: &mut Vec<Vec<f64>>,
    ) {
        if selected.len() == full {
            if residual == 0.0 {
                let mut weights = vec![0.0; dimension];
                for &index in selected.iter() {
                    weights[index] = cap;
                }
                out.push(weights);
            } else {
                for extra in 0..dimension {
                    if selected.contains(&extra) {
                        continue;
                    }
                    let mut weights = vec![0.0; dimension];
                    for &index in selected.iter() {
                        weights[index] = cap;
                    }
                    weights[extra] = residual;
                    out.push(weights);
                }
            }
            return;
        }
        if full == 0 {
            return;
        }
        for index in start..=dimension.saturating_sub(full - selected.len()) {
            selected.push(index);
            walk(dimension, full, cap, residual, index + 1, selected, out);
            selected.pop();
        }
    }
    walk(dimension, full, cap, residual, 0, &mut selected, &mut out);
    out
}

fn brute_force(case: &Case, gamma: f64) -> f64 {
    let (positives, negatives) = (case.positives, case.negatives);
    let credit = |x: f64, y: f64| {
        if x > y {
            1.0
        } else if x == y {
            0.5
        } else {
            0.0
        }
    };
    let mut gain = vec![vec![0.0f64; negatives]; positives];
    for (i, row) in gain.iter_mut().enumerate() {
        for (k, cell) in row.iter_mut().enumerate() {
            let negative = positives + k;
            *cell = credit(case.scores_a[i], case.scores_a[negative])
                - credit(case.scores_b[i], case.scores_b[negative]);
        }
    }
    let mut best = f64::INFINITY;
    for left in vertices(positives, gamma) {
        for right in vertices(negatives, gamma) {
            let mut total = 0.0;
            for (i, &wi) in left.iter().enumerate() {
                if wi == 0.0 {
                    continue;
                }
                for (k, &wk) in right.iter().enumerate() {
                    total += wi * wk * gain[i][k];
                }
            }
            best = best.min(total);
        }
    }
    best
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
#[cfg(feature = "highs-reference")]
fn combinatorial_matches_highs_across_gamma_and_seeds() {
    for seed in 0..12u64 {
        let case = generated(seed, 3, 4, 5);
        for gamma in GAMMAS {
            let combinatorial = value(&case.evaluation, gamma, AuRocBackend::Combinatorial);
            let reference = value(&case.evaluation, gamma, AuRocBackend::Highs);
            assert!(
                (combinatorial - reference).abs() < 1e-6,
                "seed {seed}, Gamma {gamma}: combinatorial {combinatorial}, HiGHS {reference}"
            );
        }
    }
}

#[test]
fn combinatorial_matches_exhaustive_vertex_enumeration() {
    for seed in 100..108u64 {
        let case = generated(seed, 4, 4, 4);
        for gamma in GAMMAS {
            let combinatorial = value(&case.evaluation, gamma, AuRocBackend::Combinatorial);
            let exact = brute_force(&case, gamma);
            assert!(
                (combinatorial - exact).abs() < 1e-6,
                "seed {seed}, Gamma {gamma}: solver {combinatorial}, enumeration {exact}"
            );
        }
    }
}

#[test]
fn bounds_always_bracket_the_reference_value() {
    for seed in 200..212u64 {
        let case = generated(seed, 4, 5, 3);
        for gamma in GAMMAS {
            let certificate = worst_case_auroc_difference(
                &case.evaluation,
                ConcentrationFactor::new(gamma).unwrap(),
                options(AuRocBackend::Combinatorial),
            )
            .unwrap();
            let (lower, upper) = certificate.bounds();
            assert!(
                lower <= upper + 1e-12,
                "seed {seed}, Gamma {gamma}: inverted interval [{lower}, {upper}]"
            );
            let exact = brute_force(&case, gamma);
            assert!(
                lower <= exact + 1e-6,
                "seed {seed}, Gamma {gamma}: lower bound {lower} exceeds optimum {exact}"
            );
            assert!(
                upper >= exact - 1e-6,
                "seed {seed}, Gamma {gamma}: upper bound {upper} below optimum {exact}"
            );
        }
    }
}

#[test]
fn large_gamma_keeps_the_residual_term_exact() {
    // Residual weight is a material fraction of the mass once Gamma is large.
    // Dropping it silently produced an invalid bound in an earlier prototype,
    // so this range is checked against enumeration specifically.
    let case = generated(7, 5, 5, 6);
    for gamma in [3.0, 3.5, 4.0, 4.5, 5.0] {
        let combinatorial = value(&case.evaluation, gamma, AuRocBackend::Combinatorial);
        let exact = brute_force(&case, gamma);
        assert!(
            (combinatorial - exact).abs() < 1e-6,
            "Gamma {gamma}: solver {combinatorial}, enumeration {exact}"
        );
    }
}

#[test]
fn gamma_one_reproduces_the_ordinary_paired_difference() {
    for seed in 300..308u64 {
        let case = generated(seed, 4, 6, 5);
        let ordinary = paired_auroc_difference(&case.evaluation);
        let adversarial = value(&case.evaluation, 1.0, AuRocBackend::Combinatorial);
        assert!(
            (ordinary - adversarial).abs() < 1e-12,
            "seed {seed}: Gamma=1 value {adversarial} differs from {ordinary}"
        );
    }
}

#[test]
fn values_are_nonincreasing_in_gamma() {
    for seed in 400..410u64 {
        let case = generated(seed, 4, 5, 4);
        let mut previous = f64::INFINITY;
        for gamma in GAMMAS {
            let current = value(&case.evaluation, gamma, AuRocBackend::Combinatorial);
            assert!(
                current <= previous + 1e-9,
                "seed {seed}: value rose at Gamma {gamma} ({previous} -> {current})"
            );
            previous = current;
        }
    }
}

#[test]
fn tie_heavy_scores_are_handled_with_half_credit() {
    // Two buckets forces heavy tying in both models.
    for seed in 500..506u64 {
        let case = generated(seed, 4, 4, 2);
        for gamma in [1.0, 1.5, 2.0, 3.0] {
            let combinatorial = value(&case.evaluation, gamma, AuRocBackend::Combinatorial);
            let exact = brute_force(&case, gamma);
            assert!(
                (combinatorial - exact).abs() < 1e-6,
                "seed {seed}, Gamma {gamma}: solver {combinatorial}, enumeration {exact}"
            );
        }
    }
}

#[test]
#[cfg(feature = "highs-reference")]
fn reference_backend_refuses_full_scale_problems() {
    let case = generated(1, 80, 80, 20);
    let outcome = worst_case_auroc_difference(
        &case.evaluation,
        ConcentrationFactor::new(1.5).unwrap(),
        options(AuRocBackend::Highs),
    );
    assert!(
        outcome.is_err(),
        "the HiGHS reference must decline problems it cannot encode"
    );
    // The combinatorial backend handles the same problem.
    assert!(value(&case.evaluation, 1.5, AuRocBackend::Combinatorial).is_finite());
}
