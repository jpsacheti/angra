use std::{fs, path::Path};

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename = "metadata", rename_all = "camelCase")]
pub(crate) struct MavenMetadata {
    #[serde(default)]
    pub(crate) versioning: MetadataVersioning,
}

impl MavenMetadata {
    pub(crate) fn read(path: &Path) -> Result<Self, MetadataError> {
        let raw = fs::read_to_string(path)?;
        Self::parse(&raw)
    }

    pub(crate) fn parse(raw: &str) -> Result<Self, MetadataError> {
        Ok(quick_xml::de::from_str(raw)?)
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetadataVersioning {
    #[serde(default)]
    pub(crate) latest: Option<String>,
    #[serde(default)]
    pub(crate) release: Option<String>,
    #[serde(default)]
    pub(crate) versions: MetadataVersions,
    #[serde(default)]
    pub(crate) last_updated: Option<String>,
    #[serde(default)]
    pub(crate) snapshot: Option<MetadataSnapshot>,
    #[serde(default)]
    pub(crate) snapshot_versions: MetadataSnapshotVersions,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct MetadataVersions {
    #[serde(rename = "version", default)]
    pub(crate) versions: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetadataSnapshot {
    #[serde(default)]
    pub(crate) timestamp: Option<String>,
    #[serde(default)]
    pub(crate) build_number: Option<u32>,
    #[serde(default)]
    pub(crate) local_copy: bool,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetadataSnapshotVersions {
    #[serde(rename = "snapshotVersion", default)]
    pub(crate) snapshot_versions: Vec<MetadataSnapshotVersion>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(crate) struct MetadataSnapshotVersion {
    #[serde(default)]
    pub(crate) classifier: Option<String>,
    #[serde(default)]
    pub(crate) extension: Option<String>,
    #[serde(default)]
    pub(crate) value: Option<String>,
    #[serde(default)]
    pub(crate) updated: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum MetadataError {
    #[error("failed filesystem operation: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse Maven metadata XML: {0}")]
    Xml(#[from] quick_xml::DeError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions_and_snapshot_versions() {
        let metadata = MavenMetadata::parse(
            r#"
            <metadata>
              <versioning>
                <versions>
                  <version>1.0.0</version>
                  <version>1.1.0</version>
                </versions>
                <snapshotVersions>
                  <snapshotVersion>
                    <extension>jar</extension>
                    <value>1.2.0-20240501.120000-3</value>
                    <updated>20240501120000</updated>
                  </snapshotVersion>
                </snapshotVersions>
              </versioning>
            </metadata>
            "#,
        )
        .unwrap();

        assert_eq!(metadata.versioning.versions.versions[1], "1.1.0");
        assert_eq!(
            metadata.versioning.snapshot_versions.snapshot_versions[0]
                .value
                .as_deref(),
            Some("1.2.0-20240501.120000-3")
        );
    }

    #[test]
    fn parses_missing_fields_and_empty_versions() {
        let metadata = MavenMetadata::parse("<metadata></metadata>").unwrap();
        assert!(metadata.versioning.versions.versions.is_empty());
        assert!(metadata.versioning.latest.is_none());
        assert!(metadata.versioning.release.is_none());

        let metadata = MavenMetadata::parse(
            r#"
            <metadata>
              <versioning>
                <versions/>
              </versioning>
            </metadata>
            "#,
        )
        .unwrap();
        assert!(metadata.versioning.versions.versions.is_empty());
    }

    #[test]
    fn parses_snapshot_without_snapshot_versions() {
        let metadata = MavenMetadata::parse(
            r#"
            <metadata>
              <versioning>
                <snapshot>
                  <timestamp>20240501.120000</timestamp>
                  <buildNumber>3</buildNumber>
                </snapshot>
              </versioning>
            </metadata>
            "#,
        )
        .unwrap();

        assert_eq!(
            metadata
                .versioning
                .snapshot
                .as_ref()
                .unwrap()
                .timestamp
                .as_deref(),
            Some("20240501.120000")
        );
        assert_eq!(
            metadata.versioning.snapshot.as_ref().unwrap().build_number,
            Some(3)
        );
        assert!(
            metadata
                .versioning
                .snapshot_versions
                .snapshot_versions
                .is_empty()
        );
    }

    #[test]
    fn fails_on_malformed_xml() {
        assert!(MavenMetadata::parse("<metadata><unclosed>").is_err());
    }
}
