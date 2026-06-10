use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use quick_xml::{Reader, escape::unescape, events::Event};

use crate::maven::{
    ArtifactCoordinate, ArtifactIdentity, ArtifactType, ChecksumPolicy, Coordinate, Repository,
    RepositoryPolicy, Scope,
};

#[derive(Debug, Clone)]
pub(crate) struct EffectivePom {
    pub group_id: Option<String>,
    pub artifact_id: Option<String>,
    pub version: Option<String>,
    pub properties: BTreeMap<String, String>,
    pub dependency_management: BTreeMap<ArtifactIdentity, ManagedDependency>,
    pub dependencies: Vec<PomDependency>,
    pub repositories: Vec<Repository>,
}

impl EffectivePom {
    pub(crate) fn property_context(&self) -> PomPropertyContext {
        PomPropertyContext::new(
            self.properties.clone(),
            self.group_id.clone(),
            self.artifact_id.clone(),
            self.version.clone(),
        )
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ManagedDependency {
    pub version: Option<String>,
    pub scope: Option<Scope>,
    pub exclusions: Vec<Coordinate>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct PomRepositories {
    #[serde(rename = "repository", default)]
    pub(crate) repositories: Vec<PomRepository>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PomRepositoryPolicy {
    #[serde(default)]
    pub(crate) enabled: Option<String>,
    #[serde(default)]
    pub(crate) checksum_policy: Option<String>,
}

impl PomRepositoryPolicy {
    fn is_enabled(&self) -> bool {
        self.enabled
            .as_deref()
            .map(|v| !v.trim().eq_ignore_ascii_case("false"))
            .unwrap_or(true)
    }

    fn to_repository_policy(&self) -> RepositoryPolicy {
        RepositoryPolicy {
            enabled: self.is_enabled(),
            checksum_policy: ChecksumPolicy::parse(self.checksum_policy.as_deref()),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct PomRepository {
    pub(crate) id: Option<String>,
    #[allow(dead_code)]
    pub(crate) name: Option<String>,
    pub(crate) url: Option<String>,
    #[serde(default)]
    pub(crate) releases: Option<PomRepositoryPolicy>,
    #[serde(default)]
    pub(crate) snapshots: Option<PomRepositoryPolicy>,
}

impl PomRepository {
    pub(crate) fn resolve(
        &self,
        properties: &PomPropertyContext,
        source: &str,
    ) -> Result<Option<Repository>, PomError> {
        let Some(url) = &self.url else {
            return Ok(None);
        };
        let url = properties.interpolate(url, source)?;
        let id = match &self.id {
            Some(id) => properties.interpolate(id, source)?,
            None => url.clone(),
        };
        let releases = self
            .releases
            .as_ref()
            .map(PomRepositoryPolicy::to_repository_policy)
            .unwrap_or_default();
        let snapshots = self
            .snapshots
            .as_ref()
            .map(PomRepositoryPolicy::to_repository_policy)
            .unwrap_or_default();
        Ok(Some(Repository::with_policy_details(
            &id, &url, releases, snapshots,
        )))
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename = "project")]
#[serde(rename_all = "camelCase")]
pub(crate) struct Pom {
    group_id: Option<String>,
    artifact_id: Option<String>,
    version: Option<String>,
    packaging: Option<String>,
    parent: Option<PomParent>,
    #[serde(default)]
    modules: PomModules,
    #[serde(skip)]
    properties: BTreeMap<String, String>,
    #[serde(default)]
    dependency_management: PomDependencyManagement,
    #[serde(default)]
    dependencies: PomDependencies,
    #[serde(default)]
    repositories: PomRepositories,
    #[serde(default)]
    profiles: PomProfiles,
    #[serde(default)]
    build: Option<PomSectionPresence>,
    #[serde(default)]
    reporting: Option<PomSectionPresence>,
    #[serde(default)]
    distribution_management: Option<PomSectionPresence>,
}

impl Pom {
    pub(crate) fn read(path: &Path) -> Result<Self, PomError> {
        let raw = fs::read_to_string(path)?;
        let mut pom: Self = quick_xml::de::from_str(&raw)?;
        let (project_props, profile_props_list) = read_all_properties(&raw)?;
        pom.properties = project_props;
        for (profile, properties) in pom.profiles.profile.iter_mut().zip(profile_props_list) {
            profile.properties = properties;
        }
        Ok(pom)
    }

    pub(crate) fn property_context(&self) -> PomPropertyContext {
        PomPropertyContext::new(
            self.properties.clone(),
            self.group_id.clone(),
            self.artifact_id.clone(),
            self.version.clone(),
        )
    }

    pub(crate) fn packaging(&self) -> Option<&str> {
        self.packaging.as_deref()
    }

    pub(crate) fn modules(&self) -> &[String] {
        &self.modules.modules
    }

    pub(crate) fn has_build_section(&self) -> bool {
        self.build.is_some()
    }

    pub(crate) fn has_reporting_section(&self) -> bool {
        self.reporting.is_some()
    }

    pub(crate) fn has_distribution_management_section(&self) -> bool {
        self.distribution_management.is_some()
    }

    pub(crate) fn parent_coordinate(&self, source: &str) -> Result<Option<Coordinate>, PomError> {
        let Some(parent) = &self.parent else {
            return Ok(None);
        };

        let context = self.property_context();
        parent.coordinate(&context, source)
    }

    #[allow(dead_code)]
    pub(crate) fn effective_without_parent(&self) -> EffectivePom {
        self.effective_without_parent_with_context(&ProfileActivationContext::default())
    }

    #[allow(dead_code)]
    pub(crate) fn effective_without_parent_with_context(
        &self,
        activation: &ProfileActivationContext,
    ) -> EffectivePom {
        let model = self.active_model(activation);
        model.effective_without_parent_inner()
    }

    fn effective_without_parent_inner(&self) -> EffectivePom {
        let context = self.property_context();
        let mut repositories = Vec::new();
        for repo in &self.repositories.repositories {
            if let Ok(Some(resolved)) =
                repo.resolve(&context, self.artifact_id.as_deref().unwrap_or("unknown"))
            {
                repositories.push(resolved);
            }
        }
        EffectivePom {
            group_id: self.group_id.clone(),
            artifact_id: self.artifact_id.clone(),
            version: self.version.clone(),
            properties: self.properties.clone(),
            dependency_management: BTreeMap::new(),
            dependencies: self.dependencies.dependencies.clone(),
            repositories,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn merge_with_parent(&self, parent: Option<EffectivePom>) -> EffectivePom {
        self.merge_with_parent_with_context(parent, &ProfileActivationContext::default())
    }

    pub(crate) fn merge_with_parent_with_context(
        &self,
        parent: Option<EffectivePom>,
        activation: &ProfileActivationContext,
    ) -> EffectivePom {
        let model = self.active_model(activation);
        model.merge_with_parent_inner(parent)
    }

    fn merge_with_parent_inner(&self, parent: Option<EffectivePom>) -> EffectivePom {
        let mut effective = match parent {
            Some(parent_effective) => parent_effective,
            None => return self.effective_without_parent_inner(),
        };

        effective.group_id = self.group_id.clone().or(effective.group_id);
        effective.artifact_id = self.artifact_id.clone().or(effective.artifact_id);
        effective.version = self.version.clone().or(effective.version);

        effective.properties.extend(self.properties.clone());
        effective.dependencies = self.dependencies.dependencies.clone();

        // Merge repositories: child overrides parent by ID/name
        let context = self.property_context();
        let mut child_repos = Vec::new();
        for repo in &self.repositories.repositories {
            if let Ok(Some(resolved)) =
                repo.resolve(&context, self.artifact_id.as_deref().unwrap_or("unknown"))
            {
                child_repos.push(resolved);
            }
        }

        for repo in child_repos {
            if let Some(existing) = effective
                .repositories
                .iter_mut()
                .find(|r| r.name == repo.name)
            {
                *existing = repo;
            } else {
                effective.repositories.push(repo);
            }
        }

        effective
    }

    pub(crate) fn dependency_management_entries(&self) -> &[PomDependency] {
        &self.dependency_management.dependencies.dependencies
    }

    pub(crate) fn relative_parent_path(&self) -> Option<Option<&Path>> {
        self.parent
            .as_ref()
            .map(|parent| parent.relative_path.as_deref())
    }

    pub(crate) fn active_model(&self, activation: &ProfileActivationContext) -> Self {
        let mut model = self.clone();
        model.profiles = PomProfiles::default();

        let active_profiles = self.active_profiles(activation);
        for profile in active_profiles {
            model.properties.extend(profile.properties.clone());
            model
                .dependency_management
                .dependencies
                .dependencies
                .extend(
                    profile
                        .dependency_management
                        .dependencies
                        .dependencies
                        .clone(),
                );
            model
                .dependencies
                .dependencies
                .extend(profile.dependencies.dependencies.clone());
            model
                .repositories
                .repositories
                .extend(profile.repositories.repositories.clone());
        }

        model
    }

    fn active_profiles(&self, activation: &ProfileActivationContext) -> Vec<&PomProfile> {
        let mut active = Vec::new();
        let mut defaults = Vec::new();
        let mut has_non_default_active = false;

        for profile in &self.profiles.profile {
            let id = profile.id.as_deref().unwrap_or_default();
            if activation.inactive_profiles.contains(id) {
                continue;
            }

            if activation.active_profiles.contains(id) || profile.is_active(activation, self) {
                has_non_default_active = true;
                active.push(profile);
            } else if profile
                .activation
                .as_ref()
                .is_some_and(|activation| activation.active_by_default)
            {
                defaults.push(profile);
            }
        }

        if !has_non_default_active {
            active.extend(defaults);
        }

        active
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct PomModules {
    #[serde(rename = "module", default)]
    modules: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct PomSectionPresence {}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PomParent {
    group_id: Option<String>,
    artifact_id: Option<String>,
    version: Option<String>,
    #[allow(dead_code)]
    relative_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProfileActivationContext {
    pub(crate) active_profiles: HashSet<String>,
    pub(crate) inactive_profiles: HashSet<String>,
    pub(crate) properties: BTreeMap<String, String>,
    pub(crate) java_version: Option<String>,
    pub(crate) project_dir: Option<PathBuf>,
    pub(crate) os_name: String,
    pub(crate) os_arch: String,
    pub(crate) os_version: String,
}

impl ProfileActivationContext {
    pub(crate) fn new(
        active_profiles: impl IntoIterator<Item = String>,
        inactive_profiles: impl IntoIterator<Item = String>,
        properties: BTreeMap<String, String>,
        java_version: Option<String>,
        project_dir: PathBuf,
    ) -> Self {
        let java_version = java_version.or_else(read_java_home_version);
        let mut properties = properties;
        properties.extend(std::env::vars().map(|(name, value)| (format!("env.{name}"), value)));

        Self {
            active_profiles: active_profiles.into_iter().collect(),
            inactive_profiles: inactive_profiles.into_iter().collect(),
            properties,
            java_version,
            project_dir: Some(project_dir),
            os_name: std::env::consts::OS.to_string(),
            os_arch: std::env::consts::ARCH.to_string(),
            os_version: String::new(),
        }
    }
}

fn read_java_home_version() -> Option<String> {
    let java_home = std::env::var_os("JAVA_HOME")?;
    let release = fs::read_to_string(PathBuf::from(java_home).join("release")).ok()?;
    release.lines().find_map(|line| {
        let value = line.strip_prefix("JAVA_VERSION=")?;
        Some(value.trim_matches('"').to_string())
    })
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct PomProfiles {
    #[serde(rename = "profile", default)]
    profile: Vec<PomProfile>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PomProfile {
    id: Option<String>,
    activation: Option<PomActivation>,
    #[serde(skip)]
    properties: BTreeMap<String, String>,
    #[serde(default)]
    dependency_management: PomDependencyManagement,
    #[serde(default)]
    dependencies: PomDependencies,
    #[serde(default)]
    repositories: PomRepositories,
}

impl PomProfile {
    fn is_active(&self, context: &ProfileActivationContext, pom: &Pom) -> bool {
        let Some(activation) = &self.activation else {
            return false;
        };
        activation.is_active(context, pom)
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PomActivation {
    #[serde(default)]
    active_by_default: bool,
    jdk: Option<String>,
    os: Option<PomActivationOs>,
    property: Option<PomActivationProperty>,
    file: Option<PomActivationFile>,
}

impl PomActivation {
    fn is_active(&self, context: &ProfileActivationContext, pom: &Pom) -> bool {
        let mut configured = false;

        if let Some(jdk) = &self.jdk {
            configured = true;
            if !matches_jdk(jdk, context.java_version.as_deref().unwrap_or_default()) {
                return false;
            }
        }
        if let Some(os) = &self.os {
            configured = true;
            if !os.matches(context) {
                return false;
            }
        }
        if let Some(property) = &self.property {
            configured = true;
            if !property.matches(context, pom) {
                return false;
            }
        }
        if let Some(file) = &self.file {
            configured = true;
            if !file.matches(context) {
                return false;
            }
        }

        configured
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct PomActivationOs {
    name: Option<String>,
    family: Option<String>,
    arch: Option<String>,
    version: Option<String>,
}

impl PomActivationOs {
    fn matches(&self, context: &ProfileActivationContext) -> bool {
        let mut configured = false;
        if let Some(family) = &self.family {
            configured = true;
            if !matches_negatable(family, &os_family(&context.os_name)) {
                return false;
            }
        }
        if let Some(name) = &self.name {
            configured = true;
            if !matches_negatable(name, &context.os_name) {
                return false;
            }
        }
        if let Some(arch) = &self.arch {
            configured = true;
            if !matches_negatable(arch, &context.os_arch) {
                return false;
            }
        }
        if let Some(version) = &self.version {
            configured = true;
            if !matches_negatable(version, &context.os_version) {
                return false;
            }
        }
        configured
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct PomActivationProperty {
    name: Option<String>,
    value: Option<String>,
}

impl PomActivationProperty {
    fn matches(&self, context: &ProfileActivationContext, pom: &Pom) -> bool {
        let Some(raw_name) = &self.name else {
            return false;
        };
        let reverse_name = raw_name.starts_with('!');
        let name = raw_name.strip_prefix('!').unwrap_or(raw_name);
        if name.is_empty() {
            return false;
        }

        let value = if name == "packaging" {
            Some("jar")
        } else {
            context.properties.get(name).map(String::as_str)
        };

        let Some(expected) = &self.value else {
            let present = value.is_some_and(|value| !value.is_empty());
            return if reverse_name { !present } else { present };
        };

        let expected = interpolate_activation_value(expected, context, pom);
        let reverse_value = expected.starts_with('!');
        let expected = expected.strip_prefix('!').unwrap_or(&expected);
        let matches = value == Some(expected);
        if reverse_value { !matches } else { matches }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct PomActivationFile {
    exists: Option<String>,
    missing: Option<String>,
}

impl PomActivationFile {
    fn matches(&self, context: &ProfileActivationContext) -> bool {
        let (raw_path, missing) = match (&self.exists, &self.missing) {
            (Some(path), _) if !path.trim().is_empty() => (path, false),
            (_, Some(path)) if !path.trim().is_empty() => (path, true),
            _ => return false,
        };
        let path = interpolate_activation_file(raw_path, context);
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else if let Some(project_dir) = &context.project_dir {
            project_dir.join(path)
        } else {
            return false;
        };
        let exists = path.exists();
        if missing { !exists } else { exists }
    }
}

fn matches_negatable(expected: &str, actual: &str) -> bool {
    let reverse = expected.starts_with('!');
    let expected = expected.strip_prefix('!').unwrap_or(expected);
    let matches = actual.eq_ignore_ascii_case(expected);
    if reverse { !matches } else { matches }
}

fn os_family(os_name: &str) -> String {
    match os_name.to_ascii_lowercase().as_str() {
        "macos" | "darwin" => "mac".to_string(),
        "windows" => "windows".to_string(),
        "linux" | "freebsd" | "openbsd" | "netbsd" => "unix".to_string(),
        other => other.to_string(),
    }
}

fn matches_jdk(expected: &str, actual: &str) -> bool {
    if actual.is_empty() {
        return false;
    }
    if let Some(prefix) = expected.strip_prefix('!') {
        return !actual.starts_with(prefix);
    }
    if expected.starts_with(['[', '('])
        && let Ok(range) = crate::maven::VersionRange::parse(expected)
    {
        return range.contains(actual);
    }
    actual.starts_with(expected)
}

fn interpolate_activation_value(
    value: &str,
    context: &ProfileActivationContext,
    pom: &Pom,
) -> String {
    let mut values = context.properties.clone();
    values.extend(pom.properties.clone());
    PomPropertyContext::new(values, None, None, None)
        .interpolate(value, "profile activation")
        .unwrap_or_else(|_| value.to_string())
}

fn interpolate_activation_file(value: &str, context: &ProfileActivationContext) -> String {
    let mut output = value.to_string();
    if let Some(project_dir) = &context.project_dir {
        output = output.replace("${project.basedir}", &project_dir.display().to_string());
    }
    for (key, property_value) in &context.properties {
        output = output.replace(&format!("${{{key}}}"), property_value);
    }
    output
}

impl PomParent {
    fn coordinate(
        &self,
        properties: &PomPropertyContext,
        source: &str,
    ) -> Result<Option<Coordinate>, PomError> {
        let Some(group_id) = &self.group_id else {
            return Ok(None);
        };
        let Some(artifact_id) = &self.artifact_id else {
            return Ok(None);
        };
        let Some(version) = &self.version else {
            return Ok(None);
        };

        Ok(Some(Coordinate::new(
            &properties.interpolate(group_id, source)?,
            &properties.interpolate(artifact_id, source)?,
            &properties.interpolate(version, source)?,
        )))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PomPropertyContext {
    values: BTreeMap<String, String>,
}

impl PomPropertyContext {
    pub(crate) fn new(
        mut values: BTreeMap<String, String>,
        group_id: Option<String>,
        artifact_id: Option<String>,
        version: Option<String>,
    ) -> Self {
        if let Some(group_id) = group_id {
            values.insert("project.groupId".to_string(), group_id.clone());
            values.insert("pom.groupId".to_string(), group_id);
        }
        if let Some(artifact_id) = artifact_id {
            values.insert("project.artifactId".to_string(), artifact_id.clone());
            values.insert("pom.artifactId".to_string(), artifact_id);
        }
        if let Some(version) = version {
            values.insert("project.version".to_string(), version.clone());
            values.insert("pom.version".to_string(), version);
        }

        Self { values }
    }

    #[cfg(test)]
    fn from_values(values: BTreeMap<String, String>) -> Self {
        Self { values }
    }

    pub(crate) fn interpolate(&self, value: &str, source: &str) -> Result<String, PomError> {
        self.interpolate_with_stack(value, source, &mut Vec::new())
    }

    fn interpolate_with_stack(
        &self,
        value: &str,
        source: &str,
        stack: &mut Vec<String>,
    ) -> Result<String, PomError> {
        let mut output = String::new();
        let mut remainder = value;

        while let Some(start) = remainder.find("${") {
            output.push_str(&remainder[..start]);
            let after_start = &remainder[start + 2..];
            let Some(end) = after_start.find('}') else {
                return Err(PomError::InvalidProperty {
                    pom: source.to_string(),
                    value: value.to_string(),
                });
            };

            let property_name = &after_start[..end];
            if let Some(cycle_start) = stack.iter().position(|seen| seen == property_name) {
                let mut cycle = stack[cycle_start..].to_vec();
                cycle.push(property_name.to_string());
                return Err(PomError::CyclicProperty {
                    pom: source.to_string(),
                    cycle: cycle.join(" -> "),
                });
            }

            let Some(property_value) = self.values.get(property_name) else {
                return Err(PomError::UnresolvedProperty {
                    pom: source.to_string(),
                    property: property_name.to_string(),
                });
            };

            stack.push(property_name.to_string());
            output.push_str(&self.interpolate_with_stack(property_value, source, stack)?);
            stack.pop();
            remainder = &after_start[end + 1..];
        }

        output.push_str(remainder);
        Ok(output)
    }
}

#[allow(clippy::type_complexity)]
fn read_all_properties(
    raw: &str,
) -> Result<(BTreeMap<String, String>, Vec<BTreeMap<String, String>>), PomError> {
    let mut reader = Reader::from_str(raw);
    reader.config_mut().trim_text(true);

    let mut project_properties = BTreeMap::new();
    let mut profile_properties_list = Vec::new();

    let mut stack = Vec::<String>::new();
    let mut current_profile_index = None::<usize>;
    let mut seen_profiles = 0usize;
    let mut in_properties = false;
    let mut properties_depth = 0usize;
    let mut current_property = None;
    let mut current_value = String::new();

    loop {
        match reader.read_event()? {
            Event::Start(element) => {
                let name = String::from_utf8_lossy(element.local_name().as_ref()).into_owned();
                if name == "profile" && stack_is(&stack, &["project", "profiles"]) {
                    current_profile_index = Some(seen_profiles);
                    seen_profiles += 1;
                    profile_properties_list.push(BTreeMap::new());
                }

                if !in_properties && name == "properties" {
                    let is_project = stack_is(&stack, &["project"]);
                    let is_profile = current_profile_index.is_some()
                        && stack_is(&stack, &["project", "profiles", "profile"]);
                    if is_project || is_profile {
                        in_properties = true;
                    }
                } else if in_properties {
                    properties_depth += 1;
                    if properties_depth == 1 && current_property.is_none() {
                        current_property = Some(name.clone());
                        current_value.clear();
                    }
                }
                stack.push(name);
            }
            Event::Empty(element) if in_properties && properties_depth == 0 => {
                let key = String::from_utf8_lossy(element.local_name().as_ref()).into_owned();
                if let Some(idx) = current_profile_index {
                    if let Some(profile_map) = profile_properties_list.get_mut(idx) {
                        profile_map.insert(key, String::new());
                    }
                } else {
                    project_properties.insert(key, String::new());
                }
            }
            Event::Text(text) if current_property.is_some() => {
                let decoded = text.decode()?;
                current_value.push_str(&unescape(&decoded)?);
            }
            Event::CData(text) if current_property.is_some() => {
                current_value.push_str(&text.decode()?);
            }
            Event::End(element)
                if in_properties
                    && properties_depth == 0
                    && element.local_name().as_ref() == b"properties" =>
            {
                in_properties = false;
                stack.pop();
            }
            Event::End(_) if in_properties => {
                if properties_depth == 1
                    && let Some(property) = current_property.take()
                {
                    let val = current_value.trim().to_string();
                    if let Some(idx) = current_profile_index {
                        if let Some(profile_map) = profile_properties_list.get_mut(idx) {
                            profile_map.insert(property, val);
                        }
                    } else {
                        project_properties.insert(property, val);
                    }
                    current_value.clear();
                }
                properties_depth = properties_depth.saturating_sub(1);
                stack.pop();
            }
            Event::End(element) => {
                if element.local_name().as_ref() == b"profile"
                    && stack_is(&stack, &["project", "profiles", "profile"])
                {
                    current_profile_index = None;
                }
                stack.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok((project_properties, profile_properties_list))
}

fn stack_is(stack: &[String], expected: &[&str]) -> bool {
    stack.len() == expected.len()
        && stack
            .iter()
            .zip(expected)
            .all(|(left, right)| left == right)
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct PomDependencyManagement {
    #[serde(default)]
    dependencies: PomDependencies,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct PomDependencies {
    #[serde(rename = "dependency", default)]
    dependencies: Vec<PomDependency>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PomDependency {
    group_id: Option<String>,
    artifact_id: Option<String>,
    version: Option<String>,
    #[serde(rename = "type")]
    dependency_type: Option<String>,
    classifier: Option<String>,
    #[serde(default)]
    scope: PomScope,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    exclusions: PomExclusions,
}

impl PomDependency {
    pub(crate) fn is_bom_import(&self) -> bool {
        self.scope.is_import() && self.dependency_type.as_deref().unwrap_or("jar") == "pom"
    }

    pub(crate) fn unsupported_scope(&self) -> Option<&str> {
        self.scope.unsupported_scope()
    }

    pub(crate) fn is_optional(&self) -> bool {
        self.optional
    }

    pub(crate) fn graph_scope(&self) -> Option<Scope> {
        self.scope.graph_scope()
    }

    pub(crate) fn resolve(
        &self,
        properties: &PomPropertyContext,
        source: &str,
        management: &BTreeMap<ArtifactIdentity, ManagedDependency>,
    ) -> Result<Option<ResolvedPomDependency>, PomError> {
        let Some(artifact) = self.artifact_without_version(properties, source)? else {
            return Ok(None);
        };

        let identity = artifact.identity();
        let managed = management.get(&identity);
        let version = self
            .version
            .as_deref()
            .map(|version| properties.interpolate(version, source))
            .transpose()?
            .or_else(|| managed.and_then(|dependency| dependency.version.clone()));
        let Some(version) = version else {
            return Ok(None);
        };

        let scope = if self.scope.is_explicit_graph_scope() {
            self.scope.graph_scope().unwrap_or_default()
        } else {
            managed
                .and_then(|dependency| dependency.scope)
                .unwrap_or_default()
        };

        let mut exclusions = managed
            .map(|dependency| dependency.exclusions.clone())
            .unwrap_or_default();
        exclusions.extend(self.exclusions(properties, source)?);

        Ok(Some(ResolvedPomDependency {
            artifact: ArtifactCoordinate::new(
                Coordinate::new(&identity.group, &identity.artifact, &version),
                artifact.artifact_type,
                artifact.classifier,
            ),
            scope,
            exclusions,
        }))
    }

    pub(crate) fn managed_dependency(
        &self,
        properties: &PomPropertyContext,
        source: &str,
    ) -> Result<Option<(ArtifactIdentity, ManagedDependency)>, PomError> {
        let artifact = match self.artifact_without_version(properties, source) {
            Ok(Some(artifact)) => artifact,
            Ok(None) => return Ok(None),
            Err(PomError::UnsupportedArtifactType { .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        let identity = artifact.identity();

        let version = self
            .version
            .as_deref()
            .map(|version| properties.interpolate(version, source))
            .transpose()?;
        let scope = self.scope.graph_scope();
        let exclusions = self.exclusions(properties, source)?;

        Ok(Some((
            identity,
            ManagedDependency {
                version,
                scope,
                exclusions,
            },
        )))
    }

    pub(crate) fn coordinate(
        &self,
        properties: &PomPropertyContext,
        source: &str,
    ) -> Result<Option<Coordinate>, PomError> {
        let Some(resolved) = self.resolve(properties, source, &BTreeMap::new())? else {
            return Ok(None);
        };

        Ok(Some(resolved.artifact.coordinate))
    }

    fn artifact_without_version(
        &self,
        properties: &PomPropertyContext,
        source: &str,
    ) -> Result<Option<ArtifactCoordinate>, PomError> {
        let Some(group_id) = &self.group_id else {
            return Ok(None);
        };
        let Some(artifact_id) = &self.artifact_id else {
            return Ok(None);
        };

        let artifact_type = self.artifact_type(properties, source)?;
        let classifier = self
            .classifier
            .as_deref()
            .map(|classifier| properties.interpolate(classifier, source))
            .transpose()?;

        Ok(Some(ArtifactCoordinate::new(
            Coordinate::new(
                &properties.interpolate(group_id, source)?,
                &properties.interpolate(artifact_id, source)?,
                "",
            ),
            artifact_type,
            classifier,
        )))
    }

    fn artifact_type(
        &self,
        properties: &PomPropertyContext,
        source: &str,
    ) -> Result<ArtifactType, PomError> {
        let Some(value) = &self.dependency_type else {
            return Ok(ArtifactType::Jar);
        };

        match properties.interpolate(value, source)?.as_str() {
            "" | "jar" => Ok(ArtifactType::Jar),
            "pom" => Ok(ArtifactType::Pom),
            "war" => Ok(ArtifactType::War),
            artifact_type => Err(PomError::UnsupportedArtifactType {
                pom: source.to_string(),
                artifact_type: artifact_type.to_string(),
            }),
        }
    }

    fn exclusions(
        &self,
        properties: &PomPropertyContext,
        source: &str,
    ) -> Result<Vec<Coordinate>, PomError> {
        self.exclusions
            .exclusions
            .iter()
            .map(|exclusion| {
                let group = exclusion
                    .group_id
                    .as_deref()
                    .map(|value| properties.interpolate(value, source))
                    .transpose()?
                    .unwrap_or_default();
                let artifact = exclusion
                    .artifact_id
                    .as_deref()
                    .map(|value| properties.interpolate(value, source))
                    .transpose()?
                    .unwrap_or_default();

                Ok(Coordinate::new(&group, &artifact, ""))
            })
            .filter(|coordinate| match coordinate {
                Ok(coordinate) => !coordinate.group.is_empty() && !coordinate.artifact.is_empty(),
                Err(_) => true,
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedPomDependency {
    pub artifact: ArtifactCoordinate,
    pub scope: Scope,
    pub exclusions: Vec<Coordinate>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
struct PomExclusions {
    #[serde(rename = "exclusion", default)]
    exclusions: Vec<PomExclusion>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PomExclusion {
    group_id: Option<String>,
    artifact_id: Option<String>,
}

#[derive(Debug, Clone)]
struct PomScope {
    scope: Scope,
    explicit: bool,
    import: bool,
    unsupported: Option<String>,
}

impl PomScope {
    fn graph_scope(&self) -> Option<Scope> {
        if self.import { None } else { Some(self.scope) }
    }

    fn is_explicit_graph_scope(&self) -> bool {
        self.explicit && !self.import
    }

    fn is_import(&self) -> bool {
        self.import
    }

    fn unsupported_scope(&self) -> Option<&str> {
        self.unsupported.as_deref()
    }
}

impl Default for PomScope {
    fn default() -> Self {
        Self {
            scope: Scope::Compile,
            explicit: false,
            import: false,
            unsupported: None,
        }
    }
}

impl<'de> serde::Deserialize<'de> for PomScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let scope = Option::<String>::deserialize(deserializer)?;
        Ok(match scope.as_deref() {
            None | Some("") => Self::default(),
            Some("compile") => Self {
                scope: Scope::Compile,
                explicit: true,
                import: false,
                unsupported: None,
            },
            Some("runtime") => Self {
                scope: Scope::Runtime,
                explicit: true,
                import: false,
                unsupported: None,
            },
            Some("test") => Self {
                scope: Scope::Test,
                explicit: true,
                import: false,
                unsupported: None,
            },
            Some("provided") => Self {
                scope: Scope::Provided,
                explicit: true,
                import: false,
                unsupported: None,
            },
            Some("import") => Self {
                scope: Scope::Compile,
                explicit: true,
                import: true,
                unsupported: None,
            },
            Some(scope) => Self {
                scope: Scope::Compile,
                explicit: true,
                import: false,
                unsupported: Some(scope.to_string()),
            },
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PomError {
    #[error("POM `{pom}` has invalid property expression `{value}`")]
    InvalidProperty { pom: String, value: String },
    #[error("POM `{pom}` uses unresolved property `${{{property}}}`")]
    UnresolvedProperty { pom: String, property: String },
    #[error("POM `{pom}` has cyclic property interpolation `{cycle}`")]
    CyclicProperty { pom: String, cycle: String },
    #[error("POM `{pom}` uses unsupported dependency type `{artifact_type}`")]
    UnsupportedArtifactType { pom: String, artifact_type: String },
    #[error("failed filesystem operation: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse Maven POM: {0}")]
    Xml(#[from] quick_xml::DeError),
    #[error("failed to read Maven POM XML: {0}")]
    XmlRead(#[from] quick_xml::Error),
    #[error("failed to decode Maven POM XML: {0}")]
    XmlDecode(#[from] quick_xml::encoding::EncodingError),
    #[error("failed to unescape Maven POM XML: {0}")]
    XmlEscape(#[from] quick_xml::escape::EscapeError),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::TempDir;

    use super::*;

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
        let effective = parsed.effective_without_parent();
        let properties = effective.property_context();
        let dependency = effective.dependencies.first().unwrap();

        assert_eq!(
            dependency
                .coordinate(&properties, "com.example:demo:1.0.0")
                .unwrap()
                .unwrap()
                .to_string(),
            "com.example:dep:1.0.0"
        );
        assert_eq!(dependency.graph_scope(), Some(Scope::Runtime));
    }

    #[test]
    fn interpolates_pom_dependency_properties() {
        let dir = TempDir::new().unwrap();
        let pom = dir.path().join("demo.pom");
        fs::write(
            &pom,
            r#"
            <project>
              <groupId>com.example</groupId>
              <artifactId>root</artifactId>
              <version>1.0.0</version>
              <properties>
                <child.artifact>child</child.artifact>
                <child.version>${project.version}</child.version>
              </properties>
              <dependencies>
                <dependency>
                  <groupId>${project.groupId}</groupId>
                  <artifactId>${child.artifact}</artifactId>
                  <version>${child.version}</version>
                </dependency>
              </dependencies>
            </project>
            "#,
        )
        .unwrap();

        let parsed = Pom::read(&pom).unwrap();
        let effective = parsed.effective_without_parent();
        let properties = effective.property_context();
        let dependency = effective.dependencies.first().unwrap();

        assert_eq!(
            dependency
                .coordinate(&properties, "com.example:root:1.0.0")
                .unwrap()
                .unwrap()
                .to_string(),
            "com.example:child:1.0.0"
        );
    }

    #[test]
    fn distinguishes_missing_pom_property_from_cycles() {
        let properties = PomPropertyContext::from_values(BTreeMap::from([
            ("a".to_string(), "${b}".to_string()),
            ("b".to_string(), "${a}".to_string()),
        ]));

        let missing = properties
            .interpolate("${missing}", "com.example:root:1.0.0")
            .unwrap_err();
        assert!(matches!(
            missing,
            PomError::UnresolvedProperty {
                property,
                ..
            } if property == "missing"
        ));

        let cycle = properties
            .interpolate("${a}", "com.example:root:1.0.0")
            .unwrap_err();
        assert!(matches!(
            cycle,
            PomError::CyclicProperty {
                cycle,
                ..
            } if cycle == "a -> b -> a"
        ));
    }

    #[test]
    fn resolves_dependency_version_from_management() {
        let dependency = PomDependency {
            group_id: Some("com.example".to_string()),
            artifact_id: Some("child".to_string()),
            ..Default::default()
        };
        let properties = PomPropertyContext::from_values(BTreeMap::new());
        let management = BTreeMap::from([(
            ArtifactIdentity {
                group: "com.example".to_string(),
                artifact: "child".to_string(),
                artifact_type: ArtifactType::Jar,
                classifier: None,
            },
            ManagedDependency {
                version: Some("1.0.0".to_string()),
                scope: None,
                exclusions: Vec::new(),
            },
        )]);

        let resolved = dependency
            .resolve(&properties, "com.example:root:1.0.0", &management)
            .unwrap()
            .unwrap();

        assert_eq!(
            resolved.artifact.coordinate.to_string(),
            "com.example:child:1.0.0"
        );
    }

    #[test]
    fn resolves_dependency_type_and_classifier() {
        let dependency = PomDependency {
            group_id: Some("com.example".to_string()),
            artifact_id: Some("native".to_string()),
            version: Some("1.0.0".to_string()),
            dependency_type: Some("jar".to_string()),
            classifier: Some("linux-aarch64".to_string()),
            ..Default::default()
        };
        let properties = PomPropertyContext::from_values(BTreeMap::new());

        let resolved = dependency
            .resolve(&properties, "com.example:root:1.0.0", &BTreeMap::new())
            .unwrap()
            .unwrap();

        assert_eq!(resolved.artifact.artifact_type, ArtifactType::Jar);
        assert_eq!(
            resolved.artifact.classifier.as_deref(),
            Some("linux-aarch64")
        );
        assert_eq!(
            resolved.artifact.to_string(),
            "com.example:native:jar:linux-aarch64:1.0.0"
        );
    }

    #[test]
    fn ignores_unsupported_dependency_types_in_management_only() {
        let dependency = PomDependency {
            group_id: Some("com.example".to_string()),
            artifact_id: Some("native".to_string()),
            version: Some("1.0.0".to_string()),
            dependency_type: Some("klib".to_string()),
            ..Default::default()
        };
        let properties = PomPropertyContext::from_values(BTreeMap::new());

        let managed = dependency
            .managed_dependency(&properties, "com.example:root:1.0.0")
            .unwrap();
        assert!(managed.is_none());

        let direct = dependency
            .resolve(&properties, "com.example:root:1.0.0", &BTreeMap::new())
            .unwrap_err();
        assert!(matches!(
            direct,
            PomError::UnsupportedArtifactType {
                artifact_type,
                ..
            } if artifact_type == "klib"
        ));
    }

    #[test]
    fn parses_and_inherits_repositories() {
        let dir = TempDir::new().unwrap();
        let parent_pom = dir.path().join("parent.pom");
        let child_pom = dir.path().join("child.pom");

        fs::write(
            &parent_pom,
            r#"
            <project>
              <groupId>com.example</groupId>
              <artifactId>parent</artifactId>
              <version>1.0.0</version>
              <repositories>
                <repository>
                  <id>parent-repo</id>
                  <url>https://parent.example.com/maven2</url>
                </repository>
                <repository>
                  <id>shared-repo</id>
                  <url>https://parent-shared.example.com/maven2</url>
                </repository>
              </repositories>
            </project>
            "#,
        )
        .unwrap();

        fs::write(
            &child_pom,
            r#"
            <project>
              <groupId>com.example</groupId>
              <artifactId>child</artifactId>
              <version>1.0.0</version>
              <properties>
                <custom.repo.url>https://child.example.com/maven2</custom.repo.url>
              </properties>
              <repositories>
                <repository>
                  <id>child-repo</id>
                  <url>${custom.repo.url}</url>
                </repository>
                <repository>
                  <id>shared-repo</id>
                  <url>https://child-override.example.com/maven2</url>
                </repository>
              </repositories>
            </project>
            "#,
        )
        .unwrap();

        let parent_parsed = Pom::read(&parent_pom).unwrap();
        let parent_effective = parent_parsed.effective_without_parent();
        assert_eq!(parent_effective.repositories.len(), 2);
        assert_eq!(parent_effective.repositories[0].name, "parent-repo");
        assert_eq!(
            parent_effective.repositories[0].url,
            "https://parent.example.com/maven2"
        );

        let child_parsed = Pom::read(&child_pom).unwrap();
        let child_effective = child_parsed.merge_with_parent(Some(parent_effective));

        assert_eq!(child_effective.repositories.len(), 3);

        let parent_repo = child_effective
            .repositories
            .iter()
            .find(|r| r.name == "parent-repo")
            .unwrap();
        assert_eq!(parent_repo.url, "https://parent.example.com/maven2");

        let child_repo = child_effective
            .repositories
            .iter()
            .find(|r| r.name == "child-repo")
            .unwrap();
        assert_eq!(child_repo.url, "https://child.example.com/maven2");

        let shared_repo = child_effective
            .repositories
            .iter()
            .find(|r| r.name == "shared-repo")
            .unwrap();
        assert_eq!(shared_repo.url, "https://child-override.example.com/maven2");
    }

    #[test]
    fn parses_repository_policies_from_pom() {
        let dir = TempDir::new().unwrap();
        let pom_path = dir.path().join("pom.xml");

        fs::write(
            &pom_path,
            r#"
            <project>
              <groupId>com.example</groupId>
              <artifactId>test</artifactId>
              <version>1.0.0</version>
              <repositories>
                <repository>
                  <id>releases-only</id>
                  <url>https://releases.example.com/maven2</url>
                  <snapshots>
                    <enabled>false</enabled>
                  </snapshots>
                </repository>
                <repository>
                  <id>snapshots-only</id>
                  <url>https://snapshots.example.com/maven2</url>
                  <releases>
                    <enabled>false</enabled>
                  </releases>
                </repository>
                <repository>
                  <id>both</id>
                  <url>https://both.example.com/maven2</url>
                </repository>
              </repositories>
            </project>
            "#,
        )
        .unwrap();

        let pom = Pom::read(&pom_path).unwrap();
        let effective = pom.effective_without_parent();

        assert_eq!(effective.repositories.len(), 3);

        let releases_only = effective
            .repositories
            .iter()
            .find(|r| r.name == "releases-only")
            .unwrap();
        assert!(releases_only.releases.enabled);
        assert!(!releases_only.snapshots.enabled);

        let snapshots_only = effective
            .repositories
            .iter()
            .find(|r| r.name == "snapshots-only")
            .unwrap();
        assert!(!snapshots_only.releases.enabled);
        assert!(snapshots_only.snapshots.enabled);

        let both = effective
            .repositories
            .iter()
            .find(|r| r.name == "both")
            .unwrap();
        assert!(both.releases.enabled);
        assert!(both.snapshots.enabled);
    }
}
