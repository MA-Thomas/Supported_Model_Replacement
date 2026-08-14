use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use directed_round_robin_organizer::audit::{audit_results, status_results};
use directed_round_robin_organizer::plan::{read_plan, write_plan};
use directed_round_robin_organizer::reduce::audit_reduction_for;
use directed_round_robin_organizer::{
    Error, Result, load_bundle, load_bundle_for_hashing, plan_with_match_target,
    plan_with_shard_count, reduce_tournament, run_shard, write_reduction,
};

#[derive(Debug, Parser)]
#[command(
    name = "directed_round_robin_organizer",
    version,
    about = "Complete deterministic supported-evidence round-robin tournaments"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    ValidateBundle(BundleArgs),
    BundleContentHash(BundleArgs),
    Plan(PlanArgs),
    RunShard(RunShardArgs),
    Status(StatusArgs),
    Reduce(ReduceArgs),
    Audit(AuditArgs),
}

#[derive(Debug, Args)]
struct BundleArgs {
    #[arg(long)]
    bundle: PathBuf,
}

#[derive(Debug, Args)]
struct PlanArgs {
    #[arg(long)]
    bundle: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(
        long,
        conflicts_with = "matches_per_shard",
        required_unless_present = "matches_per_shard"
    )]
    shards: Option<usize>,
    #[arg(long, conflicts_with = "shards", required_unless_present = "shards")]
    matches_per_shard: Option<usize>,
}

#[derive(Debug, Args)]
struct RunShardArgs {
    #[arg(long)]
    bundle: PathBuf,
    #[arg(long)]
    plan: PathBuf,
    #[arg(long)]
    shard_id: usize,
    #[arg(long)]
    results: PathBuf,
    #[arg(long, default_value_t = 1)]
    threads: usize,
}

#[derive(Debug, Args)]
struct StatusArgs {
    #[arg(long)]
    plan: PathBuf,
    #[arg(long)]
    results: PathBuf,
}

#[derive(Debug, Args)]
struct ReduceArgs {
    #[arg(long)]
    bundle: PathBuf,
    #[arg(long)]
    plan: PathBuf,
    #[arg(long)]
    results: PathBuf,
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Args)]
struct AuditArgs {
    #[arg(long)]
    bundle: PathBuf,
    #[arg(long)]
    plan: PathBuf,
    #[arg(long)]
    results: PathBuf,
    #[arg(long)]
    reduction: Option<PathBuf>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::ValidateBundle(args) => {
            let bundle = load_bundle(&args.bundle)?;
            print_json(&bundle.summary())
        }
        Command::BundleContentHash(args) => {
            let bundle = load_bundle_for_hashing(&args.bundle)?;
            println!("{}", bundle.manifest.bundle_content_hash);
            Ok(())
        }
        Command::Plan(args) => {
            let bundle = load_bundle(&args.bundle)?;
            let plan = match (args.shards, args.matches_per_shard) {
                (Some(shards), None) => plan_with_shard_count(&bundle, shards)?,
                (None, Some(target)) => plan_with_match_target(&bundle, target)?,
                _ => {
                    return Err(Error::InvalidPlan(
                        "exactly one sharding policy is required".into(),
                    ));
                }
            };
            write_plan(&plan, &args.output)?;
            print_json(&plan)
        }
        Command::RunShard(args) => {
            let bundle = load_bundle(&args.bundle)?;
            let plan = read_plan(&args.plan)?;
            let summary = run_shard(&bundle, &plan, args.shard_id, &args.results, args.threads)?;
            print_json(&summary)
        }
        Command::Status(args) => {
            let plan = read_plan(&args.plan)?;
            let status = status_results(&plan, &args.results)?;
            print_json(&status)
        }
        Command::Reduce(args) => {
            let bundle = load_bundle(&args.bundle)?;
            let plan = read_plan(&args.plan)?;
            let reduction = reduce_tournament(&bundle, &plan, &args.results)?;
            write_reduction(&reduction, &args.output)?;
            print_json(&reduction.selection)
        }
        Command::Audit(args) => {
            let bundle = load_bundle(&args.bundle)?;
            let plan = read_plan(&args.plan)?;
            let audit = audit_results(&bundle, &plan, &args.results)?;
            print_json(&audit)?;
            if !audit.complete {
                return Err(Error::Incomplete("audit did not pass".into()));
            }
            if let Some(reduction) = args.reduction {
                audit_reduction_for(&reduction, &plan)?;
            }
            Ok(())
        }
    }
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    serde_json::to_writer_pretty(std::io::stdout(), value).map_err(|source| Error::Json {
        path: "<stdout>".into(),
        source,
    })?;
    println!();
    Ok(())
}
