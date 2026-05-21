use std::path::PathBuf;

use angra::{ResolveOptions, resolve_project};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "angra")]
#[command(about = "A fast, Maven-compatible Java project tool")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Resolve dependencies from angra.toml and write angra.lock.
    Resolve {
        /// Resolve without fetching missing artifacts.
        #[arg(long)]
        offline: bool,

        /// Re-check remote artifacts even when local files already exist.
        #[arg(long)]
        refresh: bool,

        /// Project directory containing angra.toml.
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Command::Resolve {
            offline,
            refresh,
            project_dir,
        } => {
            let lockfile = resolve_project(ResolveOptions {
                project_dir,
                offline,
                refresh,
                local_repo: None,
            })?;

            println!(
                "resolved {} artifacts into angra.lock",
                lockfile.artifacts.len()
            );
        }
    }

    Ok(())
}
