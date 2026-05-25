use std::{
    fmt::{Display, Formatter},
    path::{Path, PathBuf},
    str::FromStr,
};

use serde::{Deserialize, Serialize};

pub const MAVEN_CENTRAL_NAME: &str = "maven-central";
pub const MAVEN_CENTRAL_URL: &str = "https://repo1.maven.org/maven2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryPolicy {
    pub enabled: bool,
}

impl Default for RepositoryPolicy {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    pub name: String,
    pub url: String,
    pub releases: RepositoryPolicy,
    pub snapshots: RepositoryPolicy,
}

impl Repository {
    pub fn new(name: &str, url: &str) -> Self {
        Self {
            name: name.to_string(),
            url: url.trim_end_matches('/').to_string(),
            releases: RepositoryPolicy::default(),
            snapshots: RepositoryPolicy::default(),
        }
    }

    pub fn with_policies(
        name: &str,
        url: &str,
        releases_enabled: bool,
        snapshots_enabled: bool,
    ) -> Self {
        Self {
            name: name.to_string(),
            url: url.trim_end_matches('/').to_string(),
            releases: RepositoryPolicy {
                enabled: releases_enabled,
            },
            snapshots: RepositoryPolicy {
                enabled: snapshots_enabled,
            },
        }
    }

    pub fn maven_central() -> Self {
        Self::with_policies(MAVEN_CENTRAL_NAME, MAVEN_CENTRAL_URL, true, false)
    }

    /// Returns true if this repository accepts artifacts with the given coordinate,
    /// based on the release/snapshot policies.
    pub fn accepts(&self, coordinate: &Coordinate) -> bool {
        if coordinate.is_snapshot() {
            self.snapshots.enabled
        } else {
            self.releases.enabled
        }
    }

    pub fn pom_url(&self, coordinate: &Coordinate) -> String {
        format!(
            "{}/{}/{}/{}/{}-{}.pom",
            self.url,
            coordinate.group_path(),
            coordinate.artifact,
            coordinate.version,
            coordinate.artifact,
            coordinate.version
        )
    }

    pub fn artifact_url(&self, artifact: &ArtifactCoordinate) -> String {
        if artifact.artifact_type == ArtifactType::Pom && artifact.classifier.is_none() {
            return self.pom_url(&artifact.coordinate);
        }

        format!(
            "{}/{}/{}/{}/{}-{}.{}",
            self.url,
            artifact.coordinate.group_path(),
            artifact.coordinate.artifact,
            artifact.coordinate.version,
            artifact.coordinate.artifact,
            artifact.version_suffix(),
            artifact.artifact_type.extension()
        )
    }
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactType {
    #[default]
    Jar,
    Pom,
    War,
}

impl ArtifactType {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Jar => "jar",
            Self::Pom => "pom",
            Self::War => "war",
        }
    }
}

impl Display for ArtifactType {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.extension())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Coordinate {
    pub group: String,
    pub artifact: String,
    pub version: String,
}

impl Coordinate {
    pub fn new(group: &str, artifact: &str, version: &str) -> Self {
        Self {
            group: group.to_string(),
            artifact: artifact.to_string(),
            version: version.to_string(),
        }
    }

    /// Returns true if this coordinate's version is a Maven SNAPSHOT version.
    pub fn is_snapshot(&self) -> bool {
        self.version.ends_with("-SNAPSHOT")
    }

    pub fn parse_without_version(raw: &str) -> Result<Self, CoordinateError> {
        let parts = raw.split(':').collect::<Vec<_>>();
        if parts.len() != 2 || parts.iter().any(|part| part.trim().is_empty()) {
            return Err(CoordinateError::InvalidExclusion(raw.to_string()));
        }

        Ok(Self::new(parts[0], parts[1], ""))
    }

    pub fn identity(&self) -> ArtifactIdentity {
        ArtifactIdentity {
            group: self.group.clone(),
            artifact: self.artifact.clone(),
            artifact_type: ArtifactType::Jar,
            classifier: None,
        }
    }

    pub fn matches_identity(&self, other: &Coordinate) -> bool {
        self.group == other.group && self.artifact == other.artifact
    }

    pub fn group_path(&self) -> String {
        self.group.replace('.', "/")
    }

    pub fn local_dir(&self, local_repo: &Path) -> PathBuf {
        local_repo
            .join(self.group_path())
            .join(&self.artifact)
            .join(&self.version)
    }

    pub fn pom_path(&self, local_repo: &Path) -> PathBuf {
        self.local_dir(local_repo)
            .join(format!("{}-{}.pom", self.artifact, self.version))
    }

    pub fn jar_path(&self, local_repo: &Path) -> PathBuf {
        ArtifactCoordinate::jar(self.clone()).artifact_path(local_repo)
    }

    pub fn central_pom_url(&self) -> String {
        Repository::maven_central().pom_url(self)
    }

    pub fn central_jar_url(&self) -> String {
        ArtifactCoordinate::jar(self.clone()).central_artifact_url()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ArtifactCoordinate {
    pub coordinate: Coordinate,
    #[serde(rename = "type")]
    pub artifact_type: ArtifactType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classifier: Option<String>,
}

impl ArtifactCoordinate {
    pub fn new(
        coordinate: Coordinate,
        artifact_type: ArtifactType,
        classifier: Option<String>,
    ) -> Self {
        Self {
            coordinate,
            artifact_type,
            classifier: classifier.filter(|value| !value.trim().is_empty()),
        }
    }

    pub fn jar(coordinate: Coordinate) -> Self {
        Self::new(coordinate, ArtifactType::Jar, None)
    }

    pub fn pom(coordinate: Coordinate) -> Self {
        Self::new(coordinate, ArtifactType::Pom, None)
    }

    pub fn identity(&self) -> ArtifactIdentity {
        ArtifactIdentity {
            group: self.coordinate.group.clone(),
            artifact: self.coordinate.artifact.clone(),
            artifact_type: self.artifact_type,
            classifier: self.classifier.clone(),
        }
    }

    pub fn local_dir(&self, local_repo: &Path) -> PathBuf {
        self.coordinate.local_dir(local_repo)
    }

    pub fn pom_path(&self, local_repo: &Path) -> PathBuf {
        self.coordinate.pom_path(local_repo)
    }

    pub fn artifact_path(&self, local_repo: &Path) -> PathBuf {
        if self.artifact_type == ArtifactType::Pom && self.classifier.is_none() {
            return self.pom_path(local_repo);
        }

        self.local_dir(local_repo).join(format!(
            "{}-{}.{}",
            self.coordinate.artifact,
            self.version_suffix(),
            self.artifact_type.extension()
        ))
    }

    pub fn central_artifact_url(&self) -> String {
        Repository::maven_central().artifact_url(self)
    }

    fn version_suffix(&self) -> String {
        match &self.classifier {
            Some(classifier) => format!("{}-{classifier}", self.coordinate.version),
            None => self.coordinate.version.clone(),
        }
    }
}

impl FromStr for Coordinate {
    type Err = CoordinateError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let parts = raw.split(':').collect::<Vec<_>>();
        if parts.len() != 3 || parts.iter().any(|part| part.trim().is_empty()) {
            return Err(CoordinateError::Invalid(raw.to_string()));
        }

        Ok(Self::new(parts[0], parts[1], parts[2]))
    }
}

impl Display for Coordinate {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}:{}:{}",
            self.group, self.artifact, self.version
        )
    }
}

impl Display for ArtifactCoordinate {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.classifier {
            Some(classifier) => write!(
                formatter,
                "{}:{}:{}:{}:{}",
                self.coordinate.group,
                self.coordinate.artifact,
                self.artifact_type,
                classifier,
                self.coordinate.version
            ),
            None if self.artifact_type != ArtifactType::Jar => write!(
                formatter,
                "{}:{}:{}:{}",
                self.coordinate.group,
                self.coordinate.artifact,
                self.artifact_type,
                self.coordinate.version
            ),
            None => write!(formatter, "{}", self.coordinate),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactIdentity {
    pub group: String,
    pub artifact: String,
    pub artifact_type: ArtifactType,
    pub classifier: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    #[default]
    Compile,
    Runtime,
    Test,
    Provided,
}

impl Scope {
    pub fn is_runtime_graph(self) -> bool {
        matches!(self, Scope::Compile | Scope::Runtime)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CoordinateError {
    #[error("invalid Maven coordinate `{0}`, expected group:artifact:version")]
    Invalid(String),
    #[error("invalid Maven exclusion `{0}`, expected group:artifact")]
    InvalidExclusion(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_coordinates() {
        assert!("guava".parse::<Coordinate>().is_err());
        assert!("com.google.guava:guava".parse::<Coordinate>().is_err());
        assert!("com.google.guava:guava:".parse::<Coordinate>().is_err());
    }

    #[test]
    fn builds_repository_urls() {
        let repository = Repository::new("internal", "https://repo.example.com/maven/");
        let coordinate = Coordinate::new("com.example", "demo", "1.0.0");
        let classified = ArtifactCoordinate::new(
            coordinate.clone(),
            ArtifactType::Jar,
            Some("linux-aarch64".to_string()),
        );

        assert_eq!(
            repository.pom_url(&coordinate),
            "https://repo.example.com/maven/com/example/demo/1.0.0/demo-1.0.0.pom"
        );
        assert_eq!(
            repository.artifact_url(&classified),
            "https://repo.example.com/maven/com/example/demo/1.0.0/demo-1.0.0-linux-aarch64.jar"
        );
    }

    #[test]
    fn converts_coordinate_to_paths_and_urls() {
        let coordinate: Coordinate = "com.google.guava:guava:33.0.0-jre".parse().unwrap();
        let local = Path::new("/tmp/m2");

        assert_eq!(
            coordinate.pom_path(local),
            Path::new("/tmp/m2/com/google/guava/guava/33.0.0-jre/guava-33.0.0-jre.pom")
        );
        assert_eq!(
            coordinate.central_jar_url(),
            "https://repo1.maven.org/maven2/com/google/guava/guava/33.0.0-jre/guava-33.0.0-jre.jar"
        );
    }

    #[test]
    fn converts_artifact_coordinates_to_paths_and_urls() {
        let local = Path::new("/tmp/m2");
        let coordinate = Coordinate::new("com.example", "demo", "1.0.0");

        assert_eq!(
            ArtifactCoordinate::jar(coordinate.clone()).artifact_path(local),
            Path::new("/tmp/m2/com/example/demo/1.0.0/demo-1.0.0.jar")
        );
        assert_eq!(
            ArtifactCoordinate::new(coordinate.clone(), ArtifactType::War, None)
                .artifact_path(local),
            Path::new("/tmp/m2/com/example/demo/1.0.0/demo-1.0.0.war")
        );
        let classified = ArtifactCoordinate::new(
            coordinate.clone(),
            ArtifactType::Jar,
            Some("linux-aarch64".to_string()),
        );
        assert_eq!(
            classified.artifact_path(local),
            Path::new("/tmp/m2/com/example/demo/1.0.0/demo-1.0.0-linux-aarch64.jar")
        );
        assert_eq!(
            classified.central_artifact_url(),
            "https://repo1.maven.org/maven2/com/example/demo/1.0.0/demo-1.0.0-linux-aarch64.jar"
        );
        assert_eq!(
            ArtifactCoordinate::pom(coordinate.clone()).artifact_path(local),
            Path::new("/tmp/m2/com/example/demo/1.0.0/demo-1.0.0.pom")
        );
        assert_eq!(
            ArtifactCoordinate::pom(coordinate.clone()).central_artifact_url(),
            "https://repo1.maven.org/maven2/com/example/demo/1.0.0/demo-1.0.0.pom"
        );
        assert_eq!(
            ArtifactCoordinate::new(coordinate, ArtifactType::War, None).central_artifact_url(),
            "https://repo1.maven.org/maven2/com/example/demo/1.0.0/demo-1.0.0.war"
        );
    }

    #[test]
    fn detects_snapshot_versions() {
        assert!(Coordinate::new("com.example", "lib", "1.0.0-SNAPSHOT").is_snapshot());
        assert!(Coordinate::new("com.example", "lib", "2.0-SNAPSHOT").is_snapshot());
        assert!(!Coordinate::new("com.example", "lib", "1.0.0").is_snapshot());
        assert!(!Coordinate::new("com.example", "lib", "1.0.0-jre").is_snapshot());
        // Case-sensitive: lowercase doesn't count
        assert!(!Coordinate::new("com.example", "lib", "1.0.0-snapshot").is_snapshot());
    }

    #[test]
    fn maven_central_rejects_snapshots() {
        let central = Repository::maven_central();
        let release = Coordinate::new("com.example", "lib", "1.0.0");
        let snapshot = Coordinate::new("com.example", "lib", "1.0.0-SNAPSHOT");

        assert!(central.accepts(&release));
        assert!(!central.accepts(&snapshot));
    }

    #[test]
    fn default_repository_accepts_both() {
        let repo = Repository::new("custom", "https://repo.example.com");
        let release = Coordinate::new("com.example", "lib", "1.0.0");
        let snapshot = Coordinate::new("com.example", "lib", "1.0.0-SNAPSHOT");

        assert!(repo.accepts(&release));
        assert!(repo.accepts(&snapshot));
    }

    #[test]
    fn snapshot_only_repo_rejects_releases() {
        let repo = Repository::with_policies("snapshots", "https://repo.example.com", false, true);
        let release = Coordinate::new("com.example", "lib", "1.0.0");
        let snapshot = Coordinate::new("com.example", "lib", "1.0.0-SNAPSHOT");

        assert!(!repo.accepts(&release));
        assert!(repo.accepts(&snapshot));
    }
}
