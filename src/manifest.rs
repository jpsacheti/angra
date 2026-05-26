use std::{collections::BTreeMap, fs, path::Path};

use indexmap::IndexMap;
use serde::Deserialize;

use crate::maven::{
    ArtifactCoordinate, ArtifactType, ChecksumPolicy, Coordinate, Repository, RepositoryPolicy,
    Scope,
};

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub project: Option<Project>,
    #[serde(default)]
    pub resolver: ResolverConfig,
    #[serde(default)]
    pub repositories: IndexMap<String, RepositorySpec>,
    #[serde(default, rename = "dependency-management")]
    pub dependency_management: BTreeMap<String, ManagedDependencySpec>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, DependencySpec>,
}

#[derive(Debug, Deserialize)]
pub struct Project {
    pub group: Option<String>,
    pub artifact: Option<String>,
    pub version: Option<String>,
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
