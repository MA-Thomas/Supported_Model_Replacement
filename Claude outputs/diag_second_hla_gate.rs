//! Diagnostic: how brittle is the second-HLA gate, and how concentrated is the
//! SPIKE improvement?
//!
//! This does NOT touch the CNAP / paired-bootstrap machinery. It recomputes the
//! two endpoint-local branches directly and reports:
//!
//!   1. the distribution of the second-distinct-HLA maximum m2 per cohort;
//!   2. gate saturation: how many endpoints sit in the sigmoid's transition
//!      band vs. saturated at 0 or 1 (with delta = 0.02 the gate is ~a step,
//!      so the transition count is the fragility measure);
//!   3. how many endpoints actually receive the HLA bonus (g_H > 0.5), split by
//!      label, and the endpoints that do;
//!   4. a transparent *raw observed AP* comparison of the peptide-only rule P
//!      against the convex hybrid S, plus a per-endpoint leave-the-bonus-out
//!      Delta-AP so you can see whether the gain rides on one or two endpoints;
//!   5. a wide sweep of the threshold t, showing how the gated-positive count
//!      and the raw AP gain move as t slides — i.e. whether t = -6.45 is a
//!      knife-edge.
//!
//! Raw observed AP here is a transparent proxy, NOT the CNAP "supported
//! magnitude" the selector uses. Directions and concentration are what to read
//! off it, not absolute numbers.
//!
//! Drop into: supported_ap_code/crates/select_adaptive_hillq/src/bin/
//! Run e.g.:
//!   cargo run --release --bin diag_second_hla_gate -- \
//!     --source-root /path/to/source_root
//! Optional overrides: --c -2.2 --kappa 0.13 --t -6.45 --delta 0.02 --w 0.12 --b 1.0
//!   --cohort SPIKE   (restrict to one cohort; default: all SELECTION_COHORTS)

use std::collections::HashMap;
use std::path::PathBuf;

use select_adaptive_hillq::config::SELECTION_COHORTS;
use select_adaptive_hillq::data::load_task;
use select_adaptive_hillq::error::{Result, SelectionError};
use select_adaptive_hillq::numeric::{
    HillOrder, PowerOrder, adaptive_components, expit, self_gated_score,
};

struct Params {
    source_root: PathBuf,
    c: f64,
    kappa: f64,
    t: f64,
    delta: f64,
    w: f64,
    b: f64,
    cohort: Option<String>,
}

fn parse_args() -> Result<Params> {
    let mut source_root: Option<PathBuf> = None;
    let mut c = -2.2;
    let mut kappa = 0.13;
    let mut t = -6.45;
    let mut delta = 0.02;
    let mut w = 0.12;
    let mut b = 1.0;
    let mut cohort: Option<String> = None;

    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut want = |name: &str| -> Result<f64> {
            it.next()
                .ok_or_else(|| SelectionError::msg(format!("missing value for {name}")))?
                .parse::<f64>()
                .map_err(|e| SelectionError::msg(format!("bad {name}: {e}")))
        };
        match flag.as_str() {
            "--source-root" => {
                source_root = Some(PathBuf::from(
                    it.next()
                        .ok_or_else(|| SelectionError::msg("missing value for --source-root"))?,
                ));
            }
            "--cohort" => {
                cohort = Some(
                    it.next()
                        .ok_or_else(|| SelectionError::msg("missing value for --cohort"))?,
                );
            }
            "--c" => c = want("--c")?,
            "--kappa" => kappa = want("--kappa")?,
            "--t" => t = want("--t")?,
            "--delta" => delta = want("--delta")?,
            "--w" => w = want("--w")?,
            "--b" => b = want("--b")?,
            other => return Err(SelectionError::msg(format!("unknown flag: {other}"))),
        }
    }

    Ok(Params {
        source_root: source_root
            .ok_or_else(|| SelectionError::msg("--source-root is required"))?,
        c,
        kappa,
        t,
        delta,
        w,
        b,
        cohort,
    })
}

/// Copied verbatim from explore_full_hla_hybrid_breadth.rs so behaviour matches.
fn collapsed_evidence(
    scores: &[f64],
    peptides: &[String],
    hlas: &[String],
) -> Result<(Vec<f64>, f64)> {
    if scores.is_empty() || scores.len() != peptides.len() || scores.len() != hlas.len() {
        return Err(SelectionError::msg(
            "invalid candidate/peptide/HLA hybrid roster",
        ));
    }
    let mut peptide_maxima: HashMap<&str, f64> = HashMap::new();
    let mut hla_maxima: HashMap<&str, f64> = HashMap::new();
    for ((&score, peptide), hla) in scores.iter().zip(peptides).zip(hlas) {
        peptide_maxima
            .entry(peptide)
            .and_modify(|value| *value = value.max(score))
            .or_insert(score);
        hla_maxima
            .entry(hla)
            .and_modify(|value| *value = value.max(score))
            .or_insert(score);
    }
    let mut hla_values: Vec<f64> = hla_maxima.into_values().collect();
    hla_values.sort_by(|a, b| b.total_cmp(a));
    let second_hla = hla_values
        .get(1)
        .copied()
        .ok_or_else(|| SelectionError::msg("hybrid rule requires two distinct HLAs"))?;
    Ok((peptide_maxima.into_values().collect(), second_hla))
}

/// Standard average precision from (score, label) pairs. Sorts by score desc,
/// stable; ties keep input order. Transparent proxy, not the CNAP statistic.
fn average_precision(scores: &[f64], labels: &[bool]) -> f64 {
    let n = scores.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
    let positives = labels.iter().filter(|&&l| l).count();
    if positives == 0 {
        return f64::NAN;
    }
    let (mut tp, mut fp, mut acc) = (0.0_f64, 0.0_f64, 0.0_f64);
    for &i in &idx {
        if labels[i] {
            tp += 1.0;
            acc += tp / (tp + fp);
        } else {
            fp += 1.0;
        }
    }
    acc / positives as f64
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let pos = q * (sorted.len() as f64 - 1.0);
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        sorted[lo]
    } else {
        let frac = pos - lo as f64;
        sorted[lo] * (1.0 - frac) + sorted[hi] * frac
    }
}

struct CohortData {
    labels: Vec<bool>,
    m: Vec<f64>,         // per-endpoint maximum (anchor)
    c_offer: Vec<f64>,   // per-endpoint breadth offer C
    p: Vec<f64>,         // per-endpoint peptide-only score P
    m2: Vec<f64>,        // per-endpoint second distinct-HLA maximum
    skipped: usize,      // endpoints without a second HLA
}

fn build_cohort(name: &str, pp: &Params) -> Result<CohortData> {
    let (task, _) = load_task(&pp.source_root, name, "full_hla", "pr")?;
    let labels_all = task.labels();
    let labels_all: Vec<bool> = labels_all.iter().map(|&l| l == 1).collect();

    let mut rosters: Vec<Vec<f64>> = Vec::new();
    let mut m2: Vec<f64> = Vec::new();
    let mut labels: Vec<bool> = Vec::new();
    let mut skipped = 0usize;

    for (i, ((scores, peptides), hlas)) in task
        .raw_candidates
        .iter()
        .zip(&task.raw_candidate_peptides)
        .zip(&task.raw_candidate_hlas)
        .enumerate()
    {
        match collapsed_evidence(scores, peptides, hlas) {
            Ok((roster, second)) => {
                rosters.push(roster);
                m2.push(second);
                labels.push(labels_all[i]);
            }
            Err(_) => skipped += 1,
        }
    }

    let (anchors, offers) =
        adaptive_components(&rosters, PowerOrder::Infinity, HillOrder::Infinity)?;
    let mut p = Vec::with_capacity(anchors.len());
    for (&anchor, &offer) in anchors.iter().zip(&offers) {
        p.push(self_gated_score(anchor, offer, pp.c, pp.kappa, 1e-10, 64)?);
    }

    Ok(CohortData {
        labels,
        m: anchors,
        c_offer: offers,
        p,
        m2,
        skipped,
    })
}

fn hybrid_scores(d: &CohortData, pp: &Params, t: f64) -> Vec<f64> {
    d.m
        .iter()
        .zip(&d.p)
        .zip(&d.m2)
        .map(|((&m, &p), &m2)| {
            let g_h = expit((m2 - t) / pp.delta);
            let h = m + pp.b * g_h;
            (1.0 - pp.w) * p + pp.w * h
        })
        .collect()
}

fn report_cohort(name: &str, pp: &Params) -> Result<()> {
    let d = build_cohort(name, pp)?;
    let n = d.labels.len();
    let pos = d.labels.iter().filter(|&&l| l).count();

    println!("\n================ {name} ================");
    println!(
        "endpoints scored: {n}   positives: {pos}   negatives: {}   skipped (no 2nd HLA): {}",
        n - pos,
        d.skipped
    );

    // (1) distribution of m2
    let mut m2_sorted = d.m2.clone();
    m2_sorted.sort_by(f64::total_cmp);
    println!(
        "m2 distribution: min {:.4}  q25 {:.4}  median {:.4}  q75 {:.4}  max {:.4}",
        quantile(&m2_sorted, 0.0),
        quantile(&m2_sorted, 0.25),
        quantile(&m2_sorted, 0.5),
        quantile(&m2_sorted, 0.75),
        quantile(&m2_sorted, 1.0),
    );

    // (2) gate saturation at the chosen (t, delta)
    let (mut off, mut on, mut trans) = (0usize, 0usize, 0usize);
    for &m2 in &d.m2 {
        let g = expit((m2 - pp.t) / pp.delta);
        if g < 1e-6 {
            off += 1;
        } else if g > 1.0 - 1e-6 {
            on += 1;
        } else {
            trans += 1;
        }
    }
    println!(
        "gate @ t={:.4}, delta={:.4}:  off(~0): {off}   on(~1): {on}   in-transition: {trans}",
        pp.t, pp.delta
    );

    // (3) how close to the threshold (in units of delta)
    for k in [1.0_f64, 3.0, 10.0] {
        let band = k * pp.delta;
        let near = d.m2.iter().filter(|&&m2| (m2 - pp.t).abs() <= band).count();
        println!("  |m2 - t| <= {k:>4.0}*delta ({band:.3}):  {near} endpoints");
    }

    // (4) endpoints receiving the bonus, split by label
    let gated: Vec<usize> = (0..n)
        .filter(|&i| expit((d.m2[i] - pp.t) / pp.delta) > 0.5)
        .collect();
    let gated_pos = gated.iter().filter(|&&i| d.labels[i]).count();
    println!(
        "endpoints with g_H > 0.5 (bonus active): {}   of which positive: {}   negative: {}",
        gated.len(),
        gated_pos,
        gated.len() - gated_pos
    );

    // (4/5) raw observed AP: peptide-only P vs convex hybrid S
    let s = hybrid_scores(&d, pp, pp.t);
    let ap_p = average_precision(&d.p, &d.labels);
    let ap_s = average_precision(&s, &d.labels);
    println!(
        "raw observed AP:  P(peptide-only) = {:.6}   S(hybrid) = {:.6}   delta = {:+.6}",
        ap_p,
        ap_s,
        ap_s - ap_p
    );

    // per-endpoint leave-the-bonus-out Delta-AP for gated endpoints
    if !gated.is_empty() {
        let mut contrib: Vec<(usize, f64)> = gated
            .iter()
            .map(|&i| {
                let mut s_off = s.clone();
                s_off[i] = d.p[i]; // remove this endpoint's HLA branch only
                let ap_off = average_precision(&s_off, &d.labels);
                (i, ap_s - ap_off)
            })
            .collect();
        contrib.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
        let show = contrib.len().min(12);
        println!("top gated endpoints by |Delta-AP| when their bonus is removed:");
        println!("   idx   label       m         m2        C      g_P      Delta-AP");
        for &(i, dap) in contrib.iter().take(show) {
            let g_p = expit((pp.c - d.p[i]) / pp.kappa);
            println!(
                "  {:>5}   {:>5}   {:>8.4}  {:>8.4}  {:>6.4}  {:>6.4}  {:+.6}",
                i,
                if d.labels[i] { "POS" } else { "neg" },
                d.m[i],
                d.m2[i],
                d.c_offer[i],
                g_p,
                dap
            );
        }
        let sum_abs: f64 = contrib.iter().map(|(_, x)| x.abs()).sum();
        let top1 = contrib.first().map(|(_, x)| x.abs()).unwrap_or(0.0);
        if sum_abs > 0.0 {
            println!(
                "  concentration: largest single |Delta-AP| is {:.1}% of the total |Delta-AP| mass",
                100.0 * top1 / sum_abs
            );
        }
    }

    // (5) wide sweep of t: does the AP gain ride on a knife-edge?
    println!("t-sweep (delta, w, B held fixed):");
    println!("       t      gated  gated_pos    AP_gain(S-P)");
    let mut tt = pp.t - 0.60;
    while tt <= pp.t + 0.60 + 1e-9 {
        let s_t = hybrid_scores(&d, pp, tt);
        let ap_t = average_precision(&s_t, &d.labels);
        let g_ct = d.m2.iter().filter(|&&m2| expit((m2 - tt) / pp.delta) > 0.5).count();
        let g_pt = (0..n)
            .filter(|&i| d.labels[i] && expit((d.m2[i] - tt) / pp.delta) > 0.5)
            .count();
        println!("  {:>8.4}   {:>5}    {:>5}      {:+.6}", tt, g_ct, g_pt, ap_t - ap_p);
        tt += 0.05;
    }

    Ok(())
}

fn run() -> Result<()> {
    let pp = parse_args()?;
    println!(
        "params: c={} kappa={} t={} delta={} w={} B={}",
        pp.c, pp.kappa, pp.t, pp.delta, pp.w, pp.b
    );
    match &pp.cohort {
        Some(name) => report_cohort(name, &pp)?,
        None => {
            for name in SELECTION_COHORTS {
                report_cohort(name, &pp)?;
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
