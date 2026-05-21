use std::{
    fmt::{Display, Formatter},
    path::{Path, PathBuf},
    str::FromStr,
};

use serde::{Deserialize, Serialize};

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
        self.local_dir(local_repo)
            .join(format!("{}-{}.jar", self.artifact, self.version))
    }

    pub fn central_pom_url(&self) -> String {
        format!(
            "https://repo1.maven.org/maven2/{}/{}/{}/{}-{}.pom",
            self.group_path(),
            self.artifact,
            self.version,
            self.artifact,
            self.version
        )
    }

    pub fn central_jar_url(&self) -> String {
        format!(
            "https://repo1.maven.org/maven2/{}/{}/{}/{}-{}.jar",
            self.group_path(),
            self.artifact,
            self.version,
            self.artifact,
            self.version
        )
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactIdentity {
    pub group: String,
    pub artifact: String,
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
}
