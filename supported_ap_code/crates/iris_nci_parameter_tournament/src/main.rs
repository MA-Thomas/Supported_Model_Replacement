use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use iris_nci_parameter_tournament::{
    PipelineConfig, audit_prepared, audit_version2, finalize_version2, prepare,
};

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    ValidateConfig {
        #[arg(long)]
        config: PathBuf,
    },
    Prepare {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        threads: Option<usize>,
    },
    AuditPrepared {
        #[arg(long)]
        package: PathBuf,
    },
    FinalizeVersion2 {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        results: PathBuf,
        #[arg(long)]
        reduction: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        threads: Option<usize>,
    },
    AuditVersion2 {
        #[arg(long)]
        output: PathBuf,
    },
}

fn configure_rayon(threads: Option<usize>) -> Result<()> {
    if let Some(threads) = threads {
        anyhow::ensure!(threads > 0, "threads must be positive");
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|index| format!("nci-parameter-tournament-{index}"))
            .build_global()?;
    }
    Ok(())
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::ValidateConfig { config } => {
            PipelineConfig::load(&config)?;
            println!("configuration is valid");
        }
        Command::Prepare {
            config,
            output,
            threads,
        } => {
            configure_rayon(threads)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&prepare(&config, &output)?)?
            );
        }
        Command::AuditPrepared { package } => {
            audit_prepared(&package)?;
            println!("prepared package is valid");
        }
        Command::FinalizeVersion2 {
            config,
            bundle,
            plan,
            results,
            reduction,
            output,
            threads,
        } => {
            configure_rayon(threads)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&finalize_version2(
                    &config, &bundle, &plan, &results, &reduction, &output,
                )?)?
            );
        }
        Command::AuditVersion2 { output } => {
            audit_version2(&output)?;
            println!("Version 2 output is valid");
        }
    }
    Ok(())
}
