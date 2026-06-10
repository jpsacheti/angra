use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    ResolveError, ResolveOptions,
    manifest::{
        DeclaredDependency, InitManifest, ManifestEditError, Project, add_dependency_to_manifest,
        default_alias, remove_dependency_from_manifest,
    },
    maven::{ArtifactCoordinate, ArtifactType, Coordinate, CoordinateError, Scope},
    resolver::{
        FrozenResolveOutput, OutdatedStatus, PomImportOptions, ResolveOutput, import_pom,
        inspect_project, outdated_project, resolve_frozen_project, resolve_project,
    },
};

#[derive(Debug, Clone)]
pub struct InitOptions {
    pub project_dir: PathBuf,
    pub group: Option<String>,
    pub artifact: Option<String>,
    pub version: Option<String>,
    pub force: bool,
}

#[derive(Debug, Clone)]
pub struct AddOptions {
    pub project_dir: PathBuf,
    pub coordinate: String,
    pub alias: Option<String>,
    pub scope: Scope,
    pub artifact_type: ArtifactType,
    pub classifier: Option<String>,
    pub exclusions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RemoveOptions {
    pub project_dir: PathBuf,
    pub alias: String,
}

#[derive(Debug, Clone)]
pub struct LockOptions {
    pub project_dir: PathBuf,
    pub offline: bool,
    pub refresh: bool,
}

#[derive(Debug, Clone)]
pub struct FrozenOptions {
    pub project_dir: PathBuf,
    pub offline: bool,
}

#[derive(Debug, Clone)]
pub struct ImportPomCommandOptions {
    pub pom_path: PathBuf,
    pub offline: bool,
    pub refresh: bool,
    pub force: bool,
}

#[derive(Debug, Clone)]
pub struct ImportPomCommandOutput {
    pub manifest_path: PathBuf,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TextOutput {
    pub text: String,
    pub warnings: Vec<String>,
}

pub fn init(options: InitOptions) -> Result<PathBuf, CommandError> {
    fs::create_dir_all(&options.project_dir)?;
    let manifest_path = manifest_path(&options.project_dir);
    let artifact = options
        .artifact
        .or_else(|| {
            options
                .project_dir
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "app".to_string());

    InitManifest {
        project: Project {
            group: options.group,
            artifact: Some(artifact),
            version: Some(options.version.unwrap_or_else(|| "0.1.0".to_string())),
        },
    }
    .write(&manifest_path, options.force)?;

    Ok(manifest_path)
}

pub fn add(options: AddOptions) -> Result<ResolveOutput, CommandError> {
    let coordinate = options.coordinate.parse::<Coordinate>()?;
    let alias = options
        .alias
        .unwrap_or_else(|| default_alias(&coordinate.artifact));
    let exclusions = options
        .exclusions
        .iter()
        .map(|exclusion| Coordinate::parse_without_version(exclusion))
        .collect::<Result<Vec<_>, _>>()?;

    add_dependency_to_manifest(
        &manifest_path(&options.project_dir),
        &DeclaredDependency {
            alias,
            artifact: ArtifactCoordinate::new(
                coordinate,
                options.artifact_type,
                options.classifier,
            ),
            scope: options.scope,
            exclusions,
        },
    )?;

    Ok(resolve_project(resolve_options(
        options.project_dir,
        false,
        false,
    ))?)
}

pub fn remove(options: RemoveOptions) -> Result<ResolveOutput, CommandError> {
    remove_dependency_from_manifest(&manifest_path(&options.project_dir), &options.alias)?;
    Ok(resolve_project(resolve_options(
        options.project_dir,
        false,
        false,
    ))?)
}

pub fn lock(options: LockOptions) -> Result<ResolveOutput, CommandError> {
    Ok(resolve_project(resolve_options(
        options.project_dir,
        options.offline,
        options.refresh,
    ))?)
}

pub fn resolve_frozen(options: FrozenOptions) -> Result<FrozenResolveOutput, CommandError> {
    Ok(resolve_frozen_project(ResolveOptions {
        project_dir: options.project_dir,
        offline: options.offline,
        refresh: false,
        local_repo: None,
    })?)
}

pub fn import_pom_manifest(
    options: ImportPomCommandOptions,
) -> Result<ImportPomCommandOutput, CommandError> {
    let project_dir = options
        .pom_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let manifest_path = manifest_path(&project_dir);
    if manifest_path.exists() && !options.force {
        return Err(ManifestEditError::AlreadyExists(manifest_path).into());
    }

    let output = import_pom(PomImportOptions {
        pom_path: options.pom_path.clone(),
        offline: options.offline,
        refresh: options.refresh,
        local_repo: None,
    })?;
    output.manifest.write(&manifest_path, options.force)?;
    Ok(ImportPomCommandOutput {
        manifest_path,
        warnings: output.warnings,
    })
}

pub fn tree(options: LockOptions) -> Result<TextOutput, CommandError> {
    let output = inspect_project(resolve_options(
        options.project_dir,
        options.offline,
        options.refresh,
    ))?;
    Ok(TextOutput {
        text: format_tree(&output),
        warnings: output.warnings,
    })
}

pub fn why(query: &str, options: LockOptions) -> Result<TextOutput, CommandError> {
    let query = WhyQuery::parse(query)?;
    let output = inspect_project(resolve_options(
        options.project_dir,
        options.offline,
        options.refresh,
    ))?;
    let Some(path) = output
        .graph
        .path_to(&query.group, &query.artifact, query.version.as_deref())
    else {
        return Err(CommandError::WhyNotFound(query.to_string()));
    };

    Ok(TextOutput {
        text: path
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" -> "),
        warnings: output.warnings,
    })
}

pub fn outdated(options: LockOptions) -> Result<TextOutput, CommandError> {
    let output = outdated_project(resolve_options(
        options.project_dir,
        options.offline,
        options.refresh,
    ))?;

    let mut warnings = output.warnings;
    let mut lines = Vec::new();
    for report in &output.reports {
        let coordinate = format!(
            "{}:{}",
            report.artifact.coordinate.group, report.artifact.coordinate.artifact
        );
        match &report.status {
            OutdatedStatus::Outdated { latest } => lines.push(format!(
                "{} ({coordinate}) {} -> {latest}",
                report.alias, report.artifact.coordinate.version
            )),
            OutdatedStatus::UpToDate => {}
            OutdatedStatus::Skipped { reason } => {
                warnings.push(format!(
                    "skipped `{}` ({coordinate}): {reason}",
                    report.alias
                ));
            }
            OutdatedStatus::NoMetadata => {
                warnings.push(format!(
                    "no version metadata found for `{}` ({coordinate})",
                    report.alias
                ));
            }
        }
    }

    let text = if output.reports.is_empty() {
        "(no dependencies declared)".to_string()
    } else if lines.is_empty() {
        "all dependencies are up to date".to_string()
    } else {
        lines.join("\n")
    };

    Ok(TextOutput { text, warnings })
}

fn resolve_options(project_dir: PathBuf, offline: bool, refresh: bool) -> ResolveOptions {
    ResolveOptions {
        project_dir,
        offline,
        refresh,
        local_repo: None,
    }
}

fn manifest_path(project_dir: &Path) -> PathBuf {
    project_dir.join("angra.toml")
}

fn format_tree(output: &ResolveOutput) -> String {
    if output.graph.artifacts.is_empty() {
        return "(no resolved artifacts)".to_string();
    }

    output
        .graph
        .artifacts
        .iter()
        .map(|entry| {
            format!(
                "{}{} [{}]",
                "  ".repeat(entry.depth),
                entry.artifact,
                entry.scope
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone)]
struct WhyQuery {
    group: String,
    artifact: String,
    version: Option<String>,
}

impl WhyQuery {
    fn parse(raw: &str) -> Result<Self, CommandError> {
        let parts = raw.split(':').collect::<Vec<_>>();
        if !(parts.len() == 2 || parts.len() == 3) || parts.iter().any(|part| part.is_empty()) {
            return Err(CommandError::InvalidWhyQuery(raw.to_string()));
        }

        Ok(Self {
            group: parts[0].to_string(),
            artifact: parts[1].to_string(),
            version: parts.get(2).map(|version| version.to_string()),
        })
    }
}

impl std::fmt::Display for WhyQuery {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.version {
            Some(version) => write!(formatter, "{}:{}:{version}", self.group, self.artifact),
            None => write!(formatter, "{}:{}", self.group, self.artifact),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error(transparent)]
    ManifestEdit(#[from] ManifestEditError),
    #[error(transparent)]
    Coordinate(#[from] CoordinateError),
    #[error("invalid why query `{0}`, expected group:artifact or group:artifact:version")]
    InvalidWhyQuery(String),
    #[error("artifact `{0}` was not found in the resolved graph")]
    WhyNotFound(String),
    #[error("failed filesystem operation: {0}")]
    Io(#[from] std::io::Error),
}

impl CommandError {
    pub fn resolve_error(&self) -> Option<&ResolveError> {
        match self {
            Self::Resolve(error) => Some(error),
            _ => None,
        }
    }
}
