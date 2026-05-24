use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs,
    io::Read,
    path::{Path, PathBuf},
    thread,
};

use reqwest::blocking::Client;
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::{
    config::GlobalConfig,
    lockfile::{LockedArtifact, Lockfile, LockfileError},
    manifest::{DeclaredDependency, Manifest, ManifestError},
    maven::{ArtifactCoordinate, ArtifactIdentity, ArtifactType, Coordinate, Repository, Scope},
    pom::{EffectivePom, Pom, PomError},
    settings::MavenSettings,
};

#[derive(Debug, Clone)]
pub struct ResolveOptions {
    pub project_dir: PathBuf,
    pub offline: bool,
    pub refresh: bool,
    pub local_repo: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct QueuedDependency {
    artifact: ArtifactCoordinate,
    scope: Scope,
    depth: usize,
    exclusions: Vec<Coordinate>,
    path: Vec<ArtifactCoordinate>,
}

#[derive(Debug, Clone)]
struct ResolvedArtifact {
    artifact: ArtifactCoordinate,
    scope: Scope,
    depth: usize,
    pom_path: PathBuf,
    artifact_path: PathBuf,
    source: String,
}

pub fn resolve_project(options: ResolveOptions) -> Result<Lockfile, ResolveError> {
    let manifest_path = options.project_dir.join("angra.toml");
    let manifest = Manifest::read(&manifest_path)?;
    let dependencies = manifest.declared_dependencies()?;
    let global_config = GlobalConfig::load()?;
    let settings = MavenSettings::load()?;
    let repositories =
        manifest.declared_repositories(&global_config.repositories(), &settings.repositories);
    let local_repo = options
        .local_repo
        .or(settings.local_repository)
        .map(Ok)
        .unwrap_or_else(default_local_repo)?;
    let resolver = Resolver::new(local_repo, repositories, options.offline, options.refresh)?;
    let artifacts = resolver.resolve(dependencies)?;

    let lockfile = Lockfile::new(
        artifacts
            .into_iter()
            .map(|artifact| {
                let sha = sha256_file(&artifact.artifact_path)?;
                Ok(LockedArtifact::new(
                    &artifact.artifact,
                    artifact.scope,
                    &artifact.source,
                    artifact.pom_path,
                    artifact.artifact_path,
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
    repositories: Vec<Repository>,
    offline: bool,
    refresh: bool,
    client: Client,
}

impl Resolver {
    fn new(
        local_repo: PathBuf,
        repositories: Vec<Repository>,
        offline: bool,
        refresh: bool,
    ) -> Result<Self, ResolveError> {
        Ok(Self {
            local_repo,
            repositories,
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
            .map(|dependency| dependency.artifact.identity())
            .collect::<HashSet<_>>();

        for dependency in dependencies {
            let artifact = dependency.artifact;
            queue.push_back(QueuedDependency {
                path: vec![artifact.clone()],
                artifact,
                scope: dependency.scope,
                depth: 0,
                exclusions: dependency.exclusions,
            });
        }

        let mut selected: BTreeMap<ArtifactIdentity, ResolvedArtifact> = BTreeMap::new();
        let mut selected_exclusions: BTreeMap<ArtifactIdentity, Vec<Coordinate>> = BTreeMap::new();
        let mut visited_versions = HashSet::new();

        while let Some(first) = queue.pop_front() {
            let batch = drain_depth_batch(first, &mut queue);
            let mut batch_identities = HashSet::new();
            let candidates = batch
                .into_iter()
                .filter(|item| item.scope.is_runtime_graph())
                .filter(|item| {
                    !item.exclusions.iter().any(|exclusion| {
                        exclusion.group == item.artifact.coordinate.group
                            && exclusion.artifact == item.artifact.coordinate.artifact
                    })
                })
                .filter(|item| {
                    let identity = item.artifact.identity();
                    if let Some(existing) = selected.get(&identity)
                        && existing.depth <= item.depth
                    {
                        return false;
                    }

                    batch_identities.insert(identity)
                })
                .collect::<Vec<_>>();

            let mut fetched_sources = self.ensure_artifacts_parallel(&candidates)?;

            for item in candidates {
                let identity = item.artifact.identity();
                let fetched = fetched_sources.remove(&identity).unwrap_or_else(|| {
                    unreachable!("parallel fetch did not return source for {identity:?}")
                });

                selected.insert(
                    identity.clone(),
                    ResolvedArtifact {
                        artifact: item.artifact.clone(),
                        scope: item.scope,
                        depth: item.depth,
                        pom_path: item.artifact.pom_path(&self.local_repo),
                        artifact_path: item.artifact.artifact_path(&self.local_repo),
                        source: fetched,
                    },
                );
                selected_exclusions.insert(identity, item.exclusions.clone());

                let version_key = item.artifact.to_string();
                if !visited_versions.insert(version_key) {
                    continue;
                }

                let pom = self
                    .effective_pom(&item.artifact.coordinate)
                    .map_err(|source| {
                        ResolveError::with_dependency_path(item.path.clone(), source)
                    })?;
                let property_context = pom.property_context();
                for dependency in pom.dependencies {
                    let Some(dependency_scope) = dependency.graph_scope() else {
                        continue;
                    };
                    if !dependency_scope.is_runtime_graph() {
                        continue;
                    }

                    let Some(resolved_dependency) = dependency
                        .resolve(
                            &property_context,
                            &item.artifact.coordinate.to_string(),
                            &pom.dependency_management,
                        )
                        .map_err(|source| {
                            ResolveError::with_dependency_path(item.path.clone(), source.into())
                        })?
                    else {
                        continue;
                    };
                    let artifact = resolved_dependency.artifact;
                    if !resolved_dependency.scope.is_runtime_graph() {
                        continue;
                    }
                    if dependency.optional && !direct_identities.contains(&artifact.identity()) {
                        continue;
                    }

                    let mut exclusions = selected_exclusions
                        .get(&item.artifact.identity())
                        .cloned()
                        .unwrap_or_default();
                    exclusions.extend(resolved_dependency.exclusions);

                    let mut path = item.path.clone();
                    path.push(artifact.clone());

                    queue.push_back(QueuedDependency {
                        artifact,
                        scope: combine_scope(item.scope, resolved_dependency.scope),
                        depth: item.depth + 1,
                        exclusions,
                        path,
                    });
                }
            }
        }

        Ok(selected.into_values().collect())
    }

    fn ensure_artifacts_parallel(
        &self,
        items: &[QueuedDependency],
    ) -> Result<BTreeMap<ArtifactIdentity, String>, ResolveError> {
        let mut fetched = BTreeMap::new();
        let parallelism = thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(4)
            .max(1);

        for chunk in items.chunks(parallelism) {
            let results = thread::scope(|scope| {
                let handles = chunk
                    .iter()
                    .map(|item| {
                        scope.spawn(move || {
                            self.ensure_artifact(&item.artifact)
                                .map(|source| (item.artifact.identity(), source))
                                .map_err(|source| {
                                    ResolveError::with_dependency_path(item.path.clone(), source)
                                })
                        })
                    })
                    .collect::<Vec<_>>();

                handles
                    .into_iter()
                    .map(|handle| handle.join())
                    .collect::<Vec<_>>()
            });

            for result in results {
                let (identity, source) =
                    result.map_err(|_| ResolveError::ParallelWorkerPanic)??;
                fetched.insert(identity, source);
            }
        }

        Ok(fetched)
    }

    fn effective_pom(&self, coordinate: &Coordinate) -> Result<EffectivePom, ResolveError> {
        self.effective_pom_inner(coordinate, &mut Vec::new())
    }

    fn effective_pom_inner(
        &self,
        coordinate: &Coordinate,
        stack: &mut Vec<String>,
    ) -> Result<EffectivePom, ResolveError> {
        let key = coordinate.to_string();
        if let Some(cycle_start) = stack.iter().position(|seen| seen == &key) {
            let mut cycle = stack[cycle_start..].to_vec();
            cycle.push(key);
            return Err(ResolveError::PomCycle(cycle.join(" -> ")));
        }

        stack.push(key.clone());
        self.ensure_pom(coordinate)?;
        let raw = Pom::read(&coordinate.pom_path(&self.local_repo))?;
        let parent = if let Some(parent) = raw.parent_coordinate(&key)? {
            Some(self.effective_pom_inner(&parent, stack)?)
        } else {
            None
        };

        let mut effective = raw.merge_with_parent(parent);
        let properties = effective.property_context();
        for dependency in raw.dependency_management_entries() {
            if dependency.is_bom_import() {
                let Some(bom) = dependency.coordinate(&properties, &key)? else {
                    continue;
                };
                let bom = self.effective_pom_inner(&bom, stack)?;
                effective
                    .dependency_management
                    .extend(bom.dependency_management);
                continue;
            }

            if let Some((identity, managed)) = dependency.managed_dependency(&properties, &key)? {
                effective.dependency_management.insert(identity, managed);
            }
        }

        stack.pop();
        Ok(effective)
    }

    fn ensure_artifact(&self, artifact: &ArtifactCoordinate) -> Result<String, ResolveError> {
        let pom_path = artifact.pom_path(&self.local_repo);
        let artifact_path = artifact.artifact_path(&self.local_repo);
        let descriptor_only =
            artifact.artifact_type == ArtifactType::Pom && artifact.classifier.is_none();

        let source =
            if !self.refresh && pom_path.exists() && (descriptor_only || artifact_path.exists()) {
                "local".to_string()
            } else {
                if self.offline {
                    return Err(ResolveError::OfflineMissing(artifact.to_string()));
                }

                self.download_artifact(artifact, &pom_path, &artifact_path, descriptor_only)?
            };

        Ok(source)
    }

    fn ensure_pom(&self, coordinate: &Coordinate) -> Result<String, ResolveError> {
        let pom_path = coordinate.pom_path(&self.local_repo);

        let source = if !self.refresh && pom_path.exists() {
            "local".to_string()
        } else {
            if self.offline {
                return Err(ResolveError::OfflineMissing(coordinate.to_string()));
            }

            self.download_pom(coordinate, &pom_path)?
        };

        Ok(source)
    }

    fn download_artifact(
        &self,
        artifact: &ArtifactCoordinate,
        pom_path: &Path,
        artifact_path: &Path,
        descriptor_only: bool,
    ) -> Result<String, ResolveError> {
        fs::create_dir_all(artifact.local_dir(&self.local_repo))?;
        let mut not_found = Vec::new();
        let needs_pom = self.refresh || !pom_path.exists();

        for repository in &self.repositories {
            let result: Result<String, ResolveError> = (|| {
                if needs_pom {
                    self.download(&repository.pom_url(&artifact.coordinate), pom_path)?;
                }
                if !descriptor_only {
                    self.download(&repository.artifact_url(artifact), artifact_path)?;
                }
                Ok(repository.name.clone())
            })();

            match result {
                Ok(source) => return Ok(source),
                Err(error) if error.is_not_found() => {
                    not_found.push(repository.name.clone());
                }
                Err(error) => return Err(error),
            }
        }

        Err(ResolveError::ArtifactNotFound {
            artifact: artifact.to_string(),
            repositories: not_found,
        })
    }

    fn download_pom(
        &self,
        coordinate: &Coordinate,
        pom_path: &Path,
    ) -> Result<String, ResolveError> {
        fs::create_dir_all(coordinate.local_dir(&self.local_repo))?;
        let mut not_found = Vec::new();

        for repository in &self.repositories {
            let result = self
                .download(&repository.pom_url(coordinate), pom_path)
                .map(|_| repository.name.clone());

            match result {
                Ok(source) => return Ok(source),
                Err(error) if error.is_not_found() => {
                    not_found.push(repository.name.clone());
                }
                Err(error) => return Err(error),
            }
        }

        Err(ResolveError::ArtifactNotFound {
            artifact: coordinate.to_string(),
            repositories: not_found,
        })
    }

    fn download(&self, url: &str, destination: &Path) -> Result<(), ResolveError> {
        let bytes = self.download_bytes(url)?;
        let checksum_url = format!("{url}.sha1");
        let checksum_bytes = self.download_bytes(&checksum_url)?;
        verify_sha1_checksum(url, &bytes, &checksum_url, &checksum_bytes)?;

        fs::write(destination, bytes)?;
        fs::write(checksum_path(destination), checksum_bytes)?;
        Ok(())
    }

    fn download_bytes(&self, url: &str) -> Result<Vec<u8>, ResolveError> {
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
        Ok(bytes.to_vec())
    }
}

fn combine_scope(parent: Scope, child: Scope) -> Scope {
    match (parent, child) {
        (Scope::Runtime, _) | (_, Scope::Runtime) => Scope::Runtime,
        _ => Scope::Compile,
    }
}

fn drain_depth_batch(
    first: QueuedDependency,
    queue: &mut VecDeque<QueuedDependency>,
) -> Vec<QueuedDependency> {
    let depth = first.depth;
    let mut batch = vec![first];

    while queue.front().is_some_and(|item| item.depth == depth) {
        let Some(item) = queue.pop_front() else {
            break;
        };
        batch.push(item);
    }

    batch
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

    Ok(hex_bytes(&hasher.finalize()))
}

fn sha1_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    hex_bytes(&hasher.finalize())
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);

    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }

    output
}

fn checksum_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(".sha1");
    PathBuf::from(value)
}

fn parse_sha1_checksum(url: &str, bytes: &[u8]) -> Result<String, ResolveError> {
    let raw = std::str::from_utf8(bytes).map_err(|_| ResolveError::InvalidChecksum {
        url: url.to_string(),
        value: String::from_utf8_lossy(bytes).into_owned(),
    })?;
    let trimmed = raw.trim();
    let checksum = if trimmed
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("sha"))
        || trimmed
            .get(..2)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("md"))
    {
        trimmed.split_whitespace().last()
    } else {
        trimmed.split_whitespace().next()
    }
    .unwrap_or_default()
    .to_ascii_lowercase();

    if checksum.len() != 40
        || !checksum
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(ResolveError::InvalidChecksum {
            url: url.to_string(),
            value: trimmed.to_string(),
        });
    }

    Ok(checksum)
}

fn verify_sha1_checksum(
    artifact_url: &str,
    artifact_bytes: &[u8],
    checksum_url: &str,
    checksum_bytes: &[u8],
) -> Result<(), ResolveError> {
    let expected = parse_sha1_checksum(checksum_url, checksum_bytes)?;
    let actual = sha1_bytes(artifact_bytes);

    if expected != actual {
        return Err(ResolveError::ChecksumMismatch {
            url: artifact_url.to_string(),
            expected,
            actual,
        });
    }

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    #[error(transparent)]
    Settings(#[from] crate::settings::SettingsError),
    #[error(transparent)]
    Lockfile(#[from] LockfileError),
    #[error("could not determine home directory for Maven local repository")]
    MissingHome,
    #[error("artifact `{0}` is missing locally and --offline was used")]
    OfflineMissing(String),
    #[error("artifact `{artifact}` was not found in configured repositories {repositories:?}")]
    ArtifactNotFound {
        artifact: String,
        repositories: Vec<String>,
    },
    #[error("cyclic POM inheritance or BOM import path `{0}`")]
    PomCycle(String),
    #[error("{0}")]
    Pom(String),
    #[error("failed to create HTTP client: {0}")]
    Http(#[from] reqwest::Error),
    #[error("failed to download `{url}`: {source}")]
    Download { url: String, source: reqwest::Error },
    #[error("checksum mismatch for `{url}`: expected {expected}, got {actual}")]
    ChecksumMismatch {
        url: String,
        expected: String,
        actual: String,
    },
    #[error("invalid SHA-1 checksum from `{url}`: {value:?}")]
    InvalidChecksum { url: String, value: String },
    #[error("failed filesystem operation: {0}")]
    Io(#[from] std::io::Error),
    #[error("parallel resolver worker panicked")]
    ParallelWorkerPanic,
    #[error("{source}")]
    DependencyPath {
        path: Vec<ArtifactCoordinate>,
        #[source]
        source: Box<ResolveError>,
    },
}

impl From<PomError> for ResolveError {
    fn from(error: PomError) -> Self {
        Self::Pom(error.to_string())
    }
}

impl ResolveError {
    fn with_dependency_path(path: Vec<ArtifactCoordinate>, source: ResolveError) -> Self {
        match source {
            Self::DependencyPath { .. } => source,
            source => Self::DependencyPath {
                path,
                source: Box::new(source),
            },
        }
    }

    pub fn dependency_path(&self) -> Option<&[ArtifactCoordinate]> {
        match self {
            Self::DependencyPath { path, .. } => Some(path),
            _ => None,
        }
    }

    pub fn root_cause(&self) -> &ResolveError {
        match self {
            Self::DependencyPath { source, .. } => source.root_cause(),
            _ => self,
        }
    }

    fn is_not_found(&self) -> bool {
        match self {
            Self::Download { source, .. } => {
                source.status() == Some(reqwest::StatusCode::NOT_FOUND)
            }
            _ => false,
        }
    }
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

    fn write_typed_artifact(repo: &Path, artifact: &ArtifactCoordinate, pom_body: &str) {
        fs::create_dir_all(artifact.local_dir(repo)).unwrap();
        fs::write(artifact.pom_path(repo), pom_body).unwrap();
        if artifact.artifact_type != ArtifactType::Pom || artifact.classifier.is_some() {
            fs::write(
                artifact.artifact_path(repo),
                format!("artifact for {artifact}"),
            )
            .unwrap();
        }
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

        let resolver = Resolver::new(repo, vec![Repository::maven_central()], true, false).unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                artifact: ArtifactCoordinate::jar(root),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        let coordinates = resolved
            .iter()
            .map(|artifact| artifact.artifact.coordinate.to_string())
            .collect::<Vec<_>>();

        assert!(coordinates.contains(&"com.example:root:1.0.0".to_string()));
        assert!(coordinates.contains(&"com.example:child:1.0.0".to_string()));
        assert!(!coordinates.contains(&"com.example:optional:1.0.0".to_string()));
    }

    #[test]
    fn resolves_same_depth_direct_dependencies() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let first = Coordinate::new("com.example", "first", "1.0.0");
        let second = Coordinate::new("com.example", "second", "1.0.0");

        write_artifact(&repo, &first, "<project/>");
        write_artifact(&repo, &second, "<project/>");

        let resolver = Resolver::new(repo, vec![Repository::maven_central()], true, false).unwrap();
        let resolved = resolver
            .resolve(vec![
                DeclaredDependency {
                    alias: "first".to_string(),
                    artifact: ArtifactCoordinate::jar(first),
                    scope: Scope::Compile,
                    exclusions: Vec::new(),
                },
                DeclaredDependency {
                    alias: "second".to_string(),
                    artifact: ArtifactCoordinate::jar(second),
                    scope: Scope::Compile,
                    exclusions: Vec::new(),
                },
            ])
            .unwrap();

        let coordinates = resolved
            .iter()
            .map(|artifact| artifact.artifact.coordinate.to_string())
            .collect::<Vec<_>>();

        assert!(coordinates.contains(&"com.example:first:1.0.0".to_string()));
        assert!(coordinates.contains(&"com.example:second:1.0.0".to_string()));
    }

    #[test]
    fn resolves_transitive_dependencies_with_pom_properties() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let root = Coordinate::new("com.example", "root", "1.0.0");
        let child = Coordinate::new("com.example", "child", "1.0.0");

        write_artifact(
            &repo,
            &root,
            r#"
            <project>
              <groupId>com.example</groupId>
              <artifactId>root</artifactId>
              <version>1.0.0</version>
              <properties>
                <child.version>${project.version}</child.version>
              </properties>
              <dependencies>
                <dependency>
                  <groupId>${project.groupId}</groupId>
                  <artifactId>child</artifactId>
                  <version>${child.version}</version>
                </dependency>
              </dependencies>
            </project>
            "#,
        );
        write_artifact(&repo, &child, "<project/>");

        let resolver = Resolver::new(repo, vec![Repository::maven_central()], true, false).unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                artifact: ArtifactCoordinate::jar(root),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        let coordinates = resolved
            .iter()
            .map(|artifact| artifact.artifact.coordinate.to_string())
            .collect::<Vec<_>>();

        assert!(coordinates.contains(&"com.example:child:1.0.0".to_string()));
    }

    #[test]
    fn inherits_parent_pom_properties() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let parent = Coordinate::new("com.example", "parent", "1.0.0");
        let root = Coordinate::new("com.example", "root", "1.0.0");
        let child = Coordinate::new("com.example", "child", "1.2.3");

        write_artifact(
            &repo,
            &parent,
            r#"
            <project>
              <groupId>com.example</groupId>
              <artifactId>parent</artifactId>
              <version>1.0.0</version>
              <properties>
                <child.version>1.2.3</child.version>
              </properties>
            </project>
            "#,
        );
        write_artifact(
            &repo,
            &root,
            r#"
            <project>
              <parent>
                <groupId>com.example</groupId>
                <artifactId>parent</artifactId>
                <version>1.0.0</version>
              </parent>
              <artifactId>root</artifactId>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>child</artifactId>
                  <version>${child.version}</version>
                </dependency>
              </dependencies>
            </project>
            "#,
        );
        write_artifact(&repo, &child, "<project/>");

        let resolver = Resolver::new(repo, vec![Repository::maven_central()], true, false).unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                artifact: ArtifactCoordinate::jar(root),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        let coordinates = resolved
            .iter()
            .map(|artifact| artifact.artifact.coordinate.to_string())
            .collect::<Vec<_>>();

        assert!(coordinates.contains(&"com.example:child:1.2.3".to_string()));
    }

    #[test]
    fn applies_dependency_management_versions() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let root = Coordinate::new("com.example", "root", "1.0.0");
        let child = Coordinate::new("com.example", "child", "2.0.0");

        write_artifact(
            &repo,
            &root,
            r#"
            <project>
              <dependencyManagement>
                <dependencies>
                  <dependency>
                    <groupId>com.example</groupId>
                    <artifactId>child</artifactId>
                    <version>2.0.0</version>
                  </dependency>
                </dependencies>
              </dependencyManagement>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>child</artifactId>
                </dependency>
              </dependencies>
            </project>
            "#,
        );
        write_artifact(&repo, &child, "<project/>");

        let resolver = Resolver::new(repo, vec![Repository::maven_central()], true, false).unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                artifact: ArtifactCoordinate::jar(root),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        let coordinates = resolved
            .iter()
            .map(|artifact| artifact.artifact.coordinate.to_string())
            .collect::<Vec<_>>();

        assert!(coordinates.contains(&"com.example:child:2.0.0".to_string()));
    }

    #[test]
    fn applies_dependency_management_scope() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let root = Coordinate::new("com.example", "root", "1.0.0");
        let child = Coordinate::new("com.example", "child", "2.0.0");

        write_artifact(
            &repo,
            &root,
            r#"
            <project>
              <dependencyManagement>
                <dependencies>
                  <dependency>
                    <groupId>com.example</groupId>
                    <artifactId>child</artifactId>
                    <version>2.0.0</version>
                    <scope>test</scope>
                  </dependency>
                </dependencies>
              </dependencyManagement>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>child</artifactId>
                </dependency>
              </dependencies>
            </project>
            "#,
        );
        write_artifact(&repo, &child, "<project/>");

        let resolver = Resolver::new(repo, vec![Repository::maven_central()], true, false).unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                artifact: ArtifactCoordinate::jar(root),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        let coordinates = resolved
            .iter()
            .map(|artifact| artifact.artifact.coordinate.to_string())
            .collect::<Vec<_>>();

        assert!(!coordinates.contains(&"com.example:child:2.0.0".to_string()));
    }

    #[test]
    fn imports_bom_dependency_management() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let root = Coordinate::new("com.example", "root", "1.0.0");
        let bom = Coordinate::new("com.example", "bom", "1.0.0");
        let child = Coordinate::new("com.example", "child", "3.0.0");

        write_artifact(
            &repo,
            &bom,
            r#"
            <project>
              <groupId>com.example</groupId>
              <artifactId>bom</artifactId>
              <version>1.0.0</version>
              <dependencyManagement>
                <dependencies>
                  <dependency>
                    <groupId>com.example</groupId>
                    <artifactId>child</artifactId>
                    <version>3.0.0</version>
                  </dependency>
                </dependencies>
              </dependencyManagement>
            </project>
            "#,
        );
        write_artifact(
            &repo,
            &root,
            r#"
            <project>
              <dependencyManagement>
                <dependencies>
                  <dependency>
                    <groupId>com.example</groupId>
                    <artifactId>bom</artifactId>
                    <version>1.0.0</version>
                    <type>pom</type>
                    <scope>import</scope>
                  </dependency>
                </dependencies>
              </dependencyManagement>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>child</artifactId>
                </dependency>
              </dependencies>
            </project>
            "#,
        );
        write_artifact(&repo, &child, "<project/>");

        let resolver = Resolver::new(repo, vec![Repository::maven_central()], true, false).unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                artifact: ArtifactCoordinate::jar(root),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        let coordinates = resolved
            .iter()
            .map(|artifact| artifact.artifact.coordinate.to_string())
            .collect::<Vec<_>>();

        assert!(coordinates.contains(&"com.example:child:3.0.0".to_string()));
    }

    #[test]
    fn resolves_transitive_classified_dependency_artifact() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let root = Coordinate::new("com.example", "root", "1.0.0");
        let native = ArtifactCoordinate::new(
            Coordinate::new("com.example", "native", "1.0.0"),
            ArtifactType::Jar,
            Some("linux-aarch64".to_string()),
        );

        write_artifact(
            &repo,
            &root,
            r#"
            <project><dependencies>
              <dependency>
                <groupId>com.example</groupId>
                <artifactId>native</artifactId>
                <version>1.0.0</version>
                <classifier>linux-aarch64</classifier>
              </dependency>
            </dependencies></project>
            "#,
        );
        write_typed_artifact(&repo, &native, "<project/>");

        let resolver = Resolver::new(repo, vec![Repository::maven_central()], true, false).unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                artifact: ArtifactCoordinate::jar(root),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        let native = resolved
            .iter()
            .find(|artifact| artifact.artifact.coordinate.artifact == "native")
            .unwrap();

        assert_eq!(native.artifact.classifier.as_deref(), Some("linux-aarch64"));
        assert!(
            native
                .artifact_path
                .ends_with("native-1.0.0-linux-aarch64.jar")
        );
    }

    #[test]
    fn resolves_transitive_war_dependency_artifact() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let root = Coordinate::new("com.example", "root", "1.0.0");
        let webapp = ArtifactCoordinate::new(
            Coordinate::new("com.example", "webapp", "1.0.0"),
            ArtifactType::War,
            None,
        );

        write_artifact(
            &repo,
            &root,
            r#"
            <project><dependencies>
              <dependency>
                <groupId>com.example</groupId>
                <artifactId>webapp</artifactId>
                <version>1.0.0</version>
                <type>war</type>
              </dependency>
            </dependencies></project>
            "#,
        );
        write_typed_artifact(&repo, &webapp, "<project/>");

        let resolver = Resolver::new(repo, vec![Repository::maven_central()], true, false).unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                artifact: ArtifactCoordinate::jar(root),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        let webapp = resolved
            .iter()
            .find(|artifact| artifact.artifact.coordinate.artifact == "webapp")
            .unwrap();

        assert_eq!(webapp.artifact.artifact_type, ArtifactType::War);
        assert!(webapp.artifact_path.ends_with("webapp-1.0.0.war"));
    }

    #[test]
    fn resolves_pom_dependency_without_artifact_file() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let root = Coordinate::new("com.example", "root", "1.0.0");
        let descriptor =
            ArtifactCoordinate::pom(Coordinate::new("com.example", "descriptor", "1.0.0"));

        write_artifact(
            &repo,
            &root,
            r#"
            <project><dependencies>
              <dependency>
                <groupId>com.example</groupId>
                <artifactId>descriptor</artifactId>
                <version>1.0.0</version>
                <type>pom</type>
              </dependency>
            </dependencies></project>
            "#,
        );
        write_typed_artifact(&repo, &descriptor, "<project/>");

        let resolver = Resolver::new(repo, vec![Repository::maven_central()], true, false).unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                artifact: ArtifactCoordinate::jar(root),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        let descriptor = resolved
            .iter()
            .find(|artifact| artifact.artifact.coordinate.artifact == "descriptor")
            .unwrap();

        assert_eq!(descriptor.artifact.artifact_type, ArtifactType::Pom);
        assert_eq!(descriptor.artifact_path, descriptor.pom_path);
    }

    #[test]
    fn reports_dependency_path_for_missing_transitive_artifact() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let root = Coordinate::new("com.example", "root", "1.0.0");

        write_artifact(
            &repo,
            &root,
            r#"
            <project><dependencies>
              <dependency>
                <groupId>com.example</groupId>
                <artifactId>missing</artifactId>
                <version>1.0.0</version>
              </dependency>
            </dependencies></project>
            "#,
        );

        let resolver = Resolver::new(repo, vec![Repository::maven_central()], true, false).unwrap();
        let error = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                artifact: ArtifactCoordinate::jar(root),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap_err();

        let path = error.dependency_path().unwrap();

        assert_eq!(path[0].to_string(), "com.example:root:1.0.0");
        assert_eq!(path[1].to_string(), "com.example:missing:1.0.0");
        assert!(matches!(
            error.root_cause(),
            ResolveError::OfflineMissing(_)
        ));
    }

    #[test]
    fn parses_maven_sha1_checksum_formats() {
        assert_eq!(
            parse_sha1_checksum(
                "https://repo.example/artifact.jar.sha1",
                b"aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d  artifact.jar"
            )
            .unwrap(),
            "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"
        );
        assert_eq!(
            parse_sha1_checksum(
                "https://repo.example/artifact.jar.sha1",
                b"SHA1 (artifact.jar) = AAF4C61DDCC5E8A2DABEDE0F3B482CD9AEA9434D"
            )
            .unwrap(),
            "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"
        );
    }

    #[test]
    fn rejects_invalid_sha1_checksum() {
        let error =
            parse_sha1_checksum("https://repo.example/artifact.jar.sha1", b"not-a-checksum")
                .unwrap_err();

        assert!(matches!(error, ResolveError::InvalidChecksum { .. }));
    }

    #[test]
    fn hashes_download_bytes_with_sha1() {
        assert_eq!(
            sha1_bytes(b"hello"),
            "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d"
        );
    }

    #[test]
    fn rejects_sha1_checksum_mismatch() {
        let error = verify_sha1_checksum(
            "https://repo.example/artifact.jar",
            b"hello",
            "https://repo.example/artifact.jar.sha1",
            b"0000000000000000000000000000000000000000",
        )
        .unwrap_err();

        assert!(matches!(error, ResolveError::ChecksumMismatch { .. }));
    }

    #[test]
    fn ignores_unresolved_properties_in_non_runtime_scopes() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let root = Coordinate::new("com.example", "root", "1.0.0");
        let child = Coordinate::new("com.example", "child", "1.0.0");

        write_artifact(
            &repo,
            &root,
            r#"
            <project><dependencies>
              <dependency>
                <groupId>com.example</groupId>
                <artifactId>test-only</artifactId>
                <version>${inherited.test.version}</version>
                <scope>test</scope>
              </dependency>
              <dependency>
                <groupId>com.example</groupId>
                <artifactId>child</artifactId>
                <version>1.0.0</version>
              </dependency>
            </dependencies></project>
            "#,
        );
        write_artifact(&repo, &child, "<project/>");

        let resolver = Resolver::new(repo, vec![Repository::maven_central()], true, false).unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                artifact: ArtifactCoordinate::jar(root),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        let coordinates = resolved
            .iter()
            .map(|artifact| artifact.artifact.coordinate.to_string())
            .collect::<Vec<_>>();

        assert!(coordinates.contains(&"com.example:child:1.0.0".to_string()));
        assert!(!coordinates.iter().any(|coord| coord.contains("test-only")));
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

        let resolver = Resolver::new(repo, vec![Repository::maven_central()], true, false).unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                artifact: ArtifactCoordinate::jar(root),
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

        let resolver = Resolver::new(repo, vec![Repository::maven_central()], true, false).unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                artifact: ArtifactCoordinate::jar(root),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        let shared = resolved
            .iter()
            .find(|artifact| artifact.artifact.coordinate.artifact == "shared")
            .unwrap();

        assert_eq!(shared.artifact.coordinate.version, "1.0.0");
    }
}
