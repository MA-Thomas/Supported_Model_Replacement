use std::path::PathBuf;

use clap::{Parser, Subcommand};
use iris_fullroster_pipeline::Result;
use iris_fullroster_pipeline::bundle::{build_combined_bundles, build_fixed_bundles};
use iris_fullroster_pipeline::transfer::{build_transfer_package, validate_transfer_package};

#[derive(Debug, Parser)]
#[command(name = "iris_fullroster_pipeline", version)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    ValidateConfig {
        #[arg(long)]
        config: PathBuf,
    },
    TransferBatch {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    ValidateTransfers {
        #[arg(long)]
        package: PathBuf,
    },
    BuildFixedBundles {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        transfers: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    BuildCombinedBundles {
        #[arg(long)]
        base_bundles: PathBuf,
        #[arg(long)]
        selection: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
}

fn run() -> Result<()> {
    let report = match Args::parse().command {
        Command::ValidateConfig { config } => {
            let validated = iris_fullroster_pipeline::contract::PipelineConfig::load(&config)?;
            serde_json::json!({
                "status": "pass",
                "config": config,
                "models": validated.models.len(),
                "cohorts": validated.cohorts.len(),
                "fixed_l2": validated.fixed_l2.len(),
            })
        }
        Command::TransferBatch { config, output } => build_transfer_package(&config, &output)?,
        Command::ValidateTransfers { package } => validate_transfer_package(&package)?,
        Command::BuildFixedBundles {
            config,
            transfers,
            output,
        } => build_fixed_bundles(&config, &transfers, &output)?,
        Command::BuildCombinedBundles {
            base_bundles,
            selection,
            output,
        } => build_combined_bundles(&base_bundles, &selection, &output)?,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
