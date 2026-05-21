use std::{collections::BTreeMap, fs, path::Path};

use serde::Deserialize;

use crate::maven::{Coordinate, Scope};

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub project: Option<Project>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, DependencySpec>,
}

#[derive(Debug, Deserialize)]
pub struct Project {
    pub group: Option<String>,
    pub artifact: Option<String>,
    pub version: Option<String>,
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
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub exclusions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredDependency {
    pub alias: String,
    pub coordinate: Coordinate,
    pub scope: Scope,
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
                let (coordinate, scope, exclusions) = match spec {
                    DependencySpec::Compact(raw) => (raw.parse()?, Scope::Compile, Vec::new()),
                    DependencySpec::Structured(dep) => {
                        let exclusions = dep
                            .exclusions
                            .iter()
                            .map(|exclusion| Coordinate::parse_without_version(exclusion))
                            .collect::<Result<Vec<_>, _>>()?;

                        (
                            Coordinate::new(&dep.group, &dep.artifact, &dep.version),
                            dep.scope,
                            exclusions,
                        )
                    }
                };

                Ok(DeclaredDependency {
                    alias: alias.clone(),
                    coordinate,
                    scope,
                    exclusions,
                })
            })
            .collect()
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
            [dependencies]
            guava = "com.google.guava:guava:33.0.0-jre"
            jackson = { group = "com.fasterxml.jackson.core", artifact = "jackson-databind", version = "2.17.2", scope = "runtime", exclusions = ["com.foo:bar"] }
            "#,
        )
        .unwrap();

        let dependencies = manifest.declared_dependencies().unwrap();

        assert_eq!(dependencies.len(), 2);
        assert_eq!(dependencies[0].alias, "guava");
        assert_eq!(dependencies[0].scope, Scope::Compile);
        assert_eq!(dependencies[1].alias, "jackson");
        assert_eq!(dependencies[1].scope, Scope::Runtime);
        assert_eq!(dependencies[1].exclusions[0].group, "com.foo");
    }
}
