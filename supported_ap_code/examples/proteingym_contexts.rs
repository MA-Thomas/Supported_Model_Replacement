//! Independent single-assay assessments; no empirical aggregation operator.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use clap::Parser;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use supported_ap::{
    DirectionalNestedFiniteEvidence, FiniteEvidenceVerdict, MagnitudeThreshold,
    NestedRetainedEffectRow, PairedEvaluation, PolicyVerdict, Prevalence, PrevalenceDecisionBounds,
    ReplacementPolicy, RetainedEffect, SearchOptions, SupportOrder, SurvivalFloor,
    SurvivalRequirement, TargetPrevalences,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const MODELS: [&str; 3] = [
    "score_eve_ensemble",
    "score_esm1v_ensemble",
    "score_esm2_650m",
];
const PAIRS: [(usize, usize); 3] = [(1, 0), (2, 1), (2, 0)];

#[derive(Parser)]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output_dir: PathBuf,
    #[arg(long)]
    run_id: String,
    #[arg(long, default_value_t = 200)]
    replications: usize,
    #[arg(long, default_value_t = 20260917)]
    seed: u64,
    #[arg(long, default_value_t = SearchOptions::default().grid_points)]
    grid_points: usize,
    #[arg(long, default_value_t = SearchOptions::default().tolerance)]
    tolerance: f64,
    #[arg(long, default_value_t = SearchOptions::default().max_iterations)]
    max_iterations: usize,
    #[arg(long, default_value_t = 8)]
    refinement_draws: usize,
    #[arg(long, default_value_t = 0.01)]
    lower: f64,
    #[arg(long, default_value_t = 0.90)]
    upper: f64,
    #[arg(long, default_value_t = 2)]
    computational_order: usize,
    #[arg(long, default_value_t = 0.0)]
    delta: f64,
    #[arg(long, default_value_t = 0.0)]
    floor: f64,
    #[arg(long, default_value_t = 0.81)]
    gamma: f64,
}

#[derive(Default)]
struct Builder {
    rows: BTreeMap<String, (usize, bool, [f64; 3])>,
}
struct Context {
    id: String,
    labels: Vec<bool>,
    scores: [Vec<f64>; 3],
    clusters: Vec<Vec<usize>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Summary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    search: Option<PrevalenceDecisionBounds>,
    magnitude: f64,
    survival: f64,
    supported: bool,
}
#[derive(Debug, Serialize, Deserialize)]
struct Direction {
    candidate: String,
    incumbent: String,
    observed: RetainedEffect,
    legacy_observed: RetainedEffect,
    observed_gate: Summary,
    computational: Vec<RetainedEffect>,
    full: Option<Summary>,
    prefix: Option<Summary>,
    final_supported: bool,
}
#[derive(Debug, Serialize, Deserialize)]
struct Report {
    schema_version: u32,
    supported_ap_version: String,
    run_id: String,
    context_id: String,
    pair_index: usize,
    observation_count: usize,
    positive_count: usize,
    position_count: usize,
    replications: usize,
    prefix_replications: usize,
    rejected_missing_class_draws: usize,
    refined_computational_draws: usize,
    maximum_refinement_difference: f64,
    refinement_gate_disagreements: usize,
    refinement_floor_disagreements: usize,
    forward: Direction,
    reverse: Direction,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args = Args::parse();
    eprintln!("Rayon worker threads: {}", rayon::current_num_threads());
    if args.replications / 2 < args.computational_order || args.computational_order == 0 {
        return Err(
            "both the full and half-length lists must accommodate computational order".into(),
        );
    }
    if !args.delta.is_finite()
        || args.delta < 0.0
        || !args.floor.is_finite()
        || args.floor < 0.0
        || !args.gamma.is_finite()
        || args.gamma <= 0.0
        || args.gamma >= 1.0
    {
        return Err("invalid nonnegative magnitude/floor or open-unit survival threshold".into());
    }
    let target = TargetPrevalences::closed_interval(
        Prevalence::new(args.lower)?,
        Prevalence::new(args.upper)?,
    )?;
    let search = SearchOptions::new(args.grid_points, args.tolerance, args.max_iterations)?;
    let refined = SearchOptions::new(
        2 * args.grid_points - 1,
        args.tolerance / 10.0,
        args.max_iterations,
    )?;
    let contexts = read_contexts(&args.input)?;
    fs::create_dir_all(&args.output_dir)?;
    let jobs: Vec<_> = contexts
        .iter()
        .flat_map(|c| (0..PAIRS.len()).map(move |p| (c, p)))
        .collect();
    let completed = AtomicUsize::new(0);
    eprintln!(
        "Assessing {} contexts, {} unordered pairs, J={}",
        contexts.len(),
        jobs.len(),
        args.replications
    );
    jobs.par_iter()
        .enumerate()
        .try_for_each(|(index, &(context, pair))| -> Result<()> {
            let path = args.output_dir.join(format!(
                "context-{:03}-pair-{pair}.json",
                index / PAIRS.len()
            ));
            if path.exists() {
                let saved: Report = serde_json::from_reader(File::open(&path)?)?;
                if saved.schema_version != 2
                    || saved.supported_ap_version != supported_ap::PACKAGE_VERSION
                    || saved.run_id != args.run_id
                    || saved.context_id != context.id
                    || saved.pair_index != pair
                {
                    return Err(format!("incompatible checkpoint: {}", path.display()).into());
                }
            } else {
                let report = assess(context, pair, &args, &target, search, refined)?;
                let temporary = path.with_extension("json.tmp");
                serde_json::to_writer_pretty(File::create(&temporary)?, &report)?;
                fs::rename(temporary, &path)?;
            }
            let n = completed.fetch_add(1, Ordering::Relaxed) + 1;
            if n % 10 == 0 || n == jobs.len() {
                eprintln!("Completed {n}/{} pair-context assessments", jobs.len());
            }
            Ok(())
        })?;
    Ok(())
}
fn read_contexts(path: &PathBuf) -> Result<Vec<Context>> {
    let mut reader = csv::Reader::from_path(path)?;
    let headers = reader.headers()?.clone();
    let col = |name: &str| {
        headers
            .iter()
            .position(|h| h == name)
            .ok_or_else(|| format!("missing column {name}"))
    };
    let (id, variant, label) = (col("assay_id")?, col("variant_id")?, col("label")?);
    let model_cols = [col(MODELS[0])?, col(MODELS[1])?, col(MODELS[2])?];
    let mut grouped: BTreeMap<String, Builder> = BTreeMap::new();
    for record in reader.records() {
        let record = record?;
        let variant_id = record[variant].to_owned();
        let position = parse_position(&variant_id)?;
        let y = match &record[label] {
            "0" => false,
            "1" => true,
            _ => return Err("labels must be 0/1".into()),
        };
        let mut scores = [0.0_f64; 3];
        for (i, &column) in model_cols.iter().enumerate() {
            scores[i] = record[column].parse()?;
            if !scores[i].is_finite() {
                return Err("nonfinite score".into());
            }
        }
        if record[id].is_empty() {
            return Err("empty context ID".into());
        }
        if grouped
            .entry(record[id].to_owned())
            .or_default()
            .rows
            .insert(variant_id, (position, y, scores))
            .is_some()
        {
            return Err("duplicate context/variant key".into());
        }
    }
    if grouped.is_empty() {
        return Err("empty input".into());
    }
    grouped
        .into_iter()
        .map(|(id, builder)| {
            let mut labels = Vec::new();
            let mut scores = [Vec::new(), Vec::new(), Vec::new()];
            let mut clusters: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
            for (position, label, values) in builder.rows.into_values() {
                clusters.entry(position).or_default().push(labels.len());
                labels.push(label);
                for i in 0..3 {
                    scores[i].push(values[i]);
                }
            }
            if clusters.len() < 2 || !labels.contains(&true) || !labels.contains(&false) {
                return Err(
                    "every context requires both classes and at least two position clusters".into(),
                );
            }
            Ok(Context {
                id,
                labels,
                scores,
                clusters: clusters.into_values().collect(),
            })
        })
        .collect()
}
fn parse_position(variant: &str) -> Result<usize> {
    let b = variant.as_bytes();
    if b.len() < 3
        || !(b[0].is_ascii_alphabetic() || b[0] == b'*')
        || !(b[b.len() - 1].is_ascii_alphabetic() || b[b.len() - 1] == b'*')
        || !b[1..b.len() - 1].iter().all(u8::is_ascii_digit)
    {
        return Err("expected single-substitution identifier".into());
    }
    let position: usize = variant[1..variant.len() - 1].parse()?;
    if position == 0 {
        return Err("position must be positive".into());
    }
    Ok(position)
}
fn policy(args: &Args) -> Result<ReplacementPolicy> {
    Ok(ReplacementPolicy::new(
        MagnitudeThreshold::new(args.delta)?,
        SurvivalFloor::new(args.floor)?,
        SurvivalRequirement::new(args.gamma)?,
    ))
}
fn gate(effect: RetainedEffect, args: &Args) -> Summary {
    let survival = if effect.value > args.floor { 1.0 } else { 0.0 };
    let supported = effect.value > args.delta && survival > args.gamma;
    let search = effect.search.map(|bound| {
        let upper_survival = if bound.upper_bound > args.floor {
            1.0
        } else {
            0.0
        };
        let verdict = if supported {
            PolicyVerdict::VerifiedPass
        } else if bound.upper_bound <= args.delta || upper_survival <= args.gamma {
            PolicyVerdict::VerifiedFailure
        } else {
            PolicyVerdict::Unresolved
        };
        PrevalenceDecisionBounds {
            supported_magnitude_lower: effect.value,
            supported_magnitude_upper: bound.upper_bound,
            literal_survival_lower: survival,
            literal_survival_upper: upper_survival,
            verdict,
        }
    });
    Summary {
        magnitude: effect.value,
        survival,
        supported,
        search,
    }
}
fn full(anchor: RetainedEffect, draws: &[RetainedEffect], args: &Args) -> Result<Summary> {
    let support = DirectionalNestedFiniteEvidence::from_retained_effect_rows(
        vec![NestedRetainedEffectRow {
            observed: anchor,
            computational: draws.to_vec(),
        }],
        SupportOrder::new(1)?,
        SupportOrder::new(args.computational_order)?,
        policy(args)?,
    )?;
    Ok(Summary {
        magnitude: support.supported_magnitude,
        survival: support.literal_survival.subset_fraction,
        supported: support.verdict == FiniteEvidenceVerdict::SupportedReplacement,
        search: support.prevalence_search,
    })
}
fn direction(
    a: usize,
    b: usize,
    observed: RetainedEffect,
    legacy_observed: RetainedEffect,
    draws: Vec<RetainedEffect>,
    args: &Args,
) -> Result<Direction> {
    let observed_gate = gate(observed, args);
    let (complete, prefix) = if observed_gate.supported {
        (
            Some(full(observed, &draws, args)?),
            Some(full(observed, &draws[..args.replications / 2], args)?),
        )
    } else {
        (None, None)
    };
    let final_supported = complete.as_ref().is_some_and(|s| s.supported);
    Ok(Direction {
        candidate: MODELS[a].into(),
        incumbent: MODELS[b].into(),
        observed,
        legacy_observed,
        observed_gate,
        computational: draws,
        full: complete,
        prefix,
        final_supported,
    })
}
fn assess(
    context: &Context,
    pair: usize,
    args: &Args,
    target: &TargetPrevalences,
    search: SearchOptions,
    refined: SearchOptions,
) -> Result<Report> {
    let (a, b) = PAIRS[pair];
    let evaluation =
        PairedEvaluation::new(&context.scores[a], &context.scores[b], &context.labels)?;
    let unit_weights = vec![1; context.labels.len()];
    let observed = boundary_checked_effects(
        &evaluation,
        &unit_weights,
        target,
        search,
        args.lower,
        args.upper,
    )?;
    // The old report slot is kept for comparisons, but both now use the
    // bounded engine; do not perform the identical observed search twice.
    let legacy_observed = observed;
    let check = boundary_checked_effects(
        &evaluation,
        &unit_weights,
        target,
        refined,
        args.lower,
        args.upper,
    )?;
    let mut max_difference = (observed.0.value - check.0.value)
        .abs()
        .max((observed.1.value - check.1.value).abs());
    let gate_disagreements =
        usize::from(gate(observed.0, args).supported != gate(check.0, args).supported)
            + usize::from(gate(observed.1, args).supported != gate(check.1, args).supported);
    let forward_pass = gate(observed.0, args).supported;
    let reverse_pass = gate(observed.1, args).supported;
    let mut forward = Vec::new();
    let mut reverse = Vec::new();
    let mut redraws = 0;
    let mut refined_draws = 0;
    let mut floor_disagreements = 0;
    if forward_pass || reverse_pass {
        // Evenly spread diagnostic replications; selection does not inspect effects.
        let indices: BTreeSet<_> = (0..args.refinement_draws.min(args.replications))
            .map(|i| i * args.replications / args.refinement_draws.min(args.replications))
            .collect();
        for replicate in 0..args.replications {
            let (weights, rejected) = multiplicities(context, args.seed, replicate)?;
            redraws += rejected;
            let effects = boundary_checked_effects(
                &evaluation,
                &weights,
                target,
                search,
                args.lower,
                args.upper,
            )?;
            if indices.contains(&replicate) {
                let check = boundary_checked_effects(
                    &evaluation,
                    &weights,
                    target,
                    refined,
                    args.lower,
                    args.upper,
                )?;
                max_difference = max_difference
                    .max((effects.0.value - check.0.value).abs())
                    .max((effects.1.value - check.1.value).abs());
                floor_disagreements +=
                    usize::from((effects.0.value > args.floor) != (check.0.value > args.floor))
                        + usize::from(
                            (effects.1.value > args.floor) != (check.1.value > args.floor),
                        );
                refined_draws += 1;
            }
            // A failing orientation has no computational assessment. Both effects
            // are computed by the paired library; only eligible directions retained.
            if forward_pass {
                forward.push(effects.0);
            }
            if reverse_pass {
                reverse.push(effects.1);
            }
        }
    }
    Ok(Report {
        schema_version: 2,
        supported_ap_version: supported_ap::PACKAGE_VERSION.into(),
        run_id: args.run_id.clone(),
        context_id: context.id.clone(),
        pair_index: pair,
        observation_count: context.labels.len(),
        positive_count: context.labels.iter().filter(|&&x| x).count(),
        position_count: context.clusters.len(),
        replications: if forward_pass || reverse_pass {
            args.replications
        } else {
            0
        },
        prefix_replications: args.replications / 2,
        rejected_missing_class_draws: redraws,
        refined_computational_draws: refined_draws,
        maximum_refinement_difference: max_difference,
        refinement_gate_disagreements: gate_disagreements,
        refinement_floor_disagreements: floor_disagreements,
        forward: direction(a, b, observed.0, legacy_observed.0, forward, args)?,
        reverse: direction(b, a, observed.1, legacy_observed.1, reverse, args)?,
    })
}
// Continuous prevalence bounds, including endpoint cells, come from the core.
fn boundary_checked_effects(
    evaluation: &PairedEvaluation,
    weights: &[usize],
    target: &TargetPrevalences,
    search: SearchOptions,
    lower: f64,
    upper: f64,
) -> Result<(RetainedEffect, RetainedEffect)> {
    let _ = (lower, upper); // retained in this example's call signature
    Ok(evaluation.retained_effects_for_multiplicities(weights, target, search)?)
}

fn stable_seed(master: u64, context: &str, replicate: usize) -> u64 {
    // Explicit FNV-1a ID hash and SplitMix64: stable across processes and row order.
    let hash = context.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    splitmix64(splitmix64(master ^ hash ^ 0x5047434f4e544558) ^ replicate as u64)
}
fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}
fn multiplicities(context: &Context, master: u64, replicate: usize) -> Result<(Vec<usize>, usize)> {
    let mut rng = ChaCha8Rng::seed_from_u64(stable_seed(master, &context.id, replicate));
    let mut weights = vec![0; context.labels.len()];
    for rejected in 0..10000 {
        weights.fill(0);
        for _ in 0..context.clusters.len() {
            for &i in &context.clusters[rng.gen_range(0..context.clusters.len())] {
                weights[i] += 1;
            }
        }
        let positive = context
            .labels
            .iter()
            .zip(&weights)
            .any(|(&y, &w)| y && w > 0);
        let negative = context
            .labels
            .iter()
            .zip(&weights)
            .any(|(&y, &w)| !y && w > 0);
        if positive && negative {
            return Ok((weights, rejected));
        }
    }
    Err("10000 cluster draws failed to contain both classes".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn point(value: f64) -> RetainedEffect {
        RetainedEffect {
            value,
            limiting_prevalence: Prevalence::new(0.1).unwrap(),
            search: None,
        }
    }
    #[test]
    fn bounded_gate_keeps_budget_ambiguity_explicit() {
        let mut effect = point(-0.01);
        effect.search = Some(supported_ap::PrevalenceSearchCertificate {
            lower_bound: -0.01,
            upper_bound: 0.01,
            sampled_value: 0.01,
            evaluations: 4,
            iterations: 1,
            stop_reason: supported_ap::PrevalenceSearchStop::BudgetExhausted,
        });
        let summary = gate(effect, &args());
        assert!(!summary.supported);
        assert_eq!(summary.search.unwrap().verdict, PolicyVerdict::Unresolved);
    }
    fn args() -> Args {
        Args::parse_from([
            "test",
            "--input",
            "unused",
            "--output-dir",
            "unused",
            "--run-id",
            "test",
        ])
    }
    #[test]
    fn anchored_operator_matches_enumeration_and_strict_floor() {
        let args = args();
        let draws: Vec<_> = [0.1, 0.2, -0.05]
            .into_iter()
            .map(|value| RetainedEffect {
                value,
                limiting_prevalence: Prevalence::new(0.1).unwrap(),
                search: None,
            })
            .collect();
        let result = full(point(0.15), &draws, &args).unwrap();
        assert!(result.magnitude.abs() < 1e-12);
        assert!((result.survival - 1.0 / 3.0).abs() < 1e-12);
        assert_eq!(full(point(0.0), &draws, &args).unwrap().survival, 0.0);
        assert!(!gate(point(0.0), &args).supported);
        assert!(gate(point(0.001), &args).supported);
    }
    #[test]
    fn clusters_stay_together_and_draws_are_repeatable() {
        let c = Context {
            id: "synthetic".into(),
            labels: vec![true, false, true, false],
            scores: [vec![], vec![], vec![]],
            clusters: vec![vec![0, 1], vec![2, 3]],
        };
        for j in 0..20 {
            let a = multiplicities(&c, 17, j).unwrap();
            let b = multiplicities(&c, 17, j).unwrap();
            assert_eq!(a, b);
            assert_eq!(a.0[0], a.0[1]);
            assert_eq!(a.0[2], a.0[3]);
            assert_eq!(a.0.iter().sum::<usize>(), 4);
        }
        assert_ne!(
            stable_seed(17, "synthetic", 1),
            stable_seed(17, "different", 1)
        );
    }
    #[test]
    fn accepts_only_single_position_identifiers() {
        assert_eq!(parse_position("A12C").unwrap(), 12);
        for bad in ["", "A0C", "A1C:B2D", "12", "A-1C"] {
            assert!(parse_position(bad).is_err());
        }
    }
}
