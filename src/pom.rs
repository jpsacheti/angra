use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use quick_xml::{Reader, escape::unescape, events::Event};

use crate::maven::{ArtifactCoordinate, ArtifactIdentity, ArtifactType, Coordinate, Scope};

#[derive(Debug, Clone)]
pub(crate) struct EffectivePom {
    pub group_id: Option<String>,
    pub artifact_id: Option<String>,
    pub version: Option<String>,
    pub properties: BTreeMap<String, String>,
    pub dependency_management: BTreeMap<ArtifactIdentity, ManagedDependency>,
    pub dependencies: Vec<PomDependency>,
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

#[derive(Debug, serde::Deserialize)]
#[serde(rename = "project")]
#[serde(rename_all = "camelCase")]
pub(crate) struct Pom {
    group_id: Option<String>,
    artifact_id: Option<String>,
    version: Option<String>,
    parent: Option<PomParent>,
    #[serde(skip)]
    properties: BTreeMap<String, String>,
    #[serde(default)]
    dependency_management: PomDependencyManagement,
    #[serde(default)]
    dependencies: PomDependencies,
}

impl Pom {
    pub(crate) fn read(path: &Path) -> Result<Self, PomError> {
        let raw = fs::read_to_string(path)?;
        let mut pom: Self = quick_xml::de::from_str(&raw)?;
        pom.properties = read_pom_properties(&raw)?;
        Ok(pom)
    }

    pub(crate) fn parent_coordinate(&self, source: &str) -> Result<Option<Coordinate>, PomError> {
        let Some(parent) = &self.parent else {
            return Ok(None);
        };

        let context = PomPropertyContext::new(
            self.properties.clone(),
            self.group_id.clone(),
            self.artifact_id.clone(),
            self.version.clone(),
        );

        parent.coordinate(&context, source)
    }

    pub(crate) fn effective_without_parent(&self) -> EffectivePom {
        EffectivePom {
            group_id: self.group_id.clone(),
            artifact_id: self.artifact_id.clone(),
            version: self.version.clone(),
            properties: self.properties.clone(),
            dependency_management: BTreeMap::new(),
            dependencies: self.dependencies.dependencies.clone(),
        }
    }

    pub(crate) fn merge_with_parent(&self, parent: Option<EffectivePom>) -> EffectivePom {
        let mut effective = parent.unwrap_or_else(|| self.effective_without_parent());

        effective.group_id = self.group_id.clone().or(effective.group_id);
        effective.artifact_id = self.artifact_id.clone().or(effective.artifact_id);
        effective.version = self.version.clone().or(effective.version);

        effective.properties.extend(self.properties.clone());
        effective.dependencies = self.dependencies.dependencies.clone();
        effective
    }

    pub(crate) fn dependency_management_entries(&self) -> &[PomDependency] {
        &self.dependency_management.dependencies.dependencies
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PomParent {
    group_id: Option<String>,
    artifact_id: Option<String>,
    version: Option<String>,
    #[allow(dead_code)]
    relative_path: Option<PathBuf>,
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

fn read_pom_properties(raw: &str) -> Result<BTreeMap<String, String>, PomError> {
    let mut reader = Reader::from_str(raw);
    reader.config_mut().trim_text(true);

    let mut values = BTreeMap::new();
    let mut in_properties = false;
    let mut properties_depth = 0usize;
    let mut current_property = None;
    let mut current_value = String::new();

    loop {
        match reader.read_event()? {
            Event::Start(element)
                if !in_properties && element.local_name().as_ref() == b"properties" =>
            {
                in_properties = true;
            }
            Event::Start(element) if in_properties => {
                properties_depth += 1;
                if properties_depth == 1 && current_property.is_none() {
                    current_property =
                        Some(String::from_utf8_lossy(element.local_name().as_ref()).into_owned());
                    current_value.clear();
                }
            }
            Event::Empty(element) if in_properties && properties_depth == 0 => {
                values.insert(
                    String::from_utf8_lossy(element.local_name().as_ref()).into_owned(),
                    String::new(),
                );
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
            }
            Event::End(_) if in_properties => {
                if properties_depth == 1
                    && let Some(property) = current_property.take()
                {
                    values.insert(property, current_value.trim().to_string());
                    current_value.clear();
                }
                properties_depth = properties_depth.saturating_sub(1);
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(values)
}

#[derive(Debug, Default, serde::Deserialize)]
struct PomDependencyManagement {
    #[serde(default)]
    dependencies: PomDependencies,
}

#[derive(Debug, Default, serde::Deserialize)]
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
        let Some(artifact) = self.artifact_without_version(properties, source)? else {
            return Ok(None);
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

#[derive(Debug, Clone, Copy)]
struct PomScope {
    scope: Scope,
    explicit: bool,
    import: bool,
}

impl PomScope {
    fn graph_scope(self) -> Option<Scope> {
        if self.import { None } else { Some(self.scope) }
    }

    fn is_explicit_graph_scope(self) -> bool {
        self.explicit && !self.import
    }

    fn is_import(self) -> bool {
        self.import
    }
}

impl Default for PomScope {
    fn default() -> Self {
        Self {
            scope: Scope::Compile,
            explicit: false,
            import: false,
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
            },
            Some("runtime") => Self {
                scope: Scope::Runtime,
                explicit: true,
                import: false,
            },
            Some("test") => Self {
                scope: Scope::Test,
                explicit: true,
                import: false,
            },
            Some("provided") => Self {
                scope: Scope::Provided,
                explicit: true,
                import: false,
            },
            Some("import") => Self {
                scope: Scope::Compile,
                explicit: true,
                import: true,
            },
            Some(_) => Self {
                scope: Scope::Compile,
                explicit: true,
                import: false,
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
}
