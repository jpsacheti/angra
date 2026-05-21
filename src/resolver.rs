use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use reqwest::blocking::Client;
use sha2::{Digest, Sha256};

use crate::{
    lockfile::{LockedArtifact, Lockfile, LockfileError},
    manifest::{DeclaredDependency, Manifest, ManifestError},
    maven::{ArtifactIdentity, Coordinate, Scope},
};

const MAVEN_CENTRAL: &str = "maven-central";

#[derive(Debug, Clone)]
pub struct ResolveOptions {
    pub project_dir: PathBuf,
    pub offline: bool,
    pub refresh: bool,
    pub local_repo: Option<PathBuf>,
}

#[derive(Debug)]
struct QueuedDependency {
    coordinate: Coordinate,
    scope: Scope,
    depth: usize,
    exclusions: Vec<Coordinate>,
}

#[derive(Debug, Clone)]
struct ResolvedArtifact {
    coordinate: Coordinate,
    scope: Scope,
    depth: usize,
    pom_path: PathBuf,
    jar_path: PathBuf,
    source: String,
}

pub fn resolve_project(options: ResolveOptions) -> Result<Lockfile, ResolveError> {
    let manifest_path = options.project_dir.join("angra.toml");
    let manifest = Manifest::read(&manifest_path)?;
    let dependencies = manifest.declared_dependencies()?;
    let local_repo = options.local_repo.unwrap_or(default_local_repo()?);
    let resolver = Resolver::new(local_repo, options.offline, options.refresh)?;
    let artifacts = resolver.resolve(dependencies)?;

    let lockfile = Lockfile::new(
        artifacts
            .into_iter()
            .map(|artifact| {
                let sha = sha256_file(&artifact.jar_path)?;
                Ok(LockedArtifact::new(
                    &artifact.coordinate,
                    artifact.scope,
                    &artifact.source,
                    artifact.pom_path,
                    artifact.jar_path,
                    Some(sha),
                ))
            })
            .collect::<Result<Vec<_>, ResolveError>>()?,
    );

    lockfile.write_if_changed(&options.project_dir.join("angra.lock"))?;
    Ok(lockfile)
}

struct Resolver {
    local_repo: PathBuf,
    offline: bool,
    refresh: bool,
    client: Client,
}

impl Resolver {
    fn new(local_repo: PathBuf, offline: bool, refresh: bool) -> Result<Self, ResolveError> {
        Ok(Self {
            local_repo,
            offline,
            refresh,
            client: Client::builder().http1_only().build()?,
        })
    }

    fn resolve(
        &self,
        dependencies: Vec<DeclaredDependency>,
    ) -> Result<Vec<ResolvedArtifact>, ResolveError> {
        let mut queue = VecDeque::new();
        let direct_identities = dependencies
            .iter()
            .map(|dependency| dependency.coordinate.identity())
            .collect::<HashSet<_>>();

        for dependency in dependencies {
            queue.push_back(QueuedDependency {
                coordinate: dependency.coordinate,
                scope: dependency.scope,
                depth: 0,
                exclusions: dependency.exclusions,
            });
        }

        let mut selected: BTreeMap<ArtifactIdentity, ResolvedArtifact> = BTreeMap::new();
        let mut selected_exclusions: BTreeMap<ArtifactIdentity, Vec<Coordinate>> = BTreeMap::new();
        let mut visited_versions = HashSet::new();

        while let Some(item) = queue.pop_front() {
            if !item.scope.is_runtime_graph() {
                continue;
            }

            if item.exclusions.iter().any(|exclusion| {
                exclusion.group == item.coordinate.group
                    && exclusion.artifact == item.coordinate.artifact
            }) {
                continue;
            }

            let identity = item.coordinate.identity();
            if let Some(existing) = selected.get(&identity)
                && existing.depth <= item.depth
            {
                continue;
            }

            let fetched = self.ensure_artifact(&item.coordinate)?;
            selected.insert(
                identity.clone(),
                ResolvedArtifact {
                    coordinate: item.coordinate.clone(),
                    scope: item.scope,
                    depth: item.depth,
                    pom_path: item.coordinate.pom_path(&self.local_repo),
                    jar_path: item.coordinate.jar_path(&self.local_repo),
                    source: fetched,
                },
            );
            selected_exclusions.insert(identity, item.exclusions.clone());

            let version_key = item.coordinate.to_string();
            if !visited_versions.insert(version_key) {
                continue;
            }

            let pom = Pom::read(&item.coordinate.pom_path(&self.local_repo))?;
            for dependency in pom.dependencies {
                let Some(coordinate) = dependency.coordinate() else {
                    continue;
                };
                if dependency.optional && !direct_identities.contains(&coordinate.identity()) {
                    continue;
                }

                let dependency_scope = dependency.scope();
                if !dependency_scope.is_runtime_graph() {
                    continue;
                }

                if coordinate.version.contains("${") {
                    return Err(ResolveError::UnsupportedPomProperty {
                        pom: item.coordinate.to_string(),
                        value: coordinate.version,
                    });
                }

                let mut exclusions = selected_exclusions
                    .get(&item.coordinate.identity())
                    .cloned()
                    .unwrap_or_default();
                exclusions.extend(dependency.exclusions());

                queue.push_back(QueuedDependency {
                    coordinate,
                    scope: combine_scope(item.scope, dependency_scope),
                    depth: item.depth + 1,
                    exclusions,
                });
            }
        }

        Ok(selected.into_values().collect())
    }

    fn ensure_artifact(&self, coordinate: &Coordinate) -> Result<String, ResolveError> {
        let pom_path = coordinate.pom_path(&self.local_repo);
        let jar_path = coordinate.jar_path(&self.local_repo);

        let source = if !self.refresh && pom_path.exists() && jar_path.exists() {
            "local".to_string()
        } else {
            if self.offline {
                return Err(ResolveError::OfflineMissing(coordinate.to_string()));
            }

            fs::create_dir_all(coordinate.local_dir(&self.local_repo))?;
            self.download(&coordinate.central_pom_url(), &pom_path)?;
            self.download(&coordinate.central_jar_url(), &jar_path)?;
            MAVEN_CENTRAL.to_string()
        };

        Ok(source)
    }

    fn download(&self, url: &str, destination: &Path) -> Result<(), ResolveError> {
        let response = self
            .client
            .get(url)
            .send()
            .map_err(|source| ResolveError::Download {
                url: url.to_string(),
                source,
            })?
            .error_for_status()
            .map_err(|source| ResolveError::Download {
                url: url.to_string(),
                source,
            })?;
        let bytes = response.bytes().map_err(|source| ResolveError::Download {
            url: url.to_string(),
            source,
        })?;
        fs::write(destination, bytes)?;
        Ok(())
    }
}

fn combine_scope(parent: Scope, child: Scope) -> Scope {
    match (parent, child) {
        (Scope::Runtime, _) | (_, Scope::Runtime) => Scope::Runtime,
        _ => Scope::Compile,
    }
}

fn default_local_repo() -> Result<PathBuf, ResolveError> {
    let home = dirs::home_dir().ok_or(ResolveError::MissingHome)?;
    Ok(home.join(".m2").join("repository"))
}

fn sha256_file(path: &Path) -> Result<String, ResolveError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename = "project")]
struct Pom {
    #[serde(default)]
    dependencies: PomDependencies,
}

impl Pom {
    fn read(path: &Path) -> Result<Self, ResolveError> {
        let raw = fs::read_to_string(path)?;
        Ok(quick_xml::de::from_str(&raw)?)
    }
}

#[derive(Debug, Default, serde::Deserialize)]
struct PomDependencies {
    #[serde(rename = "dependency", default)]
    dependencies: Vec<PomDependency>,
}

impl IntoIterator for PomDependencies {
    type Item = PomDependency;
    type IntoIter = std::vec::IntoIter<PomDependency>;

    fn into_iter(self) -> Self::IntoIter {
        self.dependencies.into_iter()
    }
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PomDependency {
    group_id: Option<String>,
    artifact_id: Option<String>,
    version: Option<String>,
    #[serde(default)]
    scope: PomScope,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    exclusions: PomExclusions,
}

impl PomDependency {
    fn coordinate(&self) -> Option<Coordinate> {
        Some(Coordinate::new(
            self.group_id.as_ref()?,
            self.artifact_id.as_ref()?,
            self.version.as_ref()?,
        ))
    }

    fn scope(&self) -> Scope {
        self.scope.0
    }

    fn exclusions(&self) -> Vec<Coordinate> {
        self.exclusions
            .exclusions
            .iter()
            .map(|exclusion| {
                Coordinate::new(
                    exclusion.group_id.as_deref().unwrap_or_default(),
                    exclusion.artifact_id.as_deref().unwrap_or_default(),
                    "",
                )
            })
            .filter(|coordinate| !coordinate.group.is_empty() && !coordinate.artifact.is_empty())
            .collect()
    }
}

#[derive(Debug, Default, serde::Deserialize)]
struct PomExclusions {
    #[serde(rename = "exclusion", default)]
    exclusions: Vec<PomExclusion>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PomExclusion {
    group_id: Option<String>,
    artifact_id: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct PomScope(Scope);

impl Default for PomScope {
    fn default() -> Self {
        Self(Scope::Compile)
    }
}

impl<'de> serde::Deserialize<'de> for PomScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let scope = Option::<String>::deserialize(deserializer)?;
        Ok(Self(match scope.as_deref() {
            None | Some("") | Some("compile") => Scope::Compile,
            Some("runtime") => Scope::Runtime,
            Some("test") => Scope::Test,
            Some("provided") => Scope::Provided,
            Some(_) => Scope::Compile,
        }))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Lockfile(#[from] LockfileError),
    #[error("could not determine home directory for Maven local repository")]
    MissingHome,
    #[error("artifact `{0}` is missing locally and --offline was used")]
    OfflineMissing(String),
    #[error("POM `{pom}` uses unsupported inherited or unresolved property `{value}`")]
    UnsupportedPomProperty { pom: String, value: String },
    #[error("failed to create HTTP client: {0}")]
    Http(#[from] reqwest::Error),
    #[error("failed to download `{url}`: {source}")]
    Download { url: String, source: reqwest::Error },
    #[error("failed filesystem operation: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse Maven POM: {0}")]
    PomXml(#[from] quick_xml::DeError),
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::manifest::DeclaredDependency;

    fn write_artifact(repo: &Path, coordinate: &Coordinate, pom_body: &str) {
        fs::create_dir_all(coordinate.local_dir(repo)).unwrap();
        fs::write(coordinate.pom_path(repo), pom_body).unwrap();
        fs::write(coordinate.jar_path(repo), format!("jar for {coordinate}")).unwrap();
    }

    #[test]
    fn parses_pom_dependencies() {
        let dir = TempDir::new().unwrap();
        let pom = dir.path().join("demo.pom");
        fs::write(
            &pom,
            r#"
            <project>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>dep</artifactId>
                  <version>1.0.0</version>
                  <scope>runtime</scope>
                </dependency>
              </dependencies>
            </project>
            "#,
        )
        .unwrap();

        let parsed = Pom::read(&pom).unwrap();
        let dependency = parsed.dependencies.dependencies.first().unwrap();

        assert_eq!(
            dependency.coordinate().unwrap().to_string(),
            "com.example:dep:1.0.0"
        );
        assert_eq!(dependency.scope(), Scope::Runtime);
    }

    #[test]
    fn resolves_transitive_runtime_graph_and_excludes_optional() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let root = Coordinate::new("com.example", "root", "1.0.0");
        let child = Coordinate::new("com.example", "child", "1.0.0");
        let optional = Coordinate::new("com.example", "optional", "1.0.0");

        write_artifact(
            &repo,
            &root,
            r#"
            <project><dependencies>
              <dependency><groupId>com.example</groupId><artifactId>child</artifactId><version>1.0.0</version><scope>runtime</scope></dependency>
              <dependency><groupId>com.example</groupId><artifactId>optional</artifactId><version>1.0.0</version><optional>true</optional></dependency>
            </dependencies></project>
            "#,
        );
        write_artifact(&repo, &child, "<project/>");
        write_artifact(&repo, &optional, "<project/>");

        let resolver = Resolver::new(repo, true, false).unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                coordinate: root,
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        let coordinates = resolved
            .iter()
            .map(|artifact| artifact.coordinate.to_string())
            .collect::<Vec<_>>();

        assert!(coordinates.contains(&"com.example:root:1.0.0".to_string()));
        assert!(coordinates.contains(&"com.example:child:1.0.0".to_string()));
        assert!(!coordinates.contains(&"com.example:optional:1.0.0".to_string()));
    }

    #[test]
    fn applies_exclusions() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let root = Coordinate::new("com.example", "root", "1.0.0");
        let excluded = Coordinate::new("com.example", "excluded", "1.0.0");

        write_artifact(
            &repo,
            &root,
            r#"
            <project><dependencies>
              <dependency><groupId>com.example</groupId><artifactId>excluded</artifactId><version>1.0.0</version></dependency>
            </dependencies></project>
            "#,
        );
        write_artifact(&repo, &excluded, "<project/>");

        let resolver = Resolver::new(repo, true, false).unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                coordinate: root,
                scope: Scope::Compile,
                exclusions: vec![Coordinate::new("com.example", "excluded", "")],
            }])
            .unwrap();

        assert_eq!(resolved.len(), 1);
    }

    #[test]
    fn nearest_wins_conflict_resolution_is_deterministic() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let root = Coordinate::new("com.example", "root", "1.0.0");
        let winner = Coordinate::new("com.example", "shared", "1.0.0");
        let loser_parent = Coordinate::new("com.example", "loser-parent", "1.0.0");
        let loser = Coordinate::new("com.example", "shared", "2.0.0");

        write_artifact(
            &repo,
            &root,
            r#"
            <project><dependencies>
              <dependency><groupId>com.example</groupId><artifactId>shared</artifactId><version>1.0.0</version></dependency>
              <dependency><groupId>com.example</groupId><artifactId>loser-parent</artifactId><version>1.0.0</version></dependency>
            </dependencies></project>
            "#,
        );
        write_artifact(&repo, &winner, "<project/>");
        write_artifact(
            &repo,
            &loser_parent,
            r#"
            <project><dependencies>
              <dependency><groupId>com.example</groupId><artifactId>shared</artifactId><version>2.0.0</version></dependency>
            </dependencies></project>
            "#,
        );
        write_artifact(&repo, &loser, "<project/>");

        let resolver = Resolver::new(repo, true, false).unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                coordinate: root,
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        let shared = resolved
            .iter()
            .find(|artifact| artifact.coordinate.artifact == "shared")
            .unwrap();

        assert_eq!(shared.coordinate.version, "1.0.0");
    }
}
