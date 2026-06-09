use std::{
    cmp::Ordering,
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
    pub checksum_policy: ChecksumPolicy,
}

impl Default for RepositoryPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            checksum_policy: ChecksumPolicy::Fail,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChecksumPolicy {
    Fail,
    Warn,
    Ignore,
    Unknown(String),
}

impl ChecksumPolicy {
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some(value) if value.eq_ignore_ascii_case("warn") => Self::Warn,
            Some(value) if value.eq_ignore_ascii_case("ignore") => Self::Ignore,
            Some(value) if value.eq_ignore_ascii_case("fail") => Self::Fail,
            Some(value) if !value.is_empty() => Self::Unknown(value.to_string()),
            _ => Self::Fail,
        }
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
        Self::with_policy_details(
            name,
            url,
            RepositoryPolicy {
                enabled: releases_enabled,
                checksum_policy: ChecksumPolicy::Fail,
            },
            RepositoryPolicy {
                enabled: snapshots_enabled,
                checksum_policy: ChecksumPolicy::Fail,
            },
        )
    }

    pub fn with_policy_details(
        name: &str,
        url: &str,
        releases: RepositoryPolicy,
        snapshots: RepositoryPolicy,
    ) -> Self {
        Self {
            name: name.to_string(),
            url: url.trim_end_matches('/').to_string(),
            releases,
            snapshots,
        }
    }

    pub fn maven_central() -> Self {
        Self::with_policies(MAVEN_CENTRAL_NAME, MAVEN_CENTRAL_URL, true, false)
    }

    /// Returns true if this repository accepts artifacts with the given coordinate,
    /// based on the release/snapshot policies.
    pub fn accepts(&self, coordinate: &Coordinate) -> bool {
        self.policy_for(coordinate).enabled
    }

    pub fn policy_for(&self, coordinate: &Coordinate) -> &RepositoryPolicy {
        if coordinate.is_snapshot() {
            &self.snapshots
        } else {
            &self.releases
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

    pub fn metadata_url(&self, coordinate: &Coordinate) -> String {
        format!(
            "{}/{}/{}/maven-metadata.xml",
            self.url,
            coordinate.group_path(),
            coordinate.artifact
        )
    }

    pub fn snapshot_metadata_url(&self, coordinate: &Coordinate) -> String {
        format!(
            "{}/{}/{}/{}/maven-metadata.xml",
            self.url,
            coordinate.group_path(),
            coordinate.artifact,
            coordinate.version
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

    pub fn metadata_path(&self, local_repo: &Path) -> PathBuf {
        local_repo
            .join(self.group_path())
            .join(&self.artifact)
            .join("maven-metadata.xml")
    }

    pub fn repository_metadata_path(&self, local_repo: &Path, repository_name: &str) -> PathBuf {
        local_repo
            .join(self.group_path())
            .join(&self.artifact)
            .join(format!("maven-metadata-{repository_name}.xml"))
    }

    pub fn snapshot_metadata_path(&self, local_repo: &Path, repository_name: &str) -> PathBuf {
        self.local_dir(local_repo)
            .join(format!("maven-metadata-{repository_name}.xml"))
    }

    pub fn central_pom_url(&self) -> String {
        Repository::maven_central().pom_url(self)
    }

    pub fn central_jar_url(&self) -> String {
        ArtifactCoordinate::jar(self.clone()).central_artifact_url()
    }
}

#[derive(Debug, Clone)]
pub struct MavenVersion {
    raw: String,
    items: Vec<VersionItem>,
}

impl PartialEq for MavenVersion {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for MavenVersion {}

impl MavenVersion {
    pub fn new(value: &str) -> Self {
        Self {
            raw: value.to_string(),
            items: parse_version_items(value),
        }
    }
}

impl Display for MavenVersion {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.raw)
    }
}

impl PartialOrd for MavenVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MavenVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_version_items(&self.items, &other.items)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VersionItem {
    Number(u128),
    Text(String),
    Separator(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRange {
    recommended: Option<MavenVersion>,
    restrictions: Vec<VersionRestriction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VersionRestriction {
    lower: Option<MavenVersion>,
    lower_inclusive: bool,
    upper: Option<MavenVersion>,
    upper_inclusive: bool,
}

impl VersionRange {
    pub fn parse(value: &str) -> Result<Self, CoordinateError> {
        let value = value.trim();
        if value.is_empty() {
            return Err(CoordinateError::InvalidVersionRange(value.to_string()));
        }
        if !value.starts_with(['[', '(']) {
            return Ok(Self {
                recommended: Some(MavenVersion::new(value)),
                restrictions: Vec::new(),
            });
        }

        let mut remaining = value;
        let mut restrictions = Vec::new();
        let mut previous_upper = None::<MavenVersion>;
        while remaining.starts_with(['[', '(']) {
            let Some(end) = remaining.find([')', ']']) else {
                return Err(CoordinateError::InvalidVersionRange(value.to_string()));
            };
            let raw_restriction = &remaining[..=end];
            let restriction = VersionRestriction::parse(raw_restriction)
                .map_err(|_| CoordinateError::InvalidVersionRange(value.to_string()))?;
            if let Some(previous) = &previous_upper
                && restriction
                    .lower
                    .as_ref()
                    .is_none_or(|lower| lower < previous)
            {
                return Err(CoordinateError::InvalidVersionRange(value.to_string()));
            }
            previous_upper = restriction.upper.clone();
            restrictions.push(restriction);
            remaining = remaining[end + 1..].trim_start();
            if let Some(after_comma) = remaining.strip_prefix(',') {
                remaining = after_comma.trim_start();
            }
        }

        if !remaining.is_empty() || restrictions.is_empty() {
            return Err(CoordinateError::InvalidVersionRange(value.to_string()));
        }

        Ok(Self {
            recommended: None,
            restrictions,
        })
    }

    pub fn is_range_spec(value: &str) -> bool {
        value.trim().starts_with(['[', '('])
    }

    pub fn is_range(&self) -> bool {
        !self.restrictions.is_empty()
    }

    pub fn exact_version(&self) -> Option<&str> {
        self.recommended
            .as_ref()
            .map(|version| version.raw.as_str())
    }

    pub fn contains(&self, version: &str) -> bool {
        let version = MavenVersion::new(version);
        if self.restrictions.is_empty() {
            return self.recommended.as_ref().is_some_and(|v| v == &version);
        }
        self.restrictions
            .iter()
            .any(|restriction| restriction.contains(&version))
    }

    pub fn highest_matching<'a>(
        &self,
        versions: impl IntoIterator<Item = &'a str>,
    ) -> Option<String> {
        versions
            .into_iter()
            .filter(|version| self.contains(version))
            .map(|version| (MavenVersion::new(version), version.to_string()))
            .max_by(|left, right| left.0.cmp(&right.0))
            .map(|(_, version)| version)
    }
}

impl VersionRestriction {
    fn parse(value: &str) -> Result<Self, ()> {
        let lower_inclusive = value.starts_with('[');
        let upper_inclusive = value.ends_with(']');
        let body = value[1..value.len() - 1].trim();
        let Some(comma) = body.find(',') else {
            if !lower_inclusive || !upper_inclusive || body.is_empty() {
                return Err(());
            }
            let version = MavenVersion::new(body);
            return Ok(Self {
                lower: Some(version.clone()),
                lower_inclusive: true,
                upper: Some(version),
                upper_inclusive: true,
            });
        };

        let lower = body[..comma].trim();
        let upper = body[comma + 1..].trim();
        let lower = (!lower.is_empty()).then(|| MavenVersion::new(lower));
        let upper = (!upper.is_empty()).then(|| MavenVersion::new(upper));

        if let (Some(lower), Some(upper)) = (&lower, &upper) {
            match lower.cmp(upper) {
                Ordering::Greater => return Err(()),
                Ordering::Equal if !lower_inclusive || !upper_inclusive => return Err(()),
                _ => {}
            }
        }

        Ok(Self {
            lower,
            lower_inclusive,
            upper,
            upper_inclusive,
        })
    }

    fn contains(&self, version: &MavenVersion) -> bool {
        if let Some(lower) = &self.lower {
            match version.cmp(lower) {
                Ordering::Less => return false,
                Ordering::Equal if !self.lower_inclusive => return false,
                _ => {}
            }
        }
        if let Some(upper) = &self.upper {
            match version.cmp(upper) {
                Ordering::Greater => return false,
                Ordering::Equal if !self.upper_inclusive => return false,
                _ => {}
            }
        }
        true
    }
}

fn parse_version_items(value: &str) -> Vec<VersionItem> {
    let mut items = Vec::new();
    let mut token = String::new();
    let mut token_is_digit = None::<bool>;
    let mut last_separator = '.';

    for character in value.chars() {
        if character == '.' || character == '-' || character == '_' {
            push_version_token(&mut items, &mut token, token_is_digit, last_separator);
            token_is_digit = None;
            last_separator = character;
            continue;
        }

        let is_digit = character.is_ascii_digit();
        if let Some(existing) = token_is_digit
            && existing != is_digit
        {
            push_version_token(&mut items, &mut token, token_is_digit, last_separator);
            last_separator = '.';
        }
        token_is_digit = Some(is_digit);
        token.push(character);
    }
    push_version_token(&mut items, &mut token, token_is_digit, last_separator);

    while items.last().is_some_and(version_item_is_zero) {
        items.pop();
    }
    items
}

fn push_version_token(
    items: &mut Vec<VersionItem>,
    token: &mut String,
    token_is_digit: Option<bool>,
    separator: char,
) {
    if token.is_empty() {
        return;
    }
    if !items.is_empty() {
        items.push(VersionItem::Separator(separator));
    }
    if token_is_digit == Some(true) {
        items.push(VersionItem::Number(
            token.trim_start_matches('0').parse().unwrap_or(0),
        ));
    } else {
        items.push(VersionItem::Text(normalize_qualifier(token)));
    }
    token.clear();
}

fn normalize_qualifier(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "a" => "alpha".to_string(),
        "b" => "beta".to_string(),
        "m" => "milestone".to_string(),
        "cr" => "rc".to_string(),
        "ga" | "final" | "release" => String::new(),
        value => value.to_string(),
    }
}

fn compare_version_items(left: &[VersionItem], right: &[VersionItem]) -> Ordering {
    let max = left.len().max(right.len());
    for index in 0..max {
        let order = compare_version_item(left.get(index), right.get(index));
        if order != Ordering::Equal {
            return order;
        }
    }
    Ordering::Equal
}

fn compare_version_item(left: Option<&VersionItem>, right: Option<&VersionItem>) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(VersionItem::Separator(_))) | (Some(VersionItem::Separator(_)), None) => {
            Ordering::Equal
        }
        (Some(item), None) => compare_version_item(Some(item), Some(&null_item_for(item))),
        (None, Some(item)) => compare_version_item(Some(&null_item_for(item)), Some(item)),
        (Some(VersionItem::Separator(left)), Some(VersionItem::Separator(right))) => {
            separator_rank(*left).cmp(&separator_rank(*right))
        }
        (Some(VersionItem::Number(left)), Some(VersionItem::Number(right))) => left.cmp(right),
        (Some(VersionItem::Text(left)), Some(VersionItem::Text(right))) => {
            qualifier_rank(left).cmp(&qualifier_rank(right))
        }
        (Some(VersionItem::Number(_)), Some(VersionItem::Text(_))) => Ordering::Greater,
        (Some(VersionItem::Text(_)), Some(VersionItem::Number(_))) => Ordering::Less,
        (Some(VersionItem::Separator(_)), Some(_)) => Ordering::Less,
        (Some(_), Some(VersionItem::Separator(_))) => Ordering::Greater,
    }
}

fn null_item_for(item: &VersionItem) -> VersionItem {
    match item {
        VersionItem::Number(_) => VersionItem::Number(0),
        VersionItem::Text(_) => VersionItem::Text(String::new()),
        VersionItem::Separator(_) => VersionItem::Separator('.'),
    }
}

fn version_item_is_zero(item: &VersionItem) -> bool {
    match item {
        VersionItem::Number(value) => *value == 0,
        VersionItem::Text(value) => value.is_empty(),
        VersionItem::Separator(_) => true,
    }
}

fn separator_rank(separator: char) -> u8 {
    match separator {
        '-' => 0,
        '_' => 1,
        _ => 2,
    }
}

fn qualifier_rank(value: &str) -> (u8, &str) {
    match value {
        "alpha" => (0, ""),
        "beta" => (1, ""),
        "milestone" => (2, ""),
        "rc" => (3, ""),
        "snapshot" => (4, ""),
        "" => (5, ""),
        "sp" => (6, ""),
        value => (7, value),
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

    pub(crate) fn version_suffix(&self) -> String {
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
    #[error("invalid Maven version range `{0}`")]
    InvalidVersionRange(String),
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

    #[test]
    fn orders_maven_versions_like_common_comparable_version_cases() {
        let ordered = [
            "1-alpha2snapshot",
            "1-alpha2",
            "1-beta-2",
            "1-m11",
            "1-rc",
            "1-SNAPSHOT",
            "1",
            "1-sp",
            "1-abc",
            "1-1",
            "1-2",
        ];

        for pair in ordered.windows(2) {
            assert!(
                MavenVersion::new(pair[0]) < MavenVersion::new(pair[1]),
                "expected {} < {}",
                pair[0],
                pair[1]
            );
        }

        assert_eq!(MavenVersion::new("1.0"), MavenVersion::new("1.0.0"));
        assert_eq!(MavenVersion::new("1ga"), MavenVersion::new("1"));
        assert_eq!(MavenVersion::new("1cr"), MavenVersion::new("1rc"));
    }

    #[test]
    fn parses_maven_version_ranges() {
        let range = VersionRange::parse("(,1.0],[1.2,)").unwrap();
        assert!(range.contains("0.9"));
        assert!(range.contains("1.0"));
        assert!(!range.contains("1.1"));
        assert!(range.contains("1.2"));

        let range = VersionRange::parse("[1.0,2.0)").unwrap();
        assert_eq!(
            range.highest_matching(["1.0", "1.5", "2.0"]),
            Some("1.5".to_string())
        );

        assert!(VersionRange::parse("(1.0)").is_err());
        assert!(VersionRange::parse("[1.1,1.0]").is_err());
    }
}
