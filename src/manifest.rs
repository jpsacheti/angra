use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use indexmap::IndexMap;
use serde::Deserialize;
use toml::{Table, Value};

use crate::maven::{
    ArtifactCoordinate, ArtifactType, ChecksumPolicy, Coordinate, Repository, RepositoryPolicy,
    Scope,
};

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub project: Option<Project>,
    #[serde(default)]
    pub workspace: Workspace,
    #[serde(default)]
    pub resolver: ResolverConfig,
    #[serde(default)]
    pub repositories: IndexMap<String, RepositorySpec>,
    #[serde(default, rename = "dependency-management")]
    pub dependency_management: BTreeMap<String, ManagedDependencySpec>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, DependencySpec>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Project {
    pub group: Option<String>,
    pub artifact: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Workspace {
    #[serde(default)]
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ResolverConfig {
    #[serde(default)]
    pub maven: MavenResolverConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct MavenResolverConfig {
    #[serde(default)]
    pub active_profiles: Vec<String>,
    #[serde(default)]
    pub inactive_profiles: Vec<String>,
    #[serde(default)]
    pub java_version: Option<String>,
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum DependencySpec {
    Compact(String),
    Structured(StructuredDependency),
}

#[derive(Debug, Clone, Deserialize)]
pub struct StructuredDependency {
    pub group: String,
    pub artifact: String,
    pub version: String,
    #[serde(default, rename = "type")]
    pub artifact_type: ArtifactType,
    #[serde(default)]
    pub classifier: Option<String>,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub exclusions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ManagedDependencySpec {
    Compact(String),
    Structured(StructuredManagedDependency),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct StructuredManagedDependency {
    pub group: String,
    pub artifact: String,
    pub version: String,
    #[serde(default, rename = "type")]
    pub artifact_type: ArtifactType,
    #[serde(default)]
    pub classifier: Option<String>,
    #[serde(default)]
    pub scope: ManagedDependencyScope,
    #[serde(default)]
    pub exclusions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ManagedDependencyScope {
    #[default]
    None,
    Graph(Scope),
    Import,
}

impl<'de> Deserialize<'de> for ManagedDependencyScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(
            match Option::<String>::deserialize(deserializer)?.as_deref() {
                None | Some("") => Self::None,
                Some("compile") => Self::Graph(Scope::Compile),
                Some("runtime") => Self::Graph(Scope::Runtime),
                Some("test") => Self::Graph(Scope::Test),
                Some("provided") => Self::Graph(Scope::Provided),
                Some("import") => Self::Import,
                Some(scope) => {
                    return Err(serde::de::Error::custom(format!(
                        "unsupported dependency-management scope `{scope}`"
                    )));
                }
            },
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RepositorySpec {
    Compact(String),
    Structured(StructuredRepository),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct StructuredRepository {
    pub url: String,
    #[serde(default)]
    pub releases: Option<bool>,
    #[serde(default)]
    pub snapshots: Option<bool>,
    #[serde(default)]
    pub checksum_policy: Option<String>,
}

impl RepositorySpec {
    pub fn to_repository(&self, name: &str) -> Repository {
        match self {
            Self::Compact(url) => Repository::new(name, url),
            Self::Structured(repository) => {
                let releases = RepositoryPolicy {
                    enabled: repository.releases.unwrap_or(true),
                    checksum_policy: ChecksumPolicy::parse(repository.checksum_policy.as_deref()),
                };
                let snapshots = RepositoryPolicy {
                    enabled: repository.snapshots.unwrap_or(true),
                    checksum_policy: ChecksumPolicy::parse(repository.checksum_policy.as_deref()),
                };
                Repository::with_policy_details(name, &repository.url, releases, snapshots)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredDependency {
    pub alias: String,
    pub artifact: ArtifactCoordinate,
    pub scope: Scope,
    pub exclusions: Vec<Coordinate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredManagedDependency {
    pub alias: String,
    pub artifact: ArtifactCoordinate,
    pub scope: ManagedDependencyScope,
    pub exclusions: Vec<Coordinate>,
}

#[derive(Debug, Clone, Default)]
pub struct WritableManifest {
    pub project: Option<Project>,
    pub workspace_members: Vec<String>,
    pub repositories: Vec<Repository>,
    pub dependency_management: Vec<DeclaredManagedDependency>,
    pub dependencies: Vec<DeclaredDependency>,
}

#[derive(Debug, Clone)]
pub struct InitManifest {
    pub project: Project,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestEditError {
    #[error("manifest already exists at `{0}`; use --force to replace it")]
    AlreadyExists(PathBuf),
    #[error("manifest does not exist at `{0}`")]
    Missing(PathBuf),
    #[error("manifest table `{0}` is not a TOML table")]
    InvalidTable(String),
    #[error("dependency alias `{0}` already exists")]
    AliasExists(String),
    #[error("dependency alias `{0}` does not exist")]
    AliasMissing(String),
    #[error("failed filesystem operation: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse manifest TOML: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("failed to serialize manifest TOML: {0}")]
    TomlSer(#[from] toml::ser::Error),
}

impl Manifest {
    pub fn read(path: &Path) -> Result<Self, ManifestError> {
        let raw = fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    }

    pub fn declared_dependencies(&self) -> Result<Vec<DeclaredDependency>, ManifestError> {
        self.dependencies
            .iter()
            .map(|(alias, spec)| {
                let (artifact, scope, exclusions) = match spec {
                    DependencySpec::Compact(raw) => (
                        ArtifactCoordinate::jar(raw.parse()?),
                        Scope::Compile,
                        Vec::new(),
                    ),
                    DependencySpec::Structured(dep) => {
                        let exclusions = dep
                            .exclusions
                            .iter()
                            .map(|exclusion| Coordinate::parse_without_version(exclusion))
                            .collect::<Result<Vec<_>, _>>()?;

                        (
                            ArtifactCoordinate::new(
                                Coordinate::new(&dep.group, &dep.artifact, &dep.version),
                                dep.artifact_type,
                                dep.classifier.clone(),
                            ),
                            dep.scope,
                            exclusions,
                        )
                    }
                };

                Ok(DeclaredDependency {
                    alias: alias.clone(),
                    artifact,
                    scope,
                    exclusions,
                })
            })
            .collect()
    }

    pub fn declared_dependency_management(
        &self,
    ) -> Result<Vec<DeclaredManagedDependency>, ManifestError> {
        self.dependency_management
            .iter()
            .map(|(alias, spec)| {
                let (artifact, scope, exclusions) = match spec {
                    ManagedDependencySpec::Compact(raw) => (
                        ArtifactCoordinate::jar(raw.parse()?),
                        ManagedDependencyScope::None,
                        Vec::new(),
                    ),
                    ManagedDependencySpec::Structured(dep) => {
                        let exclusions = dep
                            .exclusions
                            .iter()
                            .map(|exclusion| Coordinate::parse_without_version(exclusion))
                            .collect::<Result<Vec<_>, _>>()?;

                        (
                            ArtifactCoordinate::new(
                                Coordinate::new(&dep.group, &dep.artifact, &dep.version),
                                dep.artifact_type,
                                dep.classifier.clone(),
                            ),
                            dep.scope,
                            exclusions,
                        )
                    }
                };

                Ok(DeclaredManagedDependency {
                    alias: alias.clone(),
                    artifact,
                    scope,
                    exclusions,
                })
            })
            .collect()
    }

    /// Hash the resolver-relevant manifest intent for `angra.lock` drift detection.
    ///
    /// Computed from parsed declarations, not raw TOML text, so formatting and
    /// comment edits do not invalidate a lockfile. Covers dependencies,
    /// dependency management, project repositories, and `[resolver.maven]`
    /// controls; machine-global state (global config, Maven settings) is
    /// excluded so lockfiles stay portable across machines.
    pub fn resolver_fingerprint(&self) -> Result<String, ManifestError> {
        use sha2::{Digest, Sha256};

        fn exclusion_list(exclusions: &[Coordinate]) -> String {
            exclusions
                .iter()
                .map(|exclusion| format!("{}:{}", exclusion.group, exclusion.artifact))
                .collect::<Vec<_>>()
                .join(",")
        }

        let mut canonical = String::from("angra-manifest-fingerprint v1\n");
        for dependency in self.declared_dependencies()? {
            canonical.push_str(&format!(
                "dependency {} {} scope={} exclusions={}\n",
                dependency.alias,
                dependency.artifact,
                dependency.scope,
                exclusion_list(&dependency.exclusions),
            ));
        }
        for managed in self.declared_dependency_management()? {
            let scope = match managed.scope {
                ManagedDependencyScope::None => "none".to_string(),
                ManagedDependencyScope::Graph(scope) => scope.to_string(),
                ManagedDependencyScope::Import => "import".to_string(),
            };
            canonical.push_str(&format!(
                "managed {} {} scope={scope} exclusions={}\n",
                managed.alias,
                managed.artifact,
                exclusion_list(&managed.exclusions),
            ));
        }
        for (name, spec) in &self.repositories {
            let repository = spec.to_repository(name);
            canonical.push_str(&format!(
                "repository {} {} releases={}/{} snapshots={}/{}\n",
                repository.name,
                repository.url,
                repository.releases.enabled,
                repository.releases.checksum_policy.as_token(),
                repository.snapshots.enabled,
                repository.snapshots.checksum_policy.as_token(),
            ));
        }
        let maven = &self.resolver.maven;
        for profile in &maven.active_profiles {
            canonical.push_str(&format!("active-profile {profile}\n"));
        }
        for profile in &maven.inactive_profiles {
            canonical.push_str(&format!("inactive-profile {profile}\n"));
        }
        if let Some(java_version) = &maven.java_version {
            canonical.push_str(&format!("java-version {java_version}\n"));
        }
        for (key, value) in &maven.properties {
            canonical.push_str(&format!("property {key}={value}\n"));
        }

        Ok(faster_hex::hex_string(&Sha256::digest(
            canonical.as_bytes(),
        )))
    }

    /// Return repositories with global config and Maven settings merged in.
    ///
    /// Precedence by name: project repos override global repos, which override settings repos.
    /// Order: globals appear first in declaration order, followed by unmatched project repos,
    /// followed by unmatched settings repos as a compatibility tail.
    /// If neither project, global, nor settings define any repos, Maven Central is returned.
    pub fn declared_repositories(
        &self,
        global: &[Repository],
        settings: &[Repository],
    ) -> Vec<Repository> {
        if self.repositories.is_empty() && global.is_empty() && settings.is_empty() {
            return vec![Repository::maven_central()];
        }

        let mut merged = global.to_vec();
        for (name, spec) in &self.repositories {
            let repository = spec.to_repository(name);
            if let Some(existing) = merged
                .iter_mut()
                .find(|existing| existing.name == repository.name)
            {
                *existing = repository;
            } else {
                merged.push(repository);
            }
        }

        for repository in settings {
            if !merged
                .iter()
                .any(|existing| existing.name == repository.name)
            {
                merged.push(repository.clone());
            }
        }

        merged
    }
}

impl WritableManifest {
    pub fn write(&self, path: &Path, force: bool) -> Result<(), ManifestEditError> {
        if path.exists() && !force {
            return Err(ManifestEditError::AlreadyExists(path.to_path_buf()));
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        write_table(path, self.to_table()?)
    }

    fn to_table(&self) -> Result<Table, ManifestEditError> {
        let mut root = Table::new();

        if let Some(project) = &self.project {
            let mut table = Table::new();
            insert_string_if_present(&mut table, "group", project.group.as_deref());
            insert_string_if_present(&mut table, "artifact", project.artifact.as_deref());
            insert_string_if_present(&mut table, "version", project.version.as_deref());
            if !table.is_empty() {
                root.insert("project".to_string(), Value::Table(table));
            }
        }

        if !self.workspace_members.is_empty() {
            let mut table = Table::new();
            table.insert(
                "members".to_string(),
                Value::Array(
                    self.workspace_members
                        .iter()
                        .map(|member| Value::String(member.clone()))
                        .collect(),
                ),
            );
            root.insert("workspace".to_string(), Value::Table(table));
        }

        if !self.repositories.is_empty() {
            let mut table = Table::new();
            for repository in &self.repositories {
                table.insert(
                    repository.name.clone(),
                    repository_value(repository)
                        .unwrap_or_else(|| Value::String(repository.url.clone())),
                );
            }
            root.insert("repositories".to_string(), Value::Table(table));
        }

        if !self.dependency_management.is_empty() {
            let mut table = Table::new();
            for dependency in &self.dependency_management {
                table.insert(
                    dependency.alias.clone(),
                    managed_dependency_value(dependency),
                );
            }
            root.insert("dependency-management".to_string(), Value::Table(table));
        }

        if !self.dependencies.is_empty() {
            let mut table = Table::new();
            for dependency in &self.dependencies {
                table.insert(dependency.alias.clone(), dependency_value(dependency));
            }
            root.insert("dependencies".to_string(), Value::Table(table));
        }

        Ok(root)
    }
}

impl InitManifest {
    pub fn write(&self, path: &Path, force: bool) -> Result<(), ManifestEditError> {
        WritableManifest {
            project: Some(self.project.clone()),
            ..WritableManifest::default()
        }
        .write(path, force)
    }
}

pub fn add_dependency_to_manifest(
    path: &Path,
    dependency: &DeclaredDependency,
) -> Result<(), ManifestEditError> {
    let mut root = read_manifest_table(path)?;
    let dependencies = ensure_child_table(&mut root, "dependencies")?;
    if dependencies.contains_key(&dependency.alias) {
        return Err(ManifestEditError::AliasExists(dependency.alias.clone()));
    }

    dependencies.insert(dependency.alias.clone(), dependency_value(dependency));
    write_table(path, root)
}

pub fn remove_dependency_from_manifest(path: &Path, alias: &str) -> Result<(), ManifestEditError> {
    let mut root = read_manifest_table(path)?;
    let dependencies = ensure_child_table(&mut root, "dependencies")?;
    if dependencies.remove(alias).is_none() {
        return Err(ManifestEditError::AliasMissing(alias.to_string()));
    }

    write_table(path, root)
}

pub fn default_alias(artifact: &str) -> String {
    let mut alias = artifact
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    while alias.contains("--") {
        alias = alias.replace("--", "-");
    }
    alias = alias.trim_matches('-').to_string();
    if alias.is_empty() {
        "dependency".to_string()
    } else {
        alias
    }
}

pub fn unique_alias(used: &mut BTreeSet<String>, group: &str, artifact: &str) -> String {
    let base = default_alias(artifact);
    if used.insert(base.clone()) {
        return base;
    }

    let group_suffix = group
        .rsplit('.')
        .find(|part| !part.trim().is_empty())
        .map(default_alias)
        .filter(|part| !part.is_empty())
        .unwrap_or_else(|| "maven".to_string());
    let candidate = format!("{group_suffix}-{base}");
    if used.insert(candidate.clone()) {
        return candidate;
    }

    for index in 2.. {
        let candidate = format!("{base}-{index}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }

    unreachable!("unbounded alias suffix loop should always return")
}

fn read_manifest_table(path: &Path) -> Result<Table, ManifestEditError> {
    if !path.exists() {
        return Err(ManifestEditError::Missing(path.to_path_buf()));
    }

    let raw = fs::read_to_string(path)?;
    Ok(toml::from_str::<Table>(&raw)?)
}

fn write_table(path: &Path, table: Table) -> Result<(), ManifestEditError> {
    let serialized = toml::to_string_pretty(&table)?;
    fs::write(path, serialized)?;
    Ok(())
}

fn ensure_child_table<'a>(
    root: &'a mut Table,
    name: &str,
) -> Result<&'a mut Table, ManifestEditError> {
    if !root.contains_key(name) {
        root.insert(name.to_string(), Value::Table(Table::new()));
    }

    root.get_mut(name)
        .and_then(Value::as_table_mut)
        .ok_or_else(|| ManifestEditError::InvalidTable(name.to_string()))
}

fn insert_string_if_present(table: &mut Table, key: &str, value: Option<&str>) {
    if let Some(value) = value
        && !value.trim().is_empty()
    {
        table.insert(key.to_string(), Value::String(value.to_string()));
    }
}

fn repository_value(repository: &Repository) -> Option<Value> {
    let default_policy = RepositoryPolicy::default();
    if repository.releases == default_policy && repository.snapshots == default_policy {
        return Some(Value::String(repository.url.clone()));
    }

    let mut table = Table::new();
    table.insert("url".to_string(), Value::String(repository.url.clone()));
    if !repository.releases.enabled {
        table.insert("releases".to_string(), Value::Boolean(false));
    }
    if !repository.snapshots.enabled {
        table.insert("snapshots".to_string(), Value::Boolean(false));
    }
    if repository.releases.checksum_policy == repository.snapshots.checksum_policy
        && repository.releases.checksum_policy != ChecksumPolicy::Fail
    {
        table.insert(
            "checksum-policy".to_string(),
            Value::String(checksum_policy_name(&repository.releases.checksum_policy).to_string()),
        );
    }
    Some(Value::Table(table))
}

fn dependency_value(dependency: &DeclaredDependency) -> Value {
    if dependency.artifact.artifact_type == ArtifactType::Jar
        && dependency.artifact.classifier.is_none()
        && dependency.scope == Scope::Compile
        && dependency.exclusions.is_empty()
    {
        return Value::String(dependency.artifact.coordinate.to_string());
    }

    let mut table = artifact_table(&dependency.artifact);
    if dependency.scope != Scope::Compile {
        table.insert(
            "scope".to_string(),
            Value::String(dependency.scope.to_string()),
        );
    }
    insert_exclusions(&mut table, &dependency.exclusions);
    Value::Table(table)
}

fn managed_dependency_value(dependency: &DeclaredManagedDependency) -> Value {
    if dependency.artifact.artifact_type == ArtifactType::Jar
        && dependency.artifact.classifier.is_none()
        && dependency.scope == ManagedDependencyScope::None
        && dependency.exclusions.is_empty()
    {
        return Value::String(dependency.artifact.coordinate.to_string());
    }

    let mut table = artifact_table(&dependency.artifact);
    match dependency.scope {
        ManagedDependencyScope::None => {}
        ManagedDependencyScope::Graph(scope) => {
            table.insert("scope".to_string(), Value::String(scope.to_string()));
        }
        ManagedDependencyScope::Import => {
            table.insert("scope".to_string(), Value::String("import".to_string()));
        }
    }
    insert_exclusions(&mut table, &dependency.exclusions);
    Value::Table(table)
}

fn artifact_table(artifact: &ArtifactCoordinate) -> Table {
    let mut table = Table::new();
    table.insert(
        "group".to_string(),
        Value::String(artifact.coordinate.group.clone()),
    );
    table.insert(
        "artifact".to_string(),
        Value::String(artifact.coordinate.artifact.clone()),
    );
    table.insert(
        "version".to_string(),
        Value::String(artifact.coordinate.version.clone()),
    );
    if artifact.artifact_type != ArtifactType::Jar {
        table.insert(
            "type".to_string(),
            Value::String(artifact.artifact_type.to_string()),
        );
    }
    insert_string_if_present(&mut table, "classifier", artifact.classifier.as_deref());
    table
}

fn insert_exclusions(table: &mut Table, exclusions: &[Coordinate]) {
    if exclusions.is_empty() {
        return;
    }

    table.insert(
        "exclusions".to_string(),
        Value::Array(
            exclusions
                .iter()
                .map(|exclusion| {
                    Value::String(format!("{}:{}", exclusion.group, exclusion.artifact))
                })
                .collect(),
        ),
    );
}

fn checksum_policy_name(policy: &ChecksumPolicy) -> &str {
    match policy {
        ChecksumPolicy::Fail => "fail",
        ChecksumPolicy::Warn => "warn",
        ChecksumPolicy::Ignore => "ignore",
        ChecksumPolicy::Unknown(value) => value,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("failed to read manifest: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse manifest TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error(transparent)]
    Coordinate(#[from] crate::maven::CoordinateError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(toml: &str) -> String {
        toml::from_str::<Manifest>(toml)
            .unwrap()
            .resolver_fingerprint()
            .unwrap()
    }

    #[test]
    fn fingerprint_ignores_formatting_and_comments() {
        let compact = fingerprint(
            r#"
            [dependencies]
            demo = "com.example:demo:1.0.0"
            "#,
        );
        let reformatted = fingerprint(
            "# a comment\n[project]\nartifact = \"app\"\n\n[dependencies]\ndemo = \"com.example:demo:1.0.0\" # trailing\n",
        );
        assert_eq!(compact, reformatted);
    }

    #[test]
    fn fingerprint_matches_equivalent_compact_and_structured_dependencies() {
        let compact = fingerprint(
            r#"
            [dependencies]
            demo = "com.example:demo:1.0.0"
            "#,
        );
        let structured = fingerprint(
            r#"
            [dependencies]
            demo = { group = "com.example", artifact = "demo", version = "1.0.0" }
            "#,
        );
        assert_eq!(compact, structured);
    }

    #[test]
    fn fingerprint_changes_on_resolver_relevant_edits() {
        let base = fingerprint(
            r#"
            [dependencies]
            demo = "com.example:demo:1.0.0"
            "#,
        );

        let version_bump = fingerprint(
            r#"
            [dependencies]
            demo = "com.example:demo:1.0.1"
            "#,
        );
        let added = fingerprint(
            r#"
            [dependencies]
            demo = "com.example:demo:1.0.0"
            extra = "com.example:extra:2.0.0"
            "#,
        );
        let removed = fingerprint("");
        let scoped = fingerprint(
            r#"
            [dependencies]
            demo = { group = "com.example", artifact = "demo", version = "1.0.0", scope = "test" }
            "#,
        );
        let repo = fingerprint(
            r#"
            [repositories]
            internal = "https://repo.example.com/releases"

            [dependencies]
            demo = "com.example:demo:1.0.0"
            "#,
        );
        let managed = fingerprint(
            r#"
            [dependency-management]
            bom = { group = "com.example", artifact = "bom", version = "1.0.0", type = "pom", scope = "import" }

            [dependencies]
            demo = "com.example:demo:1.0.0"
            "#,
        );
        let profiles = fingerprint(
            r#"
            [resolver.maven]
            active-profiles = ["dev"]

            [dependencies]
            demo = "com.example:demo:1.0.0"
            "#,
        );

        for changed in [
            version_bump,
            added,
            removed,
            scoped,
            repo,
            managed,
            profiles,
        ] {
            assert_ne!(base, changed);
        }
    }

    #[test]
    fn writes_manifest_with_workspace_and_structured_dependencies() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("angra.toml");

        WritableManifest {
            project: Some(Project {
                group: Some("dev.angra".to_string()),
                artifact: Some("demo".to_string()),
                version: Some("0.1.0".to_string()),
            }),
            workspace_members: vec!["app".to_string(), "lib".to_string()],
            repositories: vec![Repository::with_policies(
                "snapshots",
                "https://repo.example.com/snapshots",
                false,
                true,
            )],
            dependency_management: vec![DeclaredManagedDependency {
                alias: "spring".to_string(),
                artifact: ArtifactCoordinate::pom(Coordinate::new(
                    "org.springframework.boot",
                    "spring-boot-dependencies",
                    "4.0.6",
                )),
                scope: ManagedDependencyScope::Import,
                exclusions: Vec::new(),
            }],
            dependencies: vec![DeclaredDependency {
                alias: "native".to_string(),
                artifact: ArtifactCoordinate::new(
                    Coordinate::new("com.example", "native-lib", "1.0.0"),
                    ArtifactType::Jar,
                    Some("linux-aarch64".to_string()),
                ),
                scope: Scope::Runtime,
                exclusions: vec![Coordinate::parse_without_version("com.foo:bar").unwrap()],
            }],
        }
        .write(&path, false)
        .unwrap();

        let manifest = Manifest::read(&path).unwrap();
        assert_eq!(manifest.workspace.members, vec!["app", "lib"]);
        let repositories = manifest.declared_repositories(&[], &[]);
        assert_eq!(repositories[0].name, "snapshots");
        assert!(!repositories[0].releases.enabled);

        let management = manifest.declared_dependency_management().unwrap();
        assert_eq!(management[0].scope, ManagedDependencyScope::Import);
        assert_eq!(management[0].artifact.artifact_type, ArtifactType::Pom);

        let dependencies = manifest.declared_dependencies().unwrap();
        assert_eq!(dependencies[0].alias, "native");
        assert_eq!(dependencies[0].scope, Scope::Runtime);
        assert_eq!(
            dependencies[0].artifact.classifier.as_deref(),
            Some("linux-aarch64")
        );
        assert_eq!(dependencies[0].exclusions[0].artifact, "bar");
    }

    #[test]
    fn init_manifest_refuses_to_overwrite_without_force() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("angra.toml");
        fs::write(&path, "[project]\nartifact = \"existing\"\n").unwrap();

        let result = InitManifest {
            project: Project {
                group: None,
                artifact: Some("demo".to_string()),
                version: Some("0.1.0".to_string()),
            },
        }
        .write(&path, false);

        assert!(matches!(result, Err(ManifestEditError::AlreadyExists(_))));
    }

    #[test]
    fn adds_and_removes_dependency_with_alias_conflict_checks() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("angra.toml");
        fs::write(&path, "[project]\nartifact = \"demo\"\n").unwrap();
        let dependency = DeclaredDependency {
            alias: "guava".to_string(),
            artifact: ArtifactCoordinate::jar(Coordinate::new(
                "com.google.guava",
                "guava",
                "33.0.0-jre",
            )),
            scope: Scope::Compile,
            exclusions: Vec::new(),
        };

        add_dependency_to_manifest(&path, &dependency).unwrap();
        let manifest = Manifest::read(&path).unwrap();
        assert_eq!(manifest.declared_dependencies().unwrap()[0].alias, "guava");

        let result = add_dependency_to_manifest(&path, &dependency);
        assert!(matches!(result, Err(ManifestEditError::AliasExists(alias)) if alias == "guava"));

        remove_dependency_from_manifest(&path, "guava").unwrap();
        assert!(Manifest::read(&path).unwrap().dependencies.is_empty());

        let result = remove_dependency_from_manifest(&path, "guava");
        assert!(matches!(result, Err(ManifestEditError::AliasMissing(alias)) if alias == "guava"));
    }

    #[test]
    fn parses_compact_and_structured_dependencies() {
        let manifest: Manifest = toml::from_str(
            r#"
            [repositories]
            central = "https://repo1.maven.org/maven2/"
            snapshots = { url = "https://snapshots.example.com/maven/", releases = false, snapshots = true, checksum-policy = "ignore" }

            [dependency-management]
            spring = { group = "org.springframework.boot", artifact = "spring-boot-dependencies", version = "4.0.6", type = "pom", scope = "import" }

            [dependencies]
            guava = "com.google.guava:guava:33.0.0-jre"
            jackson = { group = "com.fasterxml.jackson.core", artifact = "jackson-databind", version = "2.17.2", scope = "runtime", exclusions = ["com.foo:bar"], type = "jar", classifier = "sources" }
            "#,
        )
        .unwrap();

        let management = manifest.declared_dependency_management().unwrap();
        assert_eq!(management.len(), 1);
        assert_eq!(management[0].alias, "spring");
        assert_eq!(management[0].artifact.artifact_type, ArtifactType::Pom);
        assert_eq!(management[0].scope, ManagedDependencyScope::Import);

        let dependencies = manifest.declared_dependencies().unwrap();

        assert_eq!(dependencies.len(), 2);
        assert_eq!(dependencies[0].alias, "guava");
        assert_eq!(dependencies[0].scope, Scope::Compile);
        assert_eq!(dependencies[1].alias, "jackson");
        assert_eq!(dependencies[1].scope, Scope::Runtime);
        assert_eq!(dependencies[1].artifact.artifact_type, ArtifactType::Jar);
        assert_eq!(
            dependencies[1].artifact.classifier.as_deref(),
            Some("sources")
        );
        assert_eq!(dependencies[1].exclusions[0].group, "com.foo");

        let repositories = manifest.declared_repositories(&[], &[]);
        assert_eq!(repositories.len(), 2);
        assert_eq!(repositories[0].name, "central");
        assert_eq!(repositories[0].url, "https://repo1.maven.org/maven2");
        assert_eq!(repositories[1].name, "snapshots");
        assert!(!repositories[1].releases.enabled);
        assert!(repositories[1].snapshots.enabled);
        assert_eq!(
            repositories[1].snapshots.checksum_policy,
            ChecksumPolicy::Ignore
        );
    }

    #[test]
    fn defaults_to_maven_central_repository() {
        let manifest: Manifest = toml::from_str(
            r#"
            [dependencies]
            guava = "com.google.guava:guava:33.0.0-jre"
            "#,
        )
        .unwrap();

        assert_eq!(
            manifest.declared_repositories(&[], &[]),
            vec![Repository::maven_central()]
        );
    }

    #[test]
    fn merges_global_repositories_with_project_repositories() {
        let manifest: Manifest = toml::from_str(
            r#"
            [repositories]
            central = "https://repo1.maven.org/maven2/"
            "#,
        )
        .unwrap();

        let global = vec![
            Repository::new("central", "https://repo1.maven.org/maven2/"),
            Repository::new("corporate", "https://nexus.example.com/maven/"),
        ];

        let repos = manifest.declared_repositories(&global, &[]);
        let names: Vec<&str> = repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["central", "corporate"]);
    }

    #[test]
    fn preserves_project_repository_order() {
        let manifest: Manifest = toml::from_str(
            r#"
            [repositories]
            internal = "https://nexus.example.com/maven/"
            central = "https://repo1.maven.org/maven2/"
            "#,
        )
        .unwrap();

        let repos = manifest.declared_repositories(&[], &[]);
        let names: Vec<&str> = repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["internal", "central"]);
    }

    #[test]
    fn project_repository_overrides_global_by_name() {
        let manifest: Manifest = toml::from_str(
            r#"
            [repositories]
            corporate = "https://nexus-staging.example.com/maven/"
            "#,
        )
        .unwrap();

        let global = vec![Repository::new(
            "corporate",
            "https://nexus.example.com/maven/",
        )];

        let repos = manifest.declared_repositories(&global, &[]);
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].url, "https://nexus-staging.example.com/maven");
    }

    #[test]
    fn falls_back_to_maven_central_when_no_repos_anywhere() {
        let manifest: Manifest = toml::from_str(
            r#"
            [dependencies]
            guava = "com.google.guava:guava:33.0.0-jre"
            "#,
        )
        .unwrap();

        assert_eq!(
            manifest.declared_repositories(&[], &[]),
            vec![Repository::maven_central()]
        );
    }

    #[test]
    fn appends_settings_repositories_after_project_and_global() {
        let manifest: Manifest = toml::from_str(
            r#"
            [repositories]
            internal = "https://nexus.example.com/maven/"
            "#,
        )
        .unwrap();

        let global = vec![Repository::new(
            "central",
            "https://repo1.maven.org/maven2/",
        )];
        let settings = vec![Repository::new(
            "legacy",
            "https://legacy.example.com/maven/",
        )];

        let repos = manifest.declared_repositories(&global, &settings);
        let names: Vec<&str> = repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["central", "internal", "legacy"]);
    }

    #[test]
    fn settings_repository_does_not_override_project_or_global_by_name() {
        let manifest: Manifest = toml::from_str(
            r#"
            [repositories]
            corporate = "https://nexus-project.example.com/maven/"
            "#,
        )
        .unwrap();

        let global = vec![Repository::new(
            "central",
            "https://repo1.maven.org/maven2/",
        )];
        let settings = vec![
            Repository::new("corporate", "https://nexus-settings.example.com/maven/"),
            Repository::new("central", "https://settings-central.example.com/maven/"),
        ];

        let repos = manifest.declared_repositories(&global, &settings);
        let central = repos.iter().find(|r| r.name == "central").unwrap();
        let corporate = repos.iter().find(|r| r.name == "corporate").unwrap();
        assert_eq!(central.url, "https://repo1.maven.org/maven2");
        assert_eq!(corporate.url, "https://nexus-project.example.com/maven");
        assert_eq!(repos.len(), 2);
    }

    #[test]
    fn settings_only_repositories_are_used_when_project_and_global_empty() {
        let manifest: Manifest = toml::from_str(
            r#"
            [dependencies]
            guava = "com.google.guava:guava:33.0.0-jre"
            "#,
        )
        .unwrap();

        let settings = vec![Repository::new(
            "legacy",
            "https://legacy.example.com/maven/",
        )];

        let repos = manifest.declared_repositories(&[], &settings);
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "legacy");
    }
}
