use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    thread,
};

use reqwest::blocking::Client;
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::{
    config::GlobalConfig,
    lockfile::{LockedArtifact, Lockfile, LockfileError},
    manifest::{
        DeclaredDependency, DeclaredManagedDependency, ManagedDependencyScope, Manifest,
        ManifestError,
    },
    maven::{
        ArtifactCoordinate, ArtifactIdentity, ArtifactType, ChecksumPolicy, Coordinate, Repository,
        RepositoryPolicy, Scope, VersionRange,
    },
    metadata::MavenMetadata,
    pom::{EffectivePom, ManagedDependency, Pom, PomError, ProfileActivationContext},
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
pub struct ResolveOutput {
    pub lockfile: Lockfile,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct QueuedDependency {
    artifact: ArtifactCoordinate,
    scope: Scope,
    depth: usize,
    exclusions: Vec<Coordinate>,
    path: Vec<ArtifactCoordinate>,
    repositories: Vec<Repository>,
}

#[derive(Debug, Clone)]
struct ResolvedArtifact {
    artifact: ArtifactCoordinate,
    requested_version: Option<String>,
    scope: Scope,
    depth: usize,
    pom_path: PathBuf,
    artifact_path: PathBuf,
    source: String,
}

#[derive(Debug, Clone)]
struct FetchedArtifact {
    artifact: ArtifactCoordinate,
    requested_version: Option<String>,
    source: String,
}

#[derive(Debug, Clone)]
struct ResolvedVersion {
    coordinate: Coordinate,
    requested_version: Option<String>,
}

impl ResolvedVersion {
    fn artifact(
        self,
        artifact_type: ArtifactType,
        classifier: Option<String>,
    ) -> ResolvedArtifactVersion {
        ResolvedArtifactVersion {
            artifact: ArtifactCoordinate::new(self.coordinate, artifact_type, classifier),
            requested_version: self.requested_version,
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedArtifactVersion {
    artifact: ArtifactCoordinate,
    requested_version: Option<String>,
}

pub fn resolve_project(options: ResolveOptions) -> Result<ResolveOutput, ResolveError> {
    let manifest_path = options.project_dir.join("angra.toml");
    let manifest = Manifest::read(&manifest_path)?;
    let dependencies = manifest.declared_dependencies()?;
    let dependency_management = manifest.declared_dependency_management()?;
    let global_config = GlobalConfig::load()?;
    let settings = MavenSettings::load()?;
    let mut activation_properties = settings.properties.clone();
    activation_properties.extend(manifest.resolver.maven.properties.clone());
    let profile_activation = ProfileActivationContext::new(
        manifest.resolver.maven.active_profiles.clone(),
        manifest.resolver.maven.inactive_profiles.clone(),
        activation_properties,
        manifest.resolver.maven.java_version.clone(),
        options.project_dir.clone(),
    );
    let mut repositories =
        manifest.declared_repositories(&global_config.repositories(), &settings.repositories);
    settings.apply_mirrors(&mut repositories);
    let local_repo = options
        .local_repo
        .or(settings.local_repository.clone())
        .map(Ok)
        .unwrap_or_else(default_local_repo)?;
    let resolver = Resolver::new_with_activation(
        local_repo,
        repositories,
        settings.clone(),
        profile_activation,
        options.offline,
        options.refresh,
    )?;
    let artifacts =
        resolver.resolve_with_dependency_management(dependencies, dependency_management)?;
    let warnings = resolver.into_warnings();

    let lockfile = Lockfile::new(
        artifacts
            .into_iter()
            .map(|artifact| {
                let sha = sha256_file(&artifact.artifact_path)?;
                Ok(LockedArtifact::new(
                    &artifact.artifact,
                    artifact.requested_version.as_deref(),
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
    Ok(ResolveOutput { lockfile, warnings })
}

struct Resolver {
    local_repo: PathBuf,
    repositories: Vec<Repository>,
    settings: MavenSettings,
    profile_activation: ProfileActivationContext,
    offline: bool,
    refresh: bool,
    client: Client,
    warnings: Mutex<Vec<String>>,
    pom_cache: Mutex<BTreeMap<String, EffectivePom>>,
    path_pom_cache: Mutex<BTreeMap<PathBuf, EffectivePom>>,
}

impl Resolver {
    #[cfg(test)]
    fn new(
        local_repo: PathBuf,
        repositories: Vec<Repository>,
        settings: MavenSettings,
        offline: bool,
        refresh: bool,
    ) -> Result<Self, ResolveError> {
        Self::new_with_activation(
            local_repo,
            repositories,
            settings,
            ProfileActivationContext::default(),
            offline,
            refresh,
        )
    }

    fn new_with_activation(
        local_repo: PathBuf,
        repositories: Vec<Repository>,
        settings: MavenSettings,
        profile_activation: ProfileActivationContext,
        offline: bool,
        refresh: bool,
    ) -> Result<Self, ResolveError> {
        let warnings = Mutex::new(Vec::new());
        for repo in &repositories {
            if let ChecksumPolicy::Unknown(ref policy) = repo.releases.checksum_policy {
                warnings.lock().expect("warnings list poisoned").push(format!("unknown checksum policy '{policy}' for repository '{}', defaulting to fail", repo.name));
            }
            if let ChecksumPolicy::Unknown(ref policy) = repo.snapshots.checksum_policy {
                warnings.lock().expect("warnings list poisoned").push(format!("unknown checksum policy '{policy}' for repository '{}', defaulting to fail", repo.name));
            }
        }
        Ok(Self {
            local_repo,
            repositories,
            settings,
            profile_activation,
            offline,
            refresh,
            client: Client::builder().http1_only().build()?,
            warnings,
            pom_cache: Mutex::new(BTreeMap::new()),
            path_pom_cache: Mutex::new(BTreeMap::new()),
        })
    }

    fn into_warnings(self) -> Vec<String> {
        self.warnings.into_inner().expect("warnings list poisoned")
    }

    #[cfg(test)]
    fn resolve(
        &self,
        dependencies: Vec<DeclaredDependency>,
    ) -> Result<Vec<ResolvedArtifact>, ResolveError> {
        self.resolve_with_dependency_management(dependencies, Vec::new())
    }

    fn resolve_with_dependency_management(
        &self,
        dependencies: Vec<DeclaredDependency>,
        dependency_management: Vec<DeclaredManagedDependency>,
    ) -> Result<Vec<ResolvedArtifact>, ResolveError> {
        let root_dependency_management =
            self.resolve_declared_dependency_management(dependency_management)?;
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
                repositories: self.repositories.clone(),
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
                let Some(fetched) = fetched_sources.remove(&identity) else {
                    return Err(ResolveError::Internal(format!(
                        "parallel fetch did not return source for {identity:?}"
                    )));
                };

                selected.insert(
                    identity.clone(),
                    ResolvedArtifact {
                        artifact: fetched.artifact.clone(),
                        requested_version: fetched.requested_version.clone(),
                        scope: item.scope,
                        depth: item.depth,
                        pom_path: fetched.artifact.pom_path(&self.local_repo),
                        artifact_path: fetched.artifact.artifact_path(&self.local_repo),
                        source: fetched.source,
                    },
                );
                selected_exclusions.insert(identity, item.exclusions.clone());

                let version_key = fetched.artifact.to_string();
                if !visited_versions.insert(version_key) {
                    continue;
                }

                let pom = self
                    .effective_pom(&fetched.artifact.coordinate, &item.repositories)
                    .map_err(|source| {
                        ResolveError::with_dependency_path(item.path.clone(), source)
                    })?;
                let property_context = pom.property_context();

                let mut active_repos = item.repositories.clone();
                let mut pom_repos = pom.repositories.clone();
                self.settings.apply_mirrors(&mut pom_repos);
                for repo in pom_repos {
                    if let Some(existing) = active_repos.iter_mut().find(|r| r.name == repo.name) {
                        *existing = repo;
                    } else {
                        active_repos.push(repo);
                    }
                }

                for dependency in pom.dependencies {
                    let Some(dependency_scope) = dependency.graph_scope() else {
                        continue;
                    };
                    if !dependency_scope.is_runtime_graph() {
                        continue;
                    }

                    let mut combined_dependency_management = pom.dependency_management.clone();
                    combined_dependency_management.extend(root_dependency_management.clone());

                    let Some(mut resolved_dependency) = dependency
                        .resolve(
                            &property_context,
                            &item.artifact.coordinate.to_string(),
                            &combined_dependency_management,
                        )
                        .map_err(|source| {
                            ResolveError::with_dependency_path(item.path.clone(), source.into())
                        })?
                    else {
                        continue;
                    };
                    if let Some(managed) =
                        root_dependency_management.get(&resolved_dependency.artifact.identity())
                        && let Some(version) = &managed.version
                    {
                        resolved_dependency.artifact.coordinate.version = version.clone();
                    }
                    let artifact = self
                        .resolve_artifact_version(&resolved_dependency.artifact, &active_repos)
                        .map(|resolved| resolved.artifact)
                        .map_err(|source| {
                            ResolveError::with_dependency_path(item.path.clone(), source)
                        })?;
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
                        repositories: active_repos.clone(),
                    });
                }
            }
        }

        Ok(selected.into_values().collect())
    }

    fn resolve_declared_dependency_management(
        &self,
        dependencies: Vec<DeclaredManagedDependency>,
    ) -> Result<BTreeMap<ArtifactIdentity, ManagedDependency>, ResolveError> {
        let mut managed = BTreeMap::new();
        for dependency in dependencies {
            match dependency.scope {
                ManagedDependencyScope::Import => {
                    let bom = self.effective_pom_inner(
                        &dependency.artifact.coordinate,
                        &self.repositories,
                        &mut Vec::new(),
                    )?;
                    managed.extend(bom.dependency_management);
                }
                ManagedDependencyScope::None | ManagedDependencyScope::Graph(_) => {
                    managed.insert(
                        dependency.artifact.identity(),
                        ManagedDependency {
                            version: Some(dependency.artifact.coordinate.version),
                            scope: match dependency.scope {
                                ManagedDependencyScope::Graph(scope) => Some(scope),
                                ManagedDependencyScope::None | ManagedDependencyScope::Import => {
                                    None
                                }
                            },
                            exclusions: dependency.exclusions,
                        },
                    );
                }
            }
        }
        Ok(managed)
    }

    fn ensure_artifacts_parallel(
        &self,
        items: &[QueuedDependency],
    ) -> Result<BTreeMap<ArtifactIdentity, FetchedArtifact>, ResolveError> {
        let mut fetched = BTreeMap::new();
        let parallelism = thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(4)
            .max(1);

        let num_threads = parallelism.min(items.len()).max(1);
        let queue = Mutex::new(items.iter().collect::<VecDeque<_>>());
        let (tx, rx) = std::sync::mpsc::channel();

        thread::scope(|scope| {
            for _ in 0..num_threads {
                let queue_ref = &queue;
                let tx_ref = tx.clone();
                scope.spawn(move || {
                    loop {
                        let next_item = {
                            let mut q = queue_ref.lock().expect("queue poisoned");
                            q.pop_front()
                        };
                        let Some(item) = next_item else {
                            break;
                        };
                        let result = self
                            .ensure_artifact(&item.artifact, &item.repositories)
                            .map(|fetched| (item.artifact.identity(), fetched))
                            .map_err(|source| {
                                ResolveError::with_dependency_path(item.path.clone(), source)
                            });
                        if tx_ref.send(result).is_err() {
                            break;
                        }
                    }
                });
            }

            drop(tx);

            while let Ok(result) = rx.recv() {
                let (identity, fetched_artifact) = result?;
                fetched.insert(identity, fetched_artifact);
            }

            Ok::<(), ResolveError>(())
        })?;

        Ok(fetched)
    }

    fn effective_pom(
        &self,
        coordinate: &Coordinate,
        repositories: &[Repository],
    ) -> Result<EffectivePom, ResolveError> {
        self.effective_pom_inner(coordinate, repositories, &mut Vec::new())
    }

    fn effective_pom_inner(
        &self,
        coordinate: &Coordinate,
        repositories: &[Repository],
        stack: &mut Vec<String>,
    ) -> Result<EffectivePom, ResolveError> {
        let key = coordinate.to_string();
        if let Some(cached) = self.pom_cache.lock().expect("pom cache poisoned").get(&key) {
            return Ok(cached.clone());
        }

        if let Some(cycle_start) = stack.iter().position(|seen| seen == &key) {
            let mut cycle = stack[cycle_start..].to_vec();
            cycle.push(key);
            return Err(ResolveError::PomCycle(cycle.join(" -> ")));
        }

        stack.push(key.clone());
        let pom_path = self.ensure_pom(coordinate, repositories)?;
        let raw = Pom::read(&pom_path)?;
        let parent = if let Some(parent) = raw.parent_coordinate(&key)? {
            match self.effective_local_parent(&raw, &parent, &pom_path, repositories, stack)? {
                Some(parent) => Some(parent),
                None => Some(self.effective_pom_inner(&parent, repositories, stack)?),
            }
        } else {
            None
        };

        let active_raw = raw.active_model(&self.profile_activation);
        let mut effective = active_raw.merge_with_parent_with_context(parent, &self.profile_activation);
        let properties = effective.property_context();
        for dependency in active_raw.dependency_management_entries() {
            if dependency.is_bom_import() {
                let Some(bom) = dependency.coordinate(&properties, &key)? else {
                    continue;
                };
                let bom = self.effective_pom_inner(&bom, repositories, stack)?;
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
        self.pom_cache
            .lock()
            .expect("pom cache poisoned")
            .insert(key, effective.clone());
        Ok(effective)
    }

    fn effective_local_parent(
        &self,
        child: &Pom,
        parent_coordinate: &Coordinate,
        child_pom_path: &Path,
        repositories: &[Repository],
        stack: &mut Vec<String>,
    ) -> Result<Option<EffectivePom>, ResolveError> {
        let Some(parent_path) = child.relative_parent_path() else {
            return Ok(None);
        };

        let Some(base_dir) = child_pom_path.parent() else {
            return Ok(None);
        };

        let candidate_path = match parent_path {
            Some(path) if path.as_os_str().is_empty() => return Ok(None),
            Some(path) => base_dir.join(path),
            None => base_dir.join("..").join("pom.xml"),
        };

        if !candidate_path.exists() {
            return Ok(None);
        }

        let parent = self.effective_pom_from_path(&candidate_path, repositories, stack)?;
        let matches_identity = parent.group_id.as_deref() == Some(parent_coordinate.group.as_str())
            && parent.artifact_id.as_deref() == Some(parent_coordinate.artifact.as_str());
        let matches_version = parent.version.as_deref().is_some_and(|version| {
            version == parent_coordinate.version
                || VersionRange::parse(&parent_coordinate.version)
                    .is_ok_and(|range| range.contains(version))
        });

        if matches_identity && matches_version {
            Ok(Some(parent))
        } else {
            Ok(None)
        }
    }

    fn effective_pom_from_path(
        &self,
        path: &Path,
        repositories: &[Repository],
        stack: &mut Vec<String>,
    ) -> Result<EffectivePom, ResolveError> {
        // canonicalize() resolves symlinks. If two dependencies reach this path via different symlinks,
        // the cache still hits the canonical path, though cache misses could occur if canonicalize fails.
        let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if let Some(cached) = self
            .path_pom_cache
            .lock()
            .expect("path pom cache poisoned")
            .get(&canonical_path)
        {
            return Ok(cached.clone());
        }

        let raw = Pom::read(path)?;
        let key = path.display().to_string();
        if stack.contains(&key) {
            return Err(ResolveError::PomCycle(key));
        }
        stack.push(key.clone());
        let parent = if let Some(parent) = raw.parent_coordinate(&key)? {
            match self.effective_local_parent(&raw, &parent, path, repositories, stack)? {
                Some(parent) => Some(parent),
                None => Some(self.effective_pom_inner(&parent, repositories, stack)?),
            }
        } else {
            None
        };

        let mut effective = raw.merge_with_parent_with_context(parent, &self.profile_activation);
        let properties = effective.property_context();
        for dependency in raw.dependency_management_entries() {
            if dependency.is_bom_import() {
                let Some(bom) = dependency.coordinate(&properties, &key)? else {
                    continue;
                };
                let bom = self.effective_pom_inner(&bom, repositories, stack)?;
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
        self.path_pom_cache
            .lock()
            .expect("path pom cache poisoned")
            .insert(canonical_path, effective.clone());
        Ok(effective)
    }

    fn ensure_artifact(
        &self,
        artifact: &ArtifactCoordinate,
        repositories: &[Repository],
    ) -> Result<FetchedArtifact, ResolveError> {
        let resolved = self.resolve_artifact_version(artifact, repositories)?;
        let artifact = resolved.artifact;
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

                self.download_artifact(
                    &artifact,
                    resolved.requested_version.as_deref(),
                    repositories,
                    &pom_path,
                    &artifact_path,
                    descriptor_only,
                )?
            };

        Ok(FetchedArtifact {
            artifact,
            requested_version: resolved.requested_version,
            source,
        })
    }

    fn resolve_artifact_version(
        &self,
        artifact: &ArtifactCoordinate,
        repositories: &[Repository],
    ) -> Result<ResolvedArtifactVersion, ResolveError> {
        self.resolve_coordinate_version(
            &artifact.coordinate,
            repositories,
            artifact.artifact_type,
            artifact.classifier.as_deref(),
        )
        .map(|resolved| resolved.artifact(artifact.artifact_type, artifact.classifier.clone()))
    }

    fn resolve_coordinate_version(
        &self,
        coordinate: &Coordinate,
        repositories: &[Repository],
        artifact_type: ArtifactType,
        classifier: Option<&str>,
    ) -> Result<ResolvedVersion, ResolveError> {
        if VersionRange::is_range_spec(&coordinate.version) {
            let range = VersionRange::parse(&coordinate.version)
                .map_err(|error| ResolveError::Pom(error.to_string()))?;
            let versions = self.metadata_versions(coordinate, repositories)?;
            let Some(version) = range.highest_matching(versions.iter().map(String::as_str)) else {
                return Err(ResolveError::VersionRangeNotFound {
                    artifact: coordinate.to_string(),
                    range: coordinate.version.clone(),
                });
            };
            return Ok(ResolvedVersion {
                coordinate: Coordinate::new(&coordinate.group, &coordinate.artifact, &version),
                requested_version: Some(coordinate.version.clone()),
            });
        }

        if coordinate.is_snapshot() {
            let version =
                self.snapshot_version(coordinate, repositories, artifact_type, classifier)?;
            if version != coordinate.version {
                return Ok(ResolvedVersion {
                    coordinate: Coordinate::new(&coordinate.group, &coordinate.artifact, &version),
                    requested_version: Some(coordinate.version.clone()),
                });
            }
        }

        Ok(ResolvedVersion {
            coordinate: coordinate.clone(),
            requested_version: None,
        })
    }

    fn metadata_versions(
        &self,
        coordinate: &Coordinate,
        repositories: &[Repository],
    ) -> Result<Vec<String>, ResolveError> {
        let mut versions = Vec::new();
        if let Ok(metadata) = MavenMetadata::read(&coordinate.metadata_path(&self.local_repo)) {
            versions.extend(metadata.versioning.latest);
            versions.extend(metadata.versioning.release);
            versions.extend(metadata.versioning.versions.versions);
        }

        if self.offline {
            return Ok(versions);
        }

        for repository in repositories {
            let metadata_path =
                coordinate.repository_metadata_path(&self.local_repo, &repository.name);
            if self.refresh || !metadata_path.exists() {
                let _ = self.download_metadata(
                    &repository.metadata_url(coordinate),
                    &metadata_path,
                    repository.policy_for(coordinate),
                );
            }
            if let Ok(metadata) = MavenMetadata::read(&metadata_path) {
                versions.extend(metadata.versioning.latest);
                versions.extend(metadata.versioning.release);
                versions.extend(metadata.versioning.versions.versions.into_iter().filter(
                    |version| {
                        repository.accepts(&Coordinate::new(
                            &coordinate.group,
                            &coordinate.artifact,
                            version,
                        ))
                    },
                ));
            }
        }

        versions.sort();
        versions.dedup();
        Ok(versions)
    }

    fn snapshot_version(
        &self,
        coordinate: &Coordinate,
        repositories: &[Repository],
        artifact_type: ArtifactType,
        classifier: Option<&str>,
    ) -> Result<String, ResolveError> {
        let extension = artifact_type.extension();
        let classifier = classifier.unwrap_or("");
        let mut selected = None::<(String, String)>;

        if let Ok(metadata) = MavenMetadata::read(
            &coordinate
                .local_dir(&self.local_repo)
                .join("maven-metadata.xml"),
        ) && let Some(candidate) =
            snapshot_candidate(&metadata, coordinate, extension, classifier)
        {
            selected = Some(candidate);
        }

        for repository in repositories {
            if !repository.accepts(coordinate) {
                continue;
            }

            let metadata_path =
                coordinate.snapshot_metadata_path(&self.local_repo, &repository.name);
            if !self.offline && (self.refresh || !metadata_path.exists()) {
                let _ = self.download_metadata(
                    &repository.snapshot_metadata_url(coordinate),
                    &metadata_path,
                    repository.policy_for(coordinate),
                );
            }

            let Ok(metadata) = MavenMetadata::read(&metadata_path) else {
                continue;
            };
            let candidate = snapshot_candidate(&metadata, coordinate, extension, classifier);

            if let Some(candidate) = candidate
                && selected
                    .as_ref()
                    // fixed-width yyyyMMddHHmmss format ensures lexicographic order == chronological
                    .is_none_or(|(updated, _)| candidate.0 > *updated)
            {
                selected = Some(candidate);
            }
        }

        Ok(selected
            .map(|(_, version)| version)
            .unwrap_or_else(|| coordinate.version.clone()))
    }

    fn ensure_pom(
        &self,
        coordinate: &Coordinate,
        repositories: &[Repository],
    ) -> Result<PathBuf, ResolveError> {
        let resolved =
            self.resolve_coordinate_version(coordinate, repositories, ArtifactType::Pom, None)?;
        let coordinate = resolved.coordinate;
        let pom_path = coordinate.pom_path(&self.local_repo);

        if !self.refresh && pom_path.exists() {
            "local".to_string()
        } else {
            if self.offline {
                return Err(ResolveError::OfflineMissing(coordinate.to_string()));
            }

            self.download_pom(
                &coordinate,
                resolved.requested_version.as_deref(),
                repositories,
                &pom_path,
            )?
        };

        Ok(pom_path)
    }

    fn download_artifact(
        &self,
        artifact: &ArtifactCoordinate,
        requested_version: Option<&str>,
        repositories: &[Repository],
        pom_path: &Path,
        artifact_path: &Path,
        descriptor_only: bool,
    ) -> Result<String, ResolveError> {
        fs::create_dir_all(artifact.local_dir(&self.local_repo))?;
        let mut not_found = Vec::new();
        let needs_pom = self.refresh || !pom_path.exists();

        for repository in repositories {
            if !repository.accepts(&artifact.coordinate) {
                continue;
            }

            let result: Result<String, ResolveError> = (|| {
                let policy = repository.policy_for(&artifact.coordinate);
                if needs_pom {
                    self.download(
                        &pom_url(repository, &artifact.coordinate, requested_version),
                        pom_path,
                        policy,
                    )?;
                }
                if !descriptor_only {
                    self.download(
                        &artifact_url(repository, artifact, requested_version),
                        artifact_path,
                        policy,
                    )?;
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
        requested_version: Option<&str>,
        repositories: &[Repository],
        pom_path: &Path,
    ) -> Result<String, ResolveError> {
        fs::create_dir_all(coordinate.local_dir(&self.local_repo))?;
        let mut not_found = Vec::new();

        for repository in repositories {
            if !repository.accepts(coordinate) {
                continue;
            }

            let result = self
                .download(
                    &pom_url(repository, coordinate, requested_version),
                    pom_path,
                    repository.policy_for(coordinate),
                )
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

    fn download_metadata(
        &self,
        url: &str,
        destination: &Path,
        policy: &RepositoryPolicy,
    ) -> Result<(), ResolveError> {
        self.download(url, destination, policy)
    }

    fn download(
        &self,
        url: &str,
        destination: &Path,
        policy: &RepositoryPolicy,
    ) -> Result<(), ResolveError> {
        let bytes = self.download_bytes(url)?;
        let checksum_bytes = if matches!(policy.checksum_policy, ChecksumPolicy::Ignore) {
            None
        } else {
            let checksum_url = format!("{url}.sha1");
            match self.download_bytes(&checksum_url) {
                Ok(checksum_bytes) => {
                    match verify_sha1_checksum(url, &bytes, &checksum_url, &checksum_bytes) {
                        Ok(()) => Some(checksum_bytes),
                        Err(error) if matches!(policy.checksum_policy, ChecksumPolicy::Warn) => {
                            self.warn(error.to_string());
                            None
                        }
                        Err(error) => return Err(error),
                    }
                }
                Err(error) if matches!(policy.checksum_policy, ChecksumPolicy::Warn) => {
                    self.warn(error.to_string());
                    None
                }
                Err(error) => return Err(error),
            }
        };

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp_ext = format!(
            "tmp.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let temp_dest = destination.with_extension(&temp_ext);
        fs::write(&temp_dest, &bytes)?;
        fs::rename(&temp_dest, destination)?;

        if let Some(checksum_bytes) = checksum_bytes {
            let checksum_dest = checksum_path(destination);
            let temp_checksum = checksum_dest.with_extension(&temp_ext);
            fs::write(&temp_checksum, checksum_bytes)?;
            fs::rename(&temp_checksum, &checksum_dest)?;
        }
        Ok(())
    }

    fn warn(&self, message: String) {
        self.warnings
            .lock()
            .expect("warnings list poisoned")
            .push(message);
    }

    fn download_bytes(&self, url: &str) -> Result<Vec<u8>, ResolveError> {
        let response = self
            .client
            .get(url)
            .send()
            .map_err(|source| ResolveError::Download {
                url: url.to_string(),
                source,
            })?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(ResolveError::AuthenticationRequired {
                url: url.to_string(),
                status: status.as_u16(),
            });
        }

        let response = response
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

    Ok(faster_hex::hex_string(&hasher.finalize()))
}

fn sha1_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    faster_hex::hex_string(&hasher.finalize())
}

fn checksum_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(".sha1");
    PathBuf::from(value)
}

fn pom_url(
    repository: &Repository,
    coordinate: &Coordinate,
    requested_version: Option<&str>,
) -> String {
    let directory_version = remote_directory_version(&coordinate.version, requested_version);
    format!(
        "{}/{}/{}/{}/{}-{}.pom",
        repository.url,
        coordinate.group_path(),
        coordinate.artifact,
        directory_version,
        coordinate.artifact,
        coordinate.version
    )
}

fn artifact_url(
    repository: &Repository,
    artifact: &ArtifactCoordinate,
    requested_version: Option<&str>,
) -> String {
    if artifact.artifact_type == ArtifactType::Pom && artifact.classifier.is_none() {
        return pom_url(repository, &artifact.coordinate, requested_version);
    }

    let directory_version =
        remote_directory_version(&artifact.coordinate.version, requested_version);
    format!(
        "{}/{}/{}/{}/{}-{}.{}",
        repository.url,
        artifact.coordinate.group_path(),
        artifact.coordinate.artifact,
        directory_version,
        artifact.coordinate.artifact,
        artifact.version_suffix(),
        artifact.artifact_type.extension()
    )
}

fn remote_directory_version<'a>(
    resolved_version: &'a str,
    requested_version: Option<&'a str>,
) -> &'a str {
    requested_version
        .filter(|version| version.ends_with("-SNAPSHOT"))
        .unwrap_or(resolved_version)
}

fn snapshot_candidate(
    metadata: &MavenMetadata,
    coordinate: &Coordinate,
    extension: &str,
    classifier: &str,
) -> Option<(String, String)> {
    let versioning = &metadata.versioning;
    versioning
        .snapshot_versions
        .snapshot_versions
        .iter()
        .find(|snapshot| {
            snapshot.extension.as_deref() == Some(extension)
                && snapshot.classifier.as_deref().unwrap_or("") == classifier
        })
        .and_then(|snapshot| {
            snapshot
                .value
                .clone()
                .map(|value| (snapshot.updated.clone().unwrap_or_default(), value))
        })
        .or_else(|| {
            let snapshot = versioning.snapshot.as_ref()?;
            if snapshot.local_copy {
                return Some((
                    versioning.last_updated.clone().unwrap_or_default(),
                    coordinate.version.clone(),
                ));
            }
            let timestamp = snapshot.timestamp.as_ref()?;
            let build = snapshot.build_number?;
            let base = coordinate.version.strip_suffix("-SNAPSHOT")?;
            Some((
                versioning.last_updated.clone().unwrap_or_default(),
                format!("{base}-{timestamp}-{build}"),
            ))
        })
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
    #[error("no version matching range `{range}` was found for `{artifact}`")]
    VersionRangeNotFound { artifact: String, range: String },
    #[error("cyclic POM inheritance or BOM import path `{0}`")]
    PomCycle(String),
    #[error("{0}")]
    Pom(String),
    #[error("failed to create HTTP client: {0}")]
    Http(#[from] reqwest::Error),
    #[error("failed to download `{url}`: {source}")]
    Download { url: String, source: reqwest::Error },
    #[error(
        "repository at `{url}` requires authentication (HTTP {status}). \
            Angra does not yet support authenticated repositories. \
            Consider configuring a mirror or using a public repository."
    )]
    AuthenticationRequired { url: String, status: u16 },
    #[error("checksum mismatch for `{url}`: expected `{expected}`, found `{actual}`")]
    ChecksumMismatch {
        url: String,
        expected: String,
        actual: String,
    },
    #[error("internal resolver error: {0}")]
    Internal(String),
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

    #[cfg(test)]
    fn is_auth_required(&self) -> bool {
        matches!(self, Self::AuthenticationRequired { .. })
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs};

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

    fn write_metadata(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
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

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
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

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
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

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
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

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
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

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
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

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
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

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
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
    fn applies_manifest_dependency_management_across_graph() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let root = Coordinate::new("com.example", "root", "1.0.0");
        let child = Coordinate::new("com.example", "child", "1.0.0");
        let leaf_1 = Coordinate::new("com.example", "leaf", "1.0.0");
        let leaf_2 = Coordinate::new("com.example", "leaf", "2.0.0");

        write_artifact(
            &repo,
            &root,
            r#"
            <project>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>child</artifactId>
                  <version>1.0.0</version>
                </dependency>
              </dependencies>
            </project>
            "#,
        );
        write_artifact(
            &repo,
            &child,
            r#"
            <project>
              <dependencyManagement>
                <dependencies>
                  <dependency>
                    <groupId>com.example</groupId>
                    <artifactId>leaf</artifactId>
                    <version>1.0.0</version>
                  </dependency>
                </dependencies>
              </dependencyManagement>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>leaf</artifactId>
                  <version>1.0.0</version>
                </dependency>
              </dependencies>
            </project>
            "#,
        );
        write_artifact(&repo, &leaf_1, "<project/>");
        write_artifact(&repo, &leaf_2, "<project/>");

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
        let resolved = resolver
            .resolve_with_dependency_management(
                vec![DeclaredDependency {
                    alias: "root".to_string(),
                    artifact: ArtifactCoordinate::jar(root),
                    scope: Scope::Compile,
                    exclusions: Vec::new(),
                }],
                vec![DeclaredManagedDependency {
                    alias: "leaf".to_string(),
                    artifact: ArtifactCoordinate::jar(leaf_2),
                    scope: ManagedDependencyScope::None,
                    exclusions: Vec::new(),
                }],
            )
            .unwrap();

        let coordinates = resolved
            .iter()
            .map(|artifact| artifact.artifact.coordinate.to_string())
            .collect::<Vec<_>>();

        assert!(coordinates.contains(&"com.example:leaf:2.0.0".to_string()));
        assert!(!coordinates.contains(&"com.example:leaf:1.0.0".to_string()));
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

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
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

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
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

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
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

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
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

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
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

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
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

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
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

    #[test]
    fn propagates_lexically_scoped_repositories() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");

        let root_a = Coordinate::new("com.example", "root-a", "1.0.0");
        let child_a = Coordinate::new("com.example", "child-a", "1.0.0");
        let _grandchild_a = Coordinate::new("com.example", "grandchild-a", "1.0.0");

        let root_b = Coordinate::new("com.example", "root-b", "1.0.0");
        let child_b = Coordinate::new("com.example", "child-b", "1.0.0");
        let _grandchild_b = Coordinate::new("com.example", "grandchild-b", "1.0.0");

        // Write root-a POM with custom-repo-a, which depends on child-a
        write_artifact(
            &repo,
            &root_a,
            r#"
            <project>
              <repositories>
                <repository>
                  <id>custom-repo-a</id>
                  <url>https://repo.maven.apache.org/maven2</url>
                </repository>
              </repositories>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>child-a</artifactId>
                  <version>1.0.0</version>
                </dependency>
              </dependencies>
            </project>
            "#,
        );
        // Write child-a POM (local) which depends on grandchild-a (not in local repo)
        write_artifact(
            &repo,
            &child_a,
            r#"
            <project>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>grandchild-a</artifactId>
                  <version>1.0.0</version>
                </dependency>
              </dependencies>
            </project>
            "#,
        );

        // Write root-b POM with custom-repo-b, which depends on child-b
        write_artifact(
            &repo,
            &root_b,
            r#"
            <project>
              <repositories>
                <repository>
                  <id>custom-repo-b</id>
                  <url>https://repo.clojars.org</url>
                </repository>
              </repositories>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>child-b</artifactId>
                  <version>1.0.0</version>
                </dependency>
              </dependencies>
            </project>
            "#,
        );
        // Write child-b POM (local) which depends on grandchild-b (not in local repo)
        write_artifact(
            &repo,
            &child_b,
            r#"
            <project>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>grandchild-b</artifactId>
                  <version>1.0.0</version>
                </dependency>
              </dependencies>
            </project>
            "#,
        );

        // Run resolution for root-a (offline = false, so it tries custom-repo-a)
        let resolver = Resolver::new(
            repo.clone(),
            vec![Repository::maven_central()],
            MavenSettings::default(),
            false,
            false,
        )
        .unwrap();

        let err_a = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root-a".to_string(),
                artifact: ArtifactCoordinate::jar(root_a),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap_err();

        // The grandchild-a resolution should fail to download and report searched repositories
        let cause_a = err_a.root_cause();
        if let ResolveError::ArtifactNotFound { repositories, .. } = cause_a {
            assert!(repositories.contains(&"maven-central".to_string()));
            assert!(repositories.contains(&"custom-repo-a".to_string()));
            assert!(!repositories.contains(&"custom-repo-b".to_string()));
        } else {
            panic!("Expected ArtifactNotFound error, got {:?}", err_a);
        }

        // Run resolution for root-b (offline = false, so it tries custom-repo-b)
        let err_b = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root-b".to_string(),
                artifact: ArtifactCoordinate::jar(root_b),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap_err();

        let cause_b = err_b.root_cause();
        if let ResolveError::ArtifactNotFound { repositories, .. } = cause_b {
            assert!(repositories.contains(&"maven-central".to_string()));
            assert!(repositories.contains(&"custom-repo-b".to_string()));
            assert!(!repositories.contains(&"custom-repo-a".to_string()));
        } else {
            panic!("Expected ArtifactNotFound error, got {:?}", err_b);
        }
    }

    #[test]
    fn skips_release_only_repo_for_snapshot_artifact() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");

        // SNAPSHOT artifact — not in local repo
        let snapshot = Coordinate::new("com.example", "lib", "1.0.0-SNAPSHOT");

        // A release-only repo (like Maven Central) should be skipped entirely.
        // A snapshot-enabled repo should be tried but fail (offline).
        let release_only =
            Repository::with_policies("release-only", "https://releases.example.com", true, false);
        let snapshot_enabled =
            Repository::with_policies("snapshot-repo", "https://snapshots.example.com", true, true);

        let resolver = Resolver::new(
            repo,
            vec![release_only, snapshot_enabled],
            MavenSettings::default(),
            true, // offline
            false,
        )
        .unwrap();

        let error = resolver
            .resolve(vec![DeclaredDependency {
                alias: "lib".to_string(),
                artifact: ArtifactCoordinate::jar(snapshot),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap_err();

        // The error should be OfflineMissing since there's a valid snapshot repo
        // but we're offline and the artifact isn't in the local repo.
        assert!(matches!(
            error.root_cause(),
            ResolveError::OfflineMissing(_)
        ));
    }

    #[test]
    fn skips_snapshot_only_repo_for_release_artifact() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");

        let release = Coordinate::new("com.example", "lib", "1.0.0");
        write_artifact(&repo, &release, "<project/>");

        // Only a snapshot-only repo — it should be skipped for releases.
        // But since the artifact is already in local repo, it resolves from local.
        let snapshot_only = Repository::with_policies(
            "snapshot-only",
            "https://snapshots.example.com",
            false,
            true,
        );

        let resolver = Resolver::new(
            repo,
            vec![snapshot_only],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();

        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "lib".to_string(),
                artifact: ArtifactCoordinate::jar(release),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].source, "local");
    }

    #[test]
    fn auth_required_error_message_is_actionable() {
        let error = ResolveError::AuthenticationRequired {
            url: "https://repo.example.com/com/example/lib/1.0.0/lib-1.0.0.jar".to_string(),
            status: 401,
        };

        let message = error.to_string();
        assert!(message.contains("401"));
        assert!(message.contains("authentication"));
        assert!(message.contains("does not yet support"));
        assert!(message.contains("mirror"));
        assert!(error.is_auth_required());
        assert!(!error.is_not_found());
    }

    #[test]
    fn auth_required_403_error() {
        let error = ResolveError::AuthenticationRequired {
            url: "https://repo.example.com/artifact.jar".to_string(),
            status: 403,
        };

        let message = error.to_string();
        assert!(message.contains("403"));
        assert!(message.contains("does not yet support"));
    }

    #[test]
    fn resolves_direct_version_range_from_local_metadata() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let requested = Coordinate::new("com.example", "lib", "[1.0,2.0)");
        let selected = Coordinate::new("com.example", "lib", "1.5.0");

        write_metadata(
            &requested.metadata_path(&repo),
            r#"
            <metadata>
              <versioning>
                <versions>
                  <version>1.0.0</version>
                  <version>1.5.0</version>
                  <version>2.0.0</version>
                </versions>
              </versioning>
            </metadata>
            "#,
        );
        write_artifact(&repo, &selected, "<project/>");

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "lib".to_string(),
                artifact: ArtifactCoordinate::jar(requested),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        assert_eq!(resolved[0].artifact.coordinate.version, "1.5.0");
        assert_eq!(resolved[0].requested_version.as_deref(), Some("[1.0,2.0)"));
    }

    #[test]
    fn resolves_snapshot_timestamp_from_metadata() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let requested = Coordinate::new("com.example", "lib", "1.0-SNAPSHOT");
        let timestamped = Coordinate::new("com.example", "lib", "1.0-20240501.120000-3");

        write_metadata(
            &requested.local_dir(&repo).join("maven-metadata.xml"),
            r#"
            <metadata>
              <versioning>
                <snapshotVersions>
                  <snapshotVersion>
                    <extension>jar</extension>
                    <value>1.0-20240501.120000-3</value>
                    <updated>20240501120000</updated>
                  </snapshotVersion>
                </snapshotVersions>
              </versioning>
            </metadata>
            "#,
        );
        write_artifact(&repo, &timestamped, "<project/>");

        let resolver = Resolver::new(
            repo,
            vec![Repository::with_policies(
                "snapshots",
                "https://repo.example.com",
                false,
                true,
            )],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "lib".to_string(),
                artifact: ArtifactCoordinate::jar(requested),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        assert_eq!(
            resolved[0].artifact.coordinate.version,
            "1.0-20240501.120000-3"
        );
        assert_eq!(
            resolved[0].requested_version.as_deref(),
            Some("1.0-SNAPSHOT")
        );
    }

    #[test]
    fn injects_active_pom_profile_dependencies() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let root = Coordinate::new("com.example", "root", "1.0.0");
        let child = Coordinate::new("com.example", "profile-child", "1.0.0");

        write_artifact(
            &repo,
            &root,
            r#"
            <project>
              <profiles>
                <profile>
                  <id>dev</id>
                  <activation>
                    <property>
                      <name>environment</name>
                      <value>dev</value>
                    </property>
                  </activation>
                  <dependencies>
                    <dependency>
                      <groupId>com.example</groupId>
                      <artifactId>profile-child</artifactId>
                      <version>1.0.0</version>
                    </dependency>
                  </dependencies>
                </profile>
              </profiles>
            </project>
            "#,
        );
        write_artifact(&repo, &child, "<project/>");

        let activation = ProfileActivationContext::new(
            Vec::<String>::new(),
            Vec::<String>::new(),
            BTreeMap::from([("environment".to_string(), "dev".to_string())]),
            None,
            dir.path().to_path_buf(),
        );
        let resolver = Resolver::new_with_activation(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            activation,
            true,
            false,
        )
        .unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                artifact: ArtifactCoordinate::jar(root),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        assert!(
            resolved
                .iter()
                .any(|artifact| artifact.artifact.coordinate.artifact == "profile-child")
        );
    }

    #[test]
    fn uses_matching_local_parent_relative_path() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("m2");
        let root = Coordinate::new("com.example", "root", "1.0.0");
        let child = Coordinate::new("com.example", "child", "2.0.0");
        let root_dir = root.local_dir(&repo);

        fs::create_dir_all(&root_dir).unwrap();
        fs::write(
            root_dir.join("parent.xml"),
            r#"
            <project>
              <groupId>com.example</groupId>
              <artifactId>parent</artifactId>
              <version>1.0.0</version>
              <properties>
                <child.version>2.0.0</child.version>
              </properties>
            </project>
            "#,
        )
        .unwrap();
        fs::write(root.jar_path(&repo), "jar for root").unwrap();
        fs::write(
            root.pom_path(&repo),
            r#"
            <project>
              <parent>
                <groupId>com.example</groupId>
                <artifactId>parent</artifactId>
                <version>1.0.0</version>
                <relativePath>parent.xml</relativePath>
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
        )
        .unwrap();
        write_artifact(&repo, &child, "<project/>");

        let resolver = Resolver::new(
            repo,
            vec![Repository::maven_central()],
            MavenSettings::default(),
            true,
            false,
        )
        .unwrap();
        let resolved = resolver
            .resolve(vec![DeclaredDependency {
                alias: "root".to_string(),
                artifact: ArtifactCoordinate::jar(root),
                scope: Scope::Compile,
                exclusions: Vec::new(),
            }])
            .unwrap();

        assert!(
            resolved
                .iter()
                .any(|artifact| artifact.artifact.coordinate.to_string()
                    == "com.example:child:2.0.0")
        );
    }
}
