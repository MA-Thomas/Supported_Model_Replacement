use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use directed_round_robin_organizer::audit::{audit_results, status_results};
use directed_round_robin_organizer::plan::{read_plan, write_plan};
use directed_round_robin_organizer::reduce::audit_reduction_for;
use directed_round_robin_organizer::{
    Error, Result, SelectionStrategy, TournamentReductionSource, TournamentSource,
    audit_revision_composition_for, compose_revision, compose_revision_from_reductions,
    induce_reduction_view, load_bundle, load_bundle_for_hashing, load_reduction_for,
    plan_with_match_target, plan_with_shard_count, reduce_tournament, run_shard,
    write_induced_reduction_view, write_reduction, write_revision_composition,
};

/// CLI spelling of the reduction selection strategy (kebab-cased on the command
/// line: `candidate-conservative` / `replacement-conservative`).
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum SelectionStrategyArg {
    ReplacementConservative,
    CandidateConservative,
}

impl From<SelectionStrategyArg> for SelectionStrategy {
    fn from(value: SelectionStrategyArg) -> Self {
        match value {
            SelectionStrategyArg::ReplacementConservative => Self::ReplacementConservative,
            SelectionStrategyArg::CandidateConservative => Self::CandidateConservative,
        }
    }
}

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
    /// Plan, execute, or independently audit candidate-conservative certificates.
    Accelerated {
        #[command(subcommand)]
        command: AcceleratedCommand,
    },
    ValidateBundle(BundleArgs),
    BundleContentHash(BundleArgs),
    Plan(PlanArgs),
    RunShard(RunShardArgs),
    Status(StatusArgs),
    Reduce(ReduceArgs),
    InduceView(InduceViewArgs),
    Audit(AuditArgs),
    AuditReduction(AuditReductionArgs),
    ComposeRevision(ComposeRevisionArgs),
    ComposeRevisionFromReductions(ComposeRevisionFromReductionsArgs),
    AuditRevision(AuditRevisionArgs),
}

#[derive(Debug, Subcommand)]
enum AcceleratedCommand {
    /// Distributed stages share a single immutable accelerated plan.
    Distributed {
        #[arg(long, value_parser = ["targets", "merge", "complete", "finish"])]
        stage: String,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        results: PathBuf,
        #[arg(long)]
        receipts: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        survivors: Option<PathBuf>,
        #[arg(long)]
        shard_count: usize,
        #[arg(long, default_value_t = 0)]
        shard_id: usize,
        #[arg(long, default_value_t = 1)]
        threads: usize,
    },
    Plan {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 16)]
        batch_size: usize,
        /// Omit numerical reports needed by operational selection within S0.
        #[arg(long)]
        survivors_only: bool,
        /// Disable acceleration for these contexts; use complete SCC reduction.
        #[arg(long)]
        exhaustive_context: Vec<String>,
    },
    Run {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        results: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = 1)]
        threads: usize,
    },
    Audit {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        results: PathBuf,
        #[arg(long)]
        selection: PathBuf,
    },
}

fn accelerated(command: AcceleratedCommand) -> Result<()> {
    use directed_round_robin_organizer::accelerated::*;
    match command {
        AcceleratedCommand::Distributed {
            stage,
            bundle,
            plan,
            results,
            receipts,
            output,
            survivors,
            shard_count,
            shard_id,
            threads,
        } => {
            let bundle = load_bundle(&bundle)?;
            let plan = read_accelerated_plan(&plan)?;
            match stage.as_str() {
                "targets" => run_target_shard(
                    &bundle,
                    &plan,
                    &results,
                    &receipts,
                    shard_count,
                    shard_id,
                    threads,
                ),
                "merge" => {
                    merge_target_shards(&bundle, &plan, &results, &receipts, shard_count, &output)
                }
                "complete" | "finish" => {
                    let survivors = survivors.ok_or_else(|| {
                        directed_round_robin_organizer::Error::InvalidPlan(
                            "--survivors is required".into(),
                        )
                    })?;
                    if stage == "complete" {
                        run_completion_shard(
                            &bundle,
                            &plan,
                            &results,
                            &survivors,
                            &receipts,
                            shard_count,
                            shard_id,
                            threads,
                        )
                    } else {
                        finish_distributed(
                            &bundle,
                            &plan,
                            &results,
                            &survivors,
                            &receipts,
                            shard_count,
                            &output,
                        )
                    }
                }
                _ => unreachable!(),
            }
        }
        AcceleratedCommand::Plan {
            bundle,
            output,
            batch_size,
            survivors_only,
            exhaustive_context,
        } => {
            let bundle = load_bundle(&bundle)?;
            let plan = plan_accelerated(
                &bundle,
                AcceleratedOptions {
                    batch_size,
                    output_scope: if survivors_only {
                        OutputScope::SurvivorSet
                    } else {
                        OutputScope::SurvivorSetAndOperationalInputs
                    },
                    exhaustive_contexts: exhaustive_context.into_iter().collect(),
                },
            )?;
            write_accelerated_plan(&plan, &output)?;
            print_json(&plan)
        }
        AcceleratedCommand::Run {
            bundle,
            plan,
            results,
            output,
            threads,
        } => {
            let bundle = load_bundle(&bundle)?;
            let plan = read_accelerated_plan(&plan)?;
            print_json(&run_accelerated(
                &bundle, &plan, &results, &output, threads,
            )?)
        }
        AcceleratedCommand::Audit {
            bundle,
            plan,
            results,
            selection,
        } => {
            let bundle = load_bundle(&bundle)?;
            let plan = read_accelerated_plan(&plan)?;
            let certificate = read_certificate(&selection)?;
            print_json(&audit_selection(&bundle, &plan, &results, &certificate)?)
        }
    }
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
    #[arg(
        long,
        default_value_t = 1,
        help = "Size of the shared Rayon pool used across and within matches"
    )]
    threads: usize,
}

#[derive(Debug, Args)]
struct StatusArgs {
    #[arg(long)]
    plan: PathBuf,
    #[arg(long)]
    results: PathBuf,
    #[arg(long, default_value_t = 1)]
    threads: usize,
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
    #[arg(
        long,
        help = "Override the bundle's declared selection strategy for this reduction"
    )]
    selection_strategy: Option<SelectionStrategyArg>,
    #[arg(long, default_value_t = 1)]
    threads: usize,
}

#[derive(Debug, Args)]
struct InduceViewArgs {
    #[arg(long)]
    bundle: PathBuf,
    #[arg(long)]
    plan: PathBuf,
    #[arg(long)]
    reduction: PathBuf,
    #[arg(long, required = true, num_args = 1..)]
    exclude_system: Vec<String>,
    #[arg(long)]
    output: PathBuf,
    #[arg(
        long,
        help = "Override the bundle's declared selection strategy for the induced view"
    )]
    selection_strategy: Option<SelectionStrategyArg>,
    #[arg(long, default_value_t = 1)]
    threads: usize,
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
    #[arg(long, default_value_t = 1)]
    threads: usize,
}

#[derive(Debug, Args)]
struct AuditReductionArgs {
    #[arg(long)]
    bundle: PathBuf,
    #[arg(long)]
    plan: PathBuf,
    #[arg(long)]
    reduction: PathBuf,
    #[arg(long, default_value_t = 1)]
    threads: usize,
}

#[derive(Debug, Args)]
struct ComposeRevisionArgs {
    #[arg(long)]
    base_bundle: PathBuf,
    #[arg(long)]
    base_plan: PathBuf,
    #[arg(long)]
    base_results: PathBuf,
    #[arg(long)]
    revision_bundle: PathBuf,
    #[arg(long)]
    revision_plan: PathBuf,
    #[arg(long)]
    revision_results: PathBuf,
    #[arg(long)]
    replace_evaluation: String,
    #[arg(long)]
    revision_id: String,
    #[arg(long)]
    output: PathBuf,
}

#[derive(Debug, Args)]
struct ComposeRevisionFromReductionsArgs {
    #[arg(long)]
    base_bundle: PathBuf,
    #[arg(long)]
    base_plan: PathBuf,
    #[arg(long)]
    base_reduction: PathBuf,
    #[arg(long)]
    revision_bundle: PathBuf,
    #[arg(long)]
    revision_plan: PathBuf,
    #[arg(long)]
    revision_reduction: PathBuf,
    #[arg(long)]
    replace_evaluation: String,
    #[arg(long)]
    revision_id: String,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value_t = 1)]
    threads: usize,
}

#[derive(Debug, Args)]
struct AuditRevisionArgs {
    #[arg(long)]
    base_plan: PathBuf,
    #[arg(long)]
    revision_plan: PathBuf,
    #[arg(long)]
    replace_evaluation: String,
    #[arg(long)]
    revision_id: String,
    #[arg(long)]
    composition: PathBuf,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Accelerated { command } => accelerated(command),
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
            let status = with_pool(args.threads, || status_results(&plan, &args.results))?;
            print_json(&status)
        }
        Command::Reduce(args) => {
            let mut bundle = load_bundle(&args.bundle)?;
            if let Some(strategy) = args.selection_strategy {
                bundle.spec.selection_strategy = strategy.into();
            }
            let plan = read_plan(&args.plan)?;
            let reduction = with_pool(args.threads, || {
                reduce_tournament(&bundle, &plan, &args.results)
            })?;
            write_reduction(&reduction, &args.output)?;
            print_json(&reduction.selection)
        }
        Command::InduceView(args) => {
            let mut bundle = load_bundle(&args.bundle)?;
            if let Some(strategy) = args.selection_strategy {
                bundle.spec.selection_strategy = strategy.into();
            }
            let plan = read_plan(&args.plan)?;
            let reduction = with_pool(args.threads, || {
                load_reduction_for(&args.reduction, &bundle, &plan)
            })?;
            let view =
                induce_reduction_view(&bundle, &reduction, &args.reduction, &args.exclude_system)?;
            write_induced_reduction_view(&view, &args.output)?;
            print_json(&view.selection)
        }
        Command::Audit(args) => {
            let bundle = load_bundle(&args.bundle)?;
            let plan = read_plan(&args.plan)?;
            let audit = with_pool(args.threads, || {
                audit_results(&bundle, &plan, &args.results)
            })?;
            print_json(&audit)?;
            if !audit.complete {
                return Err(Error::Incomplete("audit did not pass".into()));
            }
            if let Some(reduction) = args.reduction {
                audit_reduction_for(&reduction, &plan)?;
            }
            Ok(())
        }
        Command::AuditReduction(args) => {
            let bundle = load_bundle(&args.bundle)?;
            let plan = read_plan(&args.plan)?;
            with_pool(args.threads, || {
                load_reduction_for(&args.reduction, &bundle, &plan).map(|_| ())
            })
        }
        Command::ComposeRevision(args) => {
            let base_bundle = load_bundle(&args.base_bundle)?;
            let base_plan = read_plan(&args.base_plan)?;
            let revision_bundle = load_bundle(&args.revision_bundle)?;
            let revision_plan = read_plan(&args.revision_plan)?;
            let composition = compose_revision(
                TournamentSource {
                    bundle: &base_bundle,
                    plan: &base_plan,
                    results: &args.base_results,
                },
                TournamentSource {
                    bundle: &revision_bundle,
                    plan: &revision_plan,
                    results: &args.revision_results,
                },
                &args.replace_evaluation,
                &args.revision_id,
            )?;
            write_revision_composition(&composition, &args.output)?;
            print_json(&composition.selection)
        }
        Command::ComposeRevisionFromReductions(args) => {
            let base_bundle = load_bundle(&args.base_bundle)?;
            let base_plan = read_plan(&args.base_plan)?;
            let revision_bundle = load_bundle(&args.revision_bundle)?;
            let revision_plan = read_plan(&args.revision_plan)?;
            let composition = with_pool(args.threads, || {
                let (base_reduction, revision_reduction) = rayon::join(
                    || load_reduction_for(&args.base_reduction, &base_bundle, &base_plan),
                    || {
                        load_reduction_for(
                            &args.revision_reduction,
                            &revision_bundle,
                            &revision_plan,
                        )
                    },
                );
                compose_revision_from_reductions(
                    TournamentReductionSource {
                        bundle: &base_bundle,
                        plan: &base_plan,
                        reduction: &base_reduction?,
                    },
                    TournamentReductionSource {
                        bundle: &revision_bundle,
                        plan: &revision_plan,
                        reduction: &revision_reduction?,
                    },
                    &args.replace_evaluation,
                    &args.revision_id,
                )
            })?;
            write_revision_composition(&composition, &args.output)?;
            print_json(&composition.selection)
        }
        Command::AuditRevision(args) => {
            let base_plan = read_plan(&args.base_plan)?;
            let revision_plan = read_plan(&args.revision_plan)?;
            audit_revision_composition_for(
                &args.composition,
                &base_plan,
                &revision_plan,
                &args.replace_evaluation,
                &args.revision_id,
            )
        }
    }
}

fn with_pool<T: Send>(threads: usize, operation: impl FnOnce() -> Result<T> + Send) -> Result<T> {
    if threads == 0 {
        return Err(Error::InvalidPlan(
            "finalization thread count must be positive".into(),
        ));
    }
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|index| format!("directed-round-robin-finalize-{index}"))
        .build()
        .map_err(|error| Error::InvalidPlan(format!("could not create Rayon pool: {error}")))?;
    pool.install(operation)
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    serde_json::to_writer_pretty(std::io::stdout(), value).map_err(|source| Error::Json {
        path: "<stdout>".into(),
        source,
    })?;
    println!();
    Ok(())
}
