use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::maven::{ArtifactCoordinate, ArtifactType, Coordinate, Scope};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lockfile {
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_fingerprint: Option<String>,
    pub artifacts: Vec<LockedArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedArtifact {
    pub group: String,
    pub artifact: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_version: Option<String>,
    #[serde(rename = "type")]
    pub artifact_type: ArtifactType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classifier: Option<String>,
    pub scope: Scope,
    pub source: String,
    pub pom_path: PathBuf,
    pub artifact_path: PathBuf,
    pub artifact_sha256: Option<String>,
}

impl LockedArtifact {
    pub fn new(
        artifact: &ArtifactCoordinate,
        requested_version: Option<&str>,
        scope: Scope,
        source: &str,
        pom_path: PathBuf,
        artifact_path: PathBuf,
        artifact_sha256: Option<String>,
    ) -> Self {
        Self {
            group: artifact.coordinate.group.clone(),
            artifact: artifact.coordinate.artifact.clone(),
            version: artifact.coordinate.version.clone(),
            requested_version: requested_version.map(str::to_string),
            artifact_type: artifact.artifact_type,
            classifier: artifact.classifier.clone(),
            scope,
            source: source.to_string(),
            pom_path,
            artifact_path,
            artifact_sha256,
        }
    }

    pub fn artifact_coordinate(&self) -> ArtifactCoordinate {
        ArtifactCoordinate::new(
            Coordinate::new(&self.group, &self.artifact, &self.version),
            self.artifact_type,
            self.classifier.clone(),
        )
    }
}

impl Lockfile {
    pub fn new(manifest_fingerprint: Option<String>, mut artifacts: Vec<LockedArtifact>) -> Self {
        artifacts.sort_by(|left, right| {
            (
                &left.group,
                &left.artifact,
                left.artifact_type,
                &left.classifier,
                &left.version,
            )
                .cmp(&(
                    &right.group,
                    &right.artifact,
                    right.artifact_type,
                    &right.classifier,
                    &right.version,
                ))
        });

        Self {
            version: 1,
            manifest_fingerprint,
            artifacts,
        }
    }

    pub fn read(path: &Path) -> Result<Self, LockfileError> {
        let raw = fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    }

    pub fn write_if_changed(&self, path: &Path) -> Result<bool, LockfileError> {
        let serialized = toml::to_string_pretty(self)?;
        if fs::read_to_string(path).is_ok_and(|existing| existing == serialized) {
            return Ok(false);
        }

        fs::write(path, serialized)?;
        Ok(true)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LockfileError {
    #[error("failed to serialize lockfile: {0}")]
    Toml(#[from] toml::ser::Error),
    #[error("failed to parse lockfile: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("failed to write lockfile: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_stable_lockfile() {
        let lockfile = Lockfile::new(
            Some("fp".to_string()),
            vec![LockedArtifact::new(
                &ArtifactCoordinate::jar(crate::maven::Coordinate::new("b", "a", "1")),
                None,
                Scope::Compile,
                "local",
                PathBuf::from("/m2/b/a/1/a-1.pom"),
                PathBuf::from("/m2/b/a/1/a-1.jar"),
                Some("abc".to_string()),
            )],
        );

        let serialized = toml::to_string_pretty(&lockfile).unwrap();

        assert!(serialized.contains("version = 1"));
        assert!(serialized.contains("manifest_fingerprint = \"fp\""));
        assert!(serialized.contains("type = \"jar\""));
        assert!(serialized.contains("artifact_sha256 = \"abc\""));

        let parsed: Lockfile = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed, lockfile);
    }

    #[test]
    fn reads_lockfile_without_fingerprint() {
        let parsed: Lockfile = toml::from_str("version = 1\nartifacts = []\n").unwrap();
        assert_eq!(parsed.manifest_fingerprint, None);
    }
}
