use std::path::PathBuf;

use angra::{ResolveError, ResolveOptions, maven::ArtifactCoordinate, resolve_project};
use clap::{Parser, Subcommand};

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";

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
        print_error(&error);
        std::process::exit(1);
    }
}

fn run() -> Result<(), ResolveError> {
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
                "{} resolved {} artifacts into {}",
                paint("success:", GREEN),
                paint(&lockfile.artifacts.len().to_string(), BOLD),
                paint("angra.lock", CYAN)
            );
        }
    }

    Ok(())
}

fn print_error(error: &ResolveError) {
    eprintln!(
        "{} {}",
        paint("error:", &format!("{BOLD}{RED}")),
        error.root_cause()
    );

    if let Some(path) = error.dependency_path()
        && !path.is_empty()
    {
        eprintln!();
        eprintln!("{}", paint("dependency path:", BOLD));
        eprintln!("  {}", format_dependency_path(path));
    }
}

fn format_dependency_path(path: &[ArtifactCoordinate]) -> String {
    path.iter()
        .map(|artifact| paint(&artifact.to_string(), CYAN))
        .collect::<Vec<_>>()
        .join(&format!(" {} ", paint("->", DIM)))
}

fn paint(value: &str, style: &str) -> String {
    format!("{style}{value}{RESET}")
}
