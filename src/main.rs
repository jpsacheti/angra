use std::path::PathBuf;

use angra::{
    ResolveError,
    commands::{
        self, AddOptions, CommandError, ImportPomCommandOptions, InitOptions, LockOptions,
        RemoveOptions,
    },
    maven::{ArtifactCoordinate, ArtifactType, Scope},
};
use clap::{Parser, Subcommand, ValueEnum};

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
    /// Scaffold an angra.toml manifest.
    Init {
        /// Project directory where angra.toml should be written.
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,

        /// Project group ID.
        #[arg(long)]
        group: Option<String>,

        /// Project artifact ID.
        #[arg(long)]
        artifact: Option<String>,

        /// Project version.
        #[arg(long)]
        version: Option<String>,

        /// Replace an existing angra.toml.
        #[arg(long)]
        force: bool,
    },

    /// Add a dependency to angra.toml and update angra.lock.
    Add {
        /// Maven coordinate in group:artifact:version form.
        coordinate: String,

        /// Manifest alias to use under [dependencies].
        #[arg(long)]
        alias: Option<String>,

        /// Dependency scope.
        #[arg(long, value_enum, default_value_t = CliScope::Compile)]
        scope: CliScope,

        /// Maven artifact type.
        #[arg(long = "type", value_enum, default_value_t = CliArtifactType::Jar)]
        artifact_type: CliArtifactType,

        /// Optional Maven classifier.
        #[arg(long)]
        classifier: Option<String>,

        /// Exclusion in group:artifact form. May be repeated.
        #[arg(long = "exclude")]
        exclusions: Vec<String>,

        /// Project directory containing angra.toml.
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },

    /// Remove a dependency from angra.toml and update angra.lock.
    Remove {
        /// Manifest alias to remove from [dependencies].
        alias: String,

        /// Project directory containing angra.toml.
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },

    /// Resolve dependencies and update angra.lock.
    Lock {
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

    /// Print the resolved dependency graph.
    Tree {
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

    /// Print why an artifact is present in the resolved graph.
    Why {
        /// Maven coordinate in group:artifact or group:artifact:version form.
        coordinate: String,

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

    /// Import a Maven pom.xml into an angra.toml manifest.
    ImportPom {
        /// Path to the Maven pom.xml to import.
        path: PathBuf,

        /// Resolve imported POM parents/BOMs without fetching missing artifacts.
        #[arg(long)]
        offline: bool,

        /// Re-check remote POMs even when local files already exist.
        #[arg(long)]
        refresh: bool,

        /// Replace an existing angra.toml beside the POM.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliScope {
    Compile,
    Runtime,
    Test,
    Provided,
}

impl From<CliScope> for Scope {
    fn from(value: CliScope) -> Self {
        match value {
            CliScope::Compile => Self::Compile,
            CliScope::Runtime => Self::Runtime,
            CliScope::Test => Self::Test,
            CliScope::Provided => Self::Provided,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliArtifactType {
    Jar,
    Pom,
    War,
}

impl From<CliArtifactType> for ArtifactType {
    fn from(value: CliArtifactType) -> Self {
        match value {
            CliArtifactType::Jar => Self::Jar,
            CliArtifactType::Pom => Self::Pom,
            CliArtifactType::War => Self::War,
        }
    }
}

fn main() {
    if let Err(error) = run() {
        print_error(&error);
        std::process::exit(1);
    }
}

fn run() -> Result<(), CommandError> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init {
            project_dir,
            group,
            artifact,
            version,
            force,
        } => {
            let path = commands::init(InitOptions {
                project_dir,
                group,
                artifact,
                version,
                force,
            })?;
            println!(
                "{} wrote {}",
                paint("success:", GREEN),
                paint(&path.display().to_string(), CYAN)
            );
        }
        Command::Add {
            coordinate,
            alias,
            scope,
            artifact_type,
            classifier,
            exclusions,
            project_dir,
        } => {
            let output = commands::add(AddOptions {
                project_dir,
                coordinate,
                alias,
                scope: scope.into(),
                artifact_type: artifact_type.into(),
                classifier,
                exclusions,
            })?;
            print_warnings(&output.warnings);
            print_lock_success(output.lockfile.artifacts.len());
        }
        Command::Remove { alias, project_dir } => {
            let output = commands::remove(RemoveOptions { project_dir, alias })?;
            print_warnings(&output.warnings);
            print_lock_success(output.lockfile.artifacts.len());
        }
        Command::Lock {
            offline,
            refresh,
            project_dir,
        }
        | Command::Resolve {
            offline,
            refresh,
            project_dir,
        } => {
            let output = commands::lock(LockOptions {
                project_dir,
                offline,
                refresh,
            })?;
            print_warnings(&output.warnings);
            print_lock_success(output.lockfile.artifacts.len());
        }
        Command::Tree {
            offline,
            refresh,
            project_dir,
        } => {
            let output = commands::tree(LockOptions {
                project_dir,
                offline,
                refresh,
            })?;
            print_warnings(&output.warnings);
            println!("{}", output.text);
        }
        Command::Why {
            coordinate,
            offline,
            refresh,
            project_dir,
        } => {
            let output = commands::why(
                &coordinate,
                LockOptions {
                    project_dir,
                    offline,
                    refresh,
                },
            )?;
            print_warnings(&output.warnings);
            println!("{}", output.text);
        }
        Command::ImportPom {
            path,
            offline,
            refresh,
            force,
        } => {
            let output = commands::import_pom_manifest(ImportPomCommandOptions {
                pom_path: path,
                offline,
                refresh,
                force,
            })?;
            print_warnings(&output.warnings);
            println!(
                "{} wrote {}",
                paint("success:", GREEN),
                paint(&output.manifest_path.display().to_string(), CYAN)
            );
        }
    }

    Ok(())
}

fn print_lock_success(count: usize) {
    println!(
        "{} resolved {} artifacts into {}",
        paint("success:", GREEN),
        paint(&count.to_string(), BOLD),
        paint("angra.lock", CYAN)
    );
}

fn print_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!(
            "{} {warning}",
            paint("warning:", &format!("{BOLD}\x1b[33m"))
        );
    }
}

fn print_error(error: &CommandError) {
    let message = error
        .resolve_error()
        .map(ResolveError::root_cause)
        .map(ToString::to_string)
        .unwrap_or_else(|| error.to_string());
    eprintln!("{} {}", paint("error:", &format!("{BOLD}{RED}")), message);

    if let Some(resolve_error) = error.resolve_error()
        && let Some(path) = resolve_error.dependency_path()
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
