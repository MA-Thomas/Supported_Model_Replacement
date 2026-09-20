use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use iris_nci_parameter_tournament::{
    PipelineConfig, audit_prepared, audit_prepared_for_config, audit_version2, finalize_version2,
    finalize_version2_accelerated, prepare,
};

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    ExportInputs {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    AuditInputs {
        #[arg(long)]
        package: PathBuf,
    },
    PrepareFinalists {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        prepared: PathBuf,
        #[arg(long)]
        version2: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    AuditFinalists {
        #[arg(long)]
        package: PathBuf,
    },
    SummarizeComponents {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        finalists: PathBuf,
        #[arg(long)]
        selections: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
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
        /// Require this exact configuration when reusing a prepared package.
        #[arg(long)]
        config: Option<PathBuf>,
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
    FinalizeVersion2Accelerated {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        results: PathBuf,
        #[arg(long)]
        selection: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        threads: Option<usize>,
    },
    AuditVersion2 {
        #[arg(long)]
        output: PathBuf,
        #[arg(long, requires = "bundle")]
        config: Option<PathBuf>,
        #[arg(long, requires = "config")]
        bundle: Option<PathBuf>,
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
        Command::ExportInputs { config, output } => {
            iris_nci_parameter_tournament::package::export_inputs(&config, &output)?
        }
        Command::AuditInputs { package } => {
            iris_nci_parameter_tournament::package::audit_inputs(&package)?
        }
        Command::PrepareFinalists {
            config,
            prepared,
            version2,
            output,
        } => iris_nci_parameter_tournament::finalists::prepare_finalists(
            &config, &prepared, &version2, &output,
        )?,
        Command::AuditFinalists { package } => {
            iris_nci_parameter_tournament::finalists::audit_finalists(&package)?;
        }
        Command::SummarizeComponents {
            config,
            finalists,
            selections,
            output,
        } => iris_nci_parameter_tournament::finalists::summarize_components(
            &config,
            &finalists,
            &selections,
            &output,
        )?,
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
        Command::AuditPrepared { package, config } => {
            if let Some(config) = config {
                audit_prepared_for_config(&package, &config)?;
            } else {
                audit_prepared(&package)?;
            }
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
        Command::FinalizeVersion2Accelerated {
            config,
            bundle,
            plan,
            results,
            selection,
            output,
            threads,
        } => {
            configure_rayon(threads)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&finalize_version2_accelerated(
                    &config, &bundle, &plan, &results, &selection, &output,
                )?)?
            );
        }
        Command::AuditVersion2 {
            output,
            config,
            bundle,
        } => {
            if let (Some(config), Some(bundle)) = (config, bundle) {
                let bundle = directed_round_robin_organizer::load_bundle(&bundle)
                    .map_err(anyhow::Error::msg)?;
                iris_nci_parameter_tournament::finalists::audit_selection_for(
                    &output, &config, &bundle,
                )?;
            } else {
                audit_version2(&output)?;
            }
            println!("Version 2 output is valid");
        }
    }
    Ok(())
}
