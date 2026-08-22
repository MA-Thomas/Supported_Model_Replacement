use std::path::PathBuf;

use clap::{Parser, Subcommand};
use external_validation_inputs::{Result, build_bundle, validate_bundle};

#[derive(Debug, Parser)]
#[command(
    name = "external_validation_inputs",
    about = "Build and validate deterministic IRIS full-roster external-validation inputs"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Build a new immutable input bundle from authoritative source files.
    Build {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        input_root: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Recompute hashes and structural invariants for an existing bundle.
    Validate {
        #[arg(long)]
        bundle: PathBuf,
    },
}

fn run() -> Result<()> {
    let args = Args::parse();
    let report = match args.command {
        Command::Build {
            config,
            input_root,
            output,
        } => build_bundle(&config, &input_root, &output)?,
        Command::Validate { bundle } => validate_bundle(&bundle)?,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("JSON serialization cannot fail")
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
