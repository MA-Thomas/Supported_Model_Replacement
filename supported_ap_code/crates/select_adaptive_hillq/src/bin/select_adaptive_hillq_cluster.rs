//! Cluster lifecycle for adaptive Hill-q paired-CNAP parameter selection.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use select_adaptive_hillq::cluster::{
    GridContract, PlanOptions, audit, create_plan, run_shard, status,
};
use select_adaptive_hillq::error::{Result, SelectionError};
use select_adaptive_hillq::grid::{GridSpec, default_q_tokens, parse_q_values};

#[derive(Parser, Debug)]
#[command(
    name = "select-adaptive-hillq-cluster",
    about = "Plan, shard, audit, and finalize adaptive Hill-q CNAP selection",
    allow_negative_numbers = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Plan(PlanArgs),
    RunShard(RunShardArgs),
    Status(StatusArgs),
    Audit(AuditArgs),
}

#[derive(Args, Debug)]
struct ThreadArgs {
    /// Rayon worker threads (default: all cores).
    #[arg(long)]
    threads: Option<usize>,
}

#[derive(Args, Debug)]
struct PlanArgs {
    #[arg(long)]
    source_root: PathBuf,
    #[arg(long)]
    bundle_root: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, value_delimiter = ' ', num_args = 1..)]
    q_values: Vec<String>,
    #[arg(long, default_value_t = -12.0, allow_hyphen_values = true)]
    c_min: f64,
    #[arg(long, default_value_t = 2.0, allow_hyphen_values = true)]
    c_max: f64,
    #[arg(long, default_value_t = 0.05)]
    c_step: f64,
    #[arg(long, default_value_t = 1e-4)]
    kappa_min: f64,
    #[arg(long, default_value_t = 4.0)]
    kappa_max: f64,
    #[arg(long, default_value_t = 113)]
    kappa_points: usize,
    #[arg(long, default_value_t = 1e-10)]
    solver_absolute_tolerance: f64,
    #[arg(long, default_value_t = 64)]
    solver_max_iterations: usize,
    /// Separate finite challenge roster used only for parameter selection.
    #[arg(long, default_value_t = 200)]
    selection_replications: usize,
    #[arg(long, default_value_t = 250)]
    matches_per_shard: usize,
    #[command(flatten)]
    threading: ThreadArgs,
}

#[derive(Args, Debug)]
struct RunShardArgs {
    #[arg(long)]
    source_root: PathBuf,
    #[arg(long)]
    bundle_root: PathBuf,
    #[arg(long)]
    plan: PathBuf,
    #[arg(long)]
    results: PathBuf,
    #[arg(long)]
    shard_id: usize,
    #[command(flatten)]
    threading: ThreadArgs,
}

#[derive(Args, Debug)]
struct StatusArgs {
    #[arg(long)]
    plan: PathBuf,
    #[arg(long)]
    results: PathBuf,
}

#[derive(Args, Debug)]
struct AuditArgs {
    #[arg(long)]
    source_root: PathBuf,
    #[arg(long)]
    bundle_root: PathBuf,
    #[arg(long)]
    plan: PathBuf,
    #[arg(long)]
    results: PathBuf,
}

fn configure_threads(threads: Option<usize>) -> Result<()> {
    if let Some(threads) = threads {
        if threads == 0 {
            return Err(SelectionError::msg("threads must be positive"));
        }
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .map_err(|error| SelectionError::msg(format!("failed to build Rayon pool: {error}")))?;
    }
    Ok(())
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Plan(mut args) => {
            configure_threads(args.threading.threads)?;
            if args.q_values.is_empty() {
                args.q_values = default_q_tokens();
            }
            let orders = parse_q_values(&args.q_values)?;
            let spec = GridSpec {
                c_min: args.c_min,
                c_max: args.c_max,
                c_step: args.c_step,
                kappa_min: args.kappa_min,
                kappa_max: args.kappa_max,
                kappa_points: args.kappa_points,
            };
            let manifest = create_plan(&PlanOptions {
                source_root: args.source_root,
                bundle_root: args.bundle_root,
                output: args.output,
                grid: GridContract::new(
                    &orders,
                    &spec,
                    args.solver_absolute_tolerance,
                    args.solver_max_iterations,
                ),
                selection_replications: args.selection_replications,
                matches_per_shard: args.matches_per_shard,
            })?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "plan_id": manifest.plan_id,
                    "unique_matches": manifest.unique_match_count,
                    "shards": manifest.shard_count,
                    "selection_replications": manifest.selection_replications,
                    "inherited_tournament_replications": manifest.inherited_tournament_replications,
                }))
                .expect("JSON summary is serializable")
            );
        }
        Command::RunShard(args) => {
            configure_threads(args.threading.threads)?;
            let count = run_shard(
                &args.source_root,
                &args.bundle_root,
                &args.plan,
                &args.results,
                args.shard_id,
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "shard_id": args.shard_id,
                    "completed_matches": count,
                }))
                .expect("JSON summary is serializable")
            );
        }
        Command::Status(args) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&status(&args.plan, &args.results)?).expect("status")
            );
        }
        Command::Audit(args) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&audit(
                    &args.source_root,
                    &args.bundle_root,
                    &args.plan,
                    &args.results,
                )?)
                .expect("audit")
            );
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
