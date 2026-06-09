use std::{
    fs,
    path::{Path, PathBuf},
};

use indexmap::IndexMap;
use serde::Deserialize;

use crate::{manifest::RepositorySpec, maven::Repository};

#[derive(Debug, Default, Deserialize)]
pub struct GlobalConfig {
    #[serde(default)]
    pub repositories: IndexMap<String, RepositorySpec>,
}

impl GlobalConfig {
    /// Load the global config from `~/.config/angra/config.toml`.
    /// Returns a default (empty) config if the file does not exist.
    pub fn load() -> Result<Self, ConfigError> {
        let path = config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::read(&path)
    }

    /// Read config from an explicit path. Useful for testing.
    pub fn read(path: &Path) -> Result<Self, ConfigError> {
        let raw = fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    }

    /// Return global repositories in declaration order.
    pub fn repositories(&self) -> Vec<Repository> {
        self.repositories
            .iter()
            .map(|(name, spec)| spec.to_repository(name))
            .collect()
    }
}

/// Returns the global config path.
///
/// Angra intentionally uses the XDG-style `~/.config/angra/config.toml` on
/// Unix-like systems, including macOS, so the documented path is stable across
/// developer machines. Windows uses the platform config directory.
pub fn config_path() -> PathBuf {
    #[cfg(not(windows))]
    {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config")
            .join("angra")
            .join("config.toml")
    }

    #[cfg(windows)]
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("angra")
        .join("config.toml")
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read global config: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse global config TOML: {0}")]
    Toml(#[from] toml::de::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maven::MAVEN_CENTRAL_URL;

    #[test]
    fn parses_repositories_from_global_config() {
        let config: GlobalConfig = toml::from_str(
            r#"
            [repositories]
            central = "https://repo1.maven.org/maven2/"
            corporate = { url = "https://nexus.example.com/maven/", snapshots = false, checksum-policy = "warn" }
            "#,
        )
        .unwrap();

        let repos = config.repositories();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].name, "central");
        assert_eq!(repos[0].url, MAVEN_CENTRAL_URL);
        assert_eq!(repos[1].name, "corporate");
        assert_eq!(repos[1].url, "https://nexus.example.com/maven");
        assert!(!repos[1].snapshots.enabled);
        assert_eq!(
            repos[1].releases.checksum_policy,
            crate::maven::ChecksumPolicy::Warn
        );
    }

    #[test]
    fn preserves_repository_order() {
        let config: GlobalConfig = toml::from_str(
            r#"
            [repositories]
            internal = "https://nexus.example.com/maven/"
            central = "https://repo1.maven.org/maven2/"
            "#,
        )
        .unwrap();

        let repos = config.repositories();
        assert_eq!(repos[0].name, "internal");
        assert_eq!(repos[1].name, "central");
    }

    #[cfg(not(windows))]
    #[test]
    fn uses_xdg_config_path_on_unix() {
        assert!(config_path().ends_with(Path::new(".config").join("angra").join("config.toml")));
    }

    #[test]
    fn defaults_to_empty_repositories() {
        let config = GlobalConfig::default();
        assert!(config.repositories.is_empty());
    }

    #[test]
    fn reads_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
[repositories]
central = "https://repo1.maven.org/maven2/"
"#,
        )
        .unwrap();

        let config = GlobalConfig::read(&path).unwrap();
        assert_eq!(config.repositories.len(), 1);
    }

    #[test]
    fn read_returns_error_when_file_missing() {
        let path = PathBuf::from("/tmp/angra-nonexistent-config-test.toml");
        let result = GlobalConfig::read(&path);
        assert!(result.is_err());
    }
}
