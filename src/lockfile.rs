use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::maven::{Coordinate, Scope};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lockfile {
    pub version: u32,
    pub artifacts: Vec<LockedArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedArtifact {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub scope: Scope,
    pub source: String,
    pub pom_path: PathBuf,
    pub jar_path: PathBuf,
    pub jar_sha256: Option<String>,
}

impl LockedArtifact {
    pub fn new(
        coordinate: &Coordinate,
        scope: Scope,
        source: &str,
        pom_path: PathBuf,
        jar_path: PathBuf,
        jar_sha256: Option<String>,
    ) -> Self {
        Self {
            group: coordinate.group.clone(),
            artifact: coordinate.artifact.clone(),
            version: coordinate.version.clone(),
            scope,
            source: source.to_string(),
            pom_path,
            jar_path,
            jar_sha256,
        }
    }
}

impl Lockfile {
    pub fn new(mut artifacts: Vec<LockedArtifact>) -> Self {
        artifacts.sort_by(|left, right| {
            (&left.group, &left.artifact, &left.version).cmp(&(
                &right.group,
                &right.artifact,
                &right.version,
            ))
        });

        Self {
            version: 1,
            artifacts,
        }
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
    #[error("failed to write lockfile: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_stable_lockfile() {
        let lockfile = Lockfile::new(vec![LockedArtifact::new(
            &Coordinate::new("b", "a", "1"),
            Scope::Compile,
            "local",
            PathBuf::from("/m2/b/a/1/a-1.pom"),
            PathBuf::from("/m2/b/a/1/a-1.jar"),
            Some("abc".to_string()),
        )]);

        let serialized = toml::to_string_pretty(&lockfile).unwrap();

        assert!(serialized.contains("version = 1"));
        assert!(serialized.contains("jar_sha256 = \"abc\""));
    }
}
