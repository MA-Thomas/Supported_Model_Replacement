//! Single-context ordinary AUROC (Gamma=1), followed by graph reduction.
//! Resampling is identical to the separate CNAP context study.
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use clap::Parser;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use supported_ap::{AnchoredEffectRow, SupportOrder, SurvivalFloor, anchored_nested_support};

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
    #[arg(long, default_value_t = 8)]
    check_draws: usize,
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
    magnitude: f64,
    survival: f64,
    supported: bool,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct Effect {
    value: f64,
}
#[derive(Debug, Serialize, Deserialize)]
struct Direction {
    candidate: String,
    incumbent: String,
    observed: Effect,
    observed_gate: Summary,
    computational: Vec<Effect>,
    full: Option<Summary>,
    prefix: Option<Summary>,
    final_supported: bool,
}
#[derive(Debug, Serialize, Deserialize)]
struct Report {
    schema_version: u32,
    run_id: String,
    context_id: String,
    pair_index: usize,
    observation_count: usize,
    positive_count: usize,
    position_count: usize,
    replications: usize,
    prefix_replications: usize,
    rejected_missing_class_draws: usize,
    independently_checked_draws: usize,
    maximum_check_difference: f64,
    check_gate_disagreements: usize,
    check_floor_disagreements: usize,
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
                if saved.schema_version != 1
                    || saved.run_id != args.run_id
                    || saved.context_id != context.id
                    || saved.pair_index != pair
                {
                    return Err(format!("incompatible checkpoint: {}", path.display()).into());
                }
            } else {
                let report = assess(context, pair, &args)?;
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
fn gate(effect: f64, args: &Args) -> Summary {
    let survival = if effect > args.floor { 1.0 } else { 0.0 };
    Summary {
        magnitude: effect,
        survival,
        supported: effect > args.delta && survival > args.gamma,
    }
}
fn full(anchor: f64, draws: &[Effect], args: &Args) -> Result<Summary> {
    // The library's one-row, empirical-order-one identity is exactly the main-text
    // single-context operator. No empirical rows are combined by this call.
    let support = anchored_nested_support(
        &[AnchoredEffectRow {
            observed_effect: anchor,
            computational_effects: draws.iter().map(|x| x.value).collect(),
        }],
        SupportOrder::new(1)?,
        SupportOrder::new(args.computational_order)?,
        SurvivalFloor::new(args.floor)?,
    )?;
    let magnitude = support.supported_magnitude;
    let survival = support.literal_survival.subset_fraction;
    Ok(Summary {
        magnitude,
        survival,
        supported: magnitude > args.delta && survival > args.gamma,
    })
}
fn direction(
    a: usize,
    b: usize,
    observed: Effect,
    draws: Vec<Effect>,
    args: &Args,
) -> Result<Direction> {
    let observed_gate = gate(observed.value, args);
    let (complete, prefix) = if observed_gate.supported {
        (
            Some(full(observed.value, &draws, args)?),
            Some(full(observed.value, &draws[..args.replications / 2], args)?),
        )
    } else {
        (None, None)
    };
    let final_supported = complete.as_ref().is_some_and(|s| s.supported);
    Ok(Direction {
        candidate: MODELS[a].into(),
        incumbent: MODELS[b].into(),
        observed,
        observed_gate,
        computational: draws,
        full: complete,
        prefix,
        final_supported,
    })
}
// Precompute tied score groups once; zero-weight rows can remain in groups.
fn score_groups(scores: &[f64]) -> Vec<Vec<usize>> {
    let mut indices: Vec<_> = (0..scores.len()).collect();
    indices.sort_by(|&a, &b| scores[a].partial_cmp(&scores[b]).unwrap());
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for i in indices {
        if groups.last().is_none_or(|g| scores[g[0]] != scores[i]) {
            groups.push(Vec::new());
        }
        groups.last_mut().unwrap().push(i);
    }
    groups
}
// Twice the concordant positive-negative mass, including half credit for ties.
// Integer arithmetic preserves exact zero differences and strict gate decisions.
fn twice_concordance(groups: &[Vec<usize>], labels: &[bool], weights: &[usize]) -> i128 {
    let mut below = 0_i128;
    let mut concordance = 0_i128;
    for group in groups {
        let mut positive = 0_i128;
        let mut negative = 0_i128;
        for &i in group {
            if labels[i] {
                positive += weights[i] as i128;
            } else {
                negative += weights[i] as i128;
            }
        }
        concordance += positive * (2 * below + negative);
        below += negative;
    }
    concordance
}
fn paired_effect(a: &[Vec<usize>], b: &[Vec<usize>], labels: &[bool], weights: &[usize]) -> f64 {
    let positive: i128 = labels
        .iter()
        .zip(weights)
        .filter(|(y, _)| **y)
        .map(|(_, w)| *w as i128)
        .sum();
    let negative: i128 = labels
        .iter()
        .zip(weights)
        .filter(|(y, _)| !**y)
        .map(|(_, w)| *w as i128)
        .sum();
    assert!(positive > 0 && negative > 0);
    let numerator = twice_concordance(a, labels, weights) - twice_concordance(b, labels, weights);
    numerator as f64 / (2 * positive * negative) as f64
}
// Independent check: expand sampled multiplicities and use average-rank AUROC.
fn expanded_rank_auc(scores: &[f64], labels: &[bool], weights: &[usize]) -> f64 {
    let mut expanded = Vec::new();
    for i in 0..labels.len() {
        for _ in 0..weights[i] {
            expanded.push((scores[i], labels[i]));
        }
    }
    expanded.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let positive = expanded.iter().filter(|x| x.1).count() as f64;
    let negative = expanded.len() as f64 - positive;
    let mut rank_sum = 0.0;
    let mut start = 0;
    while start < expanded.len() {
        let mut end = start + 1;
        while end < expanded.len() && expanded[end].0 == expanded[start].0 {
            end += 1;
        }
        let rank = (start + 1 + end) as f64 / 2.0;
        rank_sum += rank * expanded[start..end].iter().filter(|x| x.1).count() as f64;
        start = end;
    }
    (rank_sum - positive * (positive + 1.0) / 2.0) / (positive * negative)
}
fn assess(context: &Context, pair: usize, args: &Args) -> Result<Report> {
    let (a, b) = PAIRS[pair];
    let ga = score_groups(&context.scores[a]);
    let gb = score_groups(&context.scores[b]);
    let unit_weights = vec![1; context.labels.len()];
    let observed = paired_effect(&ga, &gb, &context.labels, &unit_weights);
    let independent = |weights: &[usize]| {
        expanded_rank_auc(&context.scores[a], &context.labels, weights)
            - expanded_rank_auc(&context.scores[b], &context.labels, weights)
    };
    let check = independent(&unit_weights);
    let mut max_difference = (observed - check).abs();
    let gate_disagreements =
        usize::from(gate(observed, args).supported != gate(check, args).supported)
            + usize::from(gate(-observed, args).supported != gate(-check, args).supported);
    let forward_pass = gate(observed, args).supported;
    let reverse_pass = gate(-observed, args).supported;
    let mut forward = Vec::new();
    let mut reverse = Vec::new();
    let mut redraws = 0;
    let mut checked = 0;
    let mut floor_disagreements = 0;
    let count = args.check_draws.min(args.replications);
    let indices: Vec<_> = (0..count).map(|i| i * args.replications / count).collect();
    if forward_pass || reverse_pass {
        for replicate in 0..args.replications {
            let (weights, rejected) = multiplicities(context, args.seed, replicate)?;
            redraws += rejected;
            let effect = paired_effect(&ga, &gb, &context.labels, &weights);
            if indices.contains(&replicate) {
                let check = independent(&weights);
                max_difference = max_difference.max((effect - check).abs());
                // Near zero, rank subtraction can round; exact integer numerator
                // defines the production decision and differences are also bounded.
                if effect.abs() > 1e-12 && check.abs() > 1e-12 {
                    floor_disagreements +=
                        usize::from((effect > args.floor) != (check > args.floor))
                            + usize::from((-effect > args.floor) != (-check > args.floor));
                }
                checked += 1;
            }
            if forward_pass {
                forward.push(Effect { value: effect });
            }
            if reverse_pass {
                reverse.push(Effect { value: -effect });
            }
        }
    }
    Ok(Report {
        schema_version: 1,
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
        independently_checked_draws: checked,
        maximum_check_difference: max_difference,
        check_gate_disagreements: gate_disagreements,
        check_floor_disagreements: floor_disagreements,
        forward: direction(a, b, Effect { value: observed }, forward, args)?,
        reverse: direction(b, a, Effect { value: -observed }, reverse, args)?,
    })
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
            .map(|value| Effect { value })
            .collect();
        let result = full(0.15, &draws, &args).unwrap();
        assert!(result.magnitude.abs() < 1e-12);
        assert!((result.survival - 1.0 / 3.0).abs() < 1e-12);
        assert_eq!(full(0.0, &draws, &args).unwrap().survival, 0.0);
        assert!(!gate(0.0, &args).supported);
        assert!(gate(0.001, &args).supported);
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
    fn weighted_ties_match_exhaustive_pair_counts_and_library() {
        use supported_ap::{PairedEvaluation, paired_auroc_difference};
        let labels = [true, false, true, false, true, false];
        let a = [0.0, -0.0, 2.0, 1.0, 1.0, 3.0];
        let b = [3.0, 1.0, 2.0, 1.0, 0.0, 0.0];
        let ga = score_groups(&a);
        let gb = score_groups(&b);
        let h = |x: f64, y: f64| {
            if x > y {
                1.0
            } else if x == y {
                0.5
            } else {
                0.0
            }
        };
        for code in 0..729_usize {
            let mut n = code;
            let mut w = vec![0; 6];
            for x in &mut w {
                *x = n % 3;
                n /= 3;
            }
            let mut numerator = 0.0;
            let mut denominator = 0.0;
            for i in 0..6 {
                for j in 0..6 {
                    if labels[i] && !labels[j] {
                        let mass = (w[i] * w[j]) as f64;
                        numerator += mass * (h(a[i], a[j]) - h(b[i], b[j]));
                        denominator += mass;
                    }
                }
            }
            if denominator == 0.0 {
                continue;
            }
            let actual = paired_effect(&ga, &gb, &labels, &w);
            assert!((actual - numerator / denominator).abs() < 1e-14);
            assert!(
                (actual
                    - (expanded_rank_auc(&a, &labels, &w) - expanded_rank_auc(&b, &labels, &w)))
                .abs()
                    < 1e-14
            );
            assert_eq!(paired_effect(&ga, &ga, &labels, &w), 0.0);
        }
        let evaluation = PairedEvaluation::new(&a, &b, &labels).unwrap();
        assert!(
            (paired_effect(&ga, &gb, &labels, &[1; 6]) - paired_auroc_difference(&evaluation))
                .abs()
                < 1e-14
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
