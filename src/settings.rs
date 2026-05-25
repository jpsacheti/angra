use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::maven::Repository;

#[derive(Debug, Default, Clone)]
pub struct MavenSettings {
    pub local_repository: Option<PathBuf>,
    pub repositories: Vec<Repository>,
    pub mirrors: Vec<Mirror>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mirror {
    pub id: String,
    pub url: String,
    pub mirror_of: String,
}

impl Mirror {
    pub fn matches(&self, repository_name: &str) -> bool {
        let mut positive_match = false;
        let mut negated = false;

        for token in self.mirror_of.split(',') {
            let token = token.trim();
            if let Some(excluded) = token.strip_prefix('!') {
                let excluded = excluded.trim();
                if excluded == "*" || excluded == repository_name {
                    negated = true;
                }
            } else if token == "*" || token == repository_name {
                positive_match = true;
            }
        }

        positive_match && !negated
    }
}

impl MavenSettings {
    pub fn load() -> Result<Self, SettingsError> {
        let path = settings_path();
        Self::load_or_default(&path)
    }

    pub fn load_or_default(path: &Path) -> Result<Self, SettingsError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        Self::read(path)
    }

    pub fn read(path: &Path) -> Result<Self, SettingsError> {
        let raw = fs::read_to_string(path)?;
        Self::parse(&raw)
    }

    pub fn parse(raw: &str) -> Result<Self, SettingsError> {
        let parsed: RawSettings = quick_xml::de::from_str(raw)?;

        let mut active_ids: HashSet<&str> = parsed
            .active_profiles
            .active_profile
            .iter()
            .map(|id| id.trim())
            .collect();

        let mut repositories = Vec::new();
        let mut seen = HashSet::new();
        for profile in &parsed.profiles.profile {
            let profile_id = profile.id.as_deref().unwrap_or("").trim();
            let active_by_default = profile
                .activation
                .as_ref()
                .map(|activation| activation.active_by_default)
                .unwrap_or(false);

            let listed_active = !profile_id.is_empty() && active_ids.remove(profile_id);
            if !listed_active && !active_by_default {
                continue;
            }

            for repository in &profile.repositories.repository {
                let Some(id) = repository.id.as_deref().map(str::trim) else {
                    continue;
                };
                let Some(url) = repository.url.as_deref().map(str::trim) else {
                    continue;
                };
                if id.is_empty() || url.is_empty() || !seen.insert(id.to_string()) {
                    continue;
                }
                repositories.push(Repository::new(id, url));
            }
        }

        let mut mirrors = Vec::new();
        let mut mirror_seen = HashSet::new();
        for mirror in &parsed.mirrors.mirror {
            let Some(id) = mirror.id.as_deref().map(str::trim) else {
                continue;
            };
            let Some(url) = mirror.url.as_deref().map(str::trim) else {
                continue;
            };
            let Some(mirror_of) = mirror.mirror_of.as_deref().map(str::trim) else {
                continue;
            };
            if id.is_empty() || url.is_empty() || mirror_of.is_empty() {
                continue;
            }
            if !mirror_seen.insert(id.to_string()) {
                continue;
            }
            mirrors.push(Mirror {
                id: id.to_string(),
                url: url.trim_end_matches('/').to_string(),
                mirror_of: mirror_of.to_string(),
            });
        }

        let local_repository = parsed
            .local_repository
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);

        Ok(Self {
            local_repository,
            repositories,
            mirrors,
        })
    }

    pub fn apply_mirrors(&self, repositories: &mut Vec<Repository>) {
        if self.mirrors.is_empty() {
            return;
        }

        for repository in repositories.iter_mut() {
            if let Some(mirror) = self.mirrors.iter().find(|m| m.matches(&repository.name)) {
                repository.name = mirror.id.clone();
                repository.url = mirror.url.clone();
            }
        }

        let mut seen = HashSet::new();
        repositories.retain(|repository| seen.insert(repository.name.clone()));
    }
}

pub fn settings_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".m2")
        .join("settings.xml")
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("failed to read Maven settings: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse Maven settings XML: {0}")]
    Xml(#[from] quick_xml::DeError),
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename = "settings", rename_all = "camelCase")]
struct RawSettings {
    local_repository: Option<String>,
    #[serde(default)]
    active_profiles: ActiveProfiles,
    #[serde(default)]
    profiles: Profiles,
    #[serde(default)]
    mirrors: Mirrors,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActiveProfiles {
    #[serde(default)]
    active_profile: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Profiles {
    #[serde(default)]
    profile: Vec<Profile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    id: Option<String>,
    activation: Option<Activation>,
    #[serde(default)]
    repositories: ProfileRepositories,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Activation {
    #[serde(default)]
    active_by_default: bool,
}

#[derive(Debug, Default, Deserialize)]
struct ProfileRepositories {
    #[serde(default)]
    repository: Vec<ProfileRepository>,
}

#[derive(Debug, Deserialize)]
struct ProfileRepository {
    id: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Mirrors {
    #[serde(default)]
    mirror: Vec<RawMirror>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMirror {
    id: Option<String>,
    url: Option<String>,
    mirror_of: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_local_repository() {
        let settings = MavenSettings::parse(
            r#"
            <settings>
              <localRepository>/var/m2/repo</localRepository>
            </settings>
            "#,
        )
        .unwrap();

        assert_eq!(
            settings.local_repository,
            Some(PathBuf::from("/var/m2/repo"))
        );
        assert!(settings.repositories.is_empty());
    }

    #[test]
    fn collects_repositories_from_active_profile() {
        let settings = MavenSettings::parse(
            r#"
            <settings>
              <activeProfiles>
                <activeProfile>corporate</activeProfile>
              </activeProfiles>
              <profiles>
                <profile>
                  <id>corporate</id>
                  <repositories>
                    <repository>
                      <id>internal</id>
                      <url>https://nexus.example.com/maven/</url>
                    </repository>
                  </repositories>
                </profile>
              </profiles>
            </settings>
            "#,
        )
        .unwrap();

        assert_eq!(settings.repositories.len(), 1);
        assert_eq!(settings.repositories[0].name, "internal");
        assert_eq!(
            settings.repositories[0].url,
            "https://nexus.example.com/maven"
        );
    }

    #[test]
    fn collects_repositories_from_active_by_default_profile() {
        let settings = MavenSettings::parse(
            r#"
            <settings>
              <profiles>
                <profile>
                  <id>default</id>
                  <activation>
                    <activeByDefault>true</activeByDefault>
                  </activation>
                  <repositories>
                    <repository>
                      <id>snapshots</id>
                      <url>https://nexus.example.com/snapshots/</url>
                    </repository>
                  </repositories>
                </profile>
              </profiles>
            </settings>
            "#,
        )
        .unwrap();

        assert_eq!(settings.repositories.len(), 1);
        assert_eq!(settings.repositories[0].name, "snapshots");
    }

    #[test]
    fn ignores_inactive_profile_repositories() {
        let settings = MavenSettings::parse(
            r#"
            <settings>
              <profiles>
                <profile>
                  <id>opt-in</id>
                  <repositories>
                    <repository>
                      <id>extra</id>
                      <url>https://nexus.example.com/extra/</url>
                    </repository>
                  </repositories>
                </profile>
              </profiles>
            </settings>
            "#,
        )
        .unwrap();

        assert!(settings.repositories.is_empty());
    }

    #[test]
    fn deduplicates_repositories_by_id_across_active_profiles() {
        let settings = MavenSettings::parse(
            r#"
            <settings>
              <activeProfiles>
                <activeProfile>first</activeProfile>
                <activeProfile>second</activeProfile>
              </activeProfiles>
              <profiles>
                <profile>
                  <id>first</id>
                  <repositories>
                    <repository>
                      <id>shared</id>
                      <url>https://first.example.com/maven/</url>
                    </repository>
                  </repositories>
                </profile>
                <profile>
                  <id>second</id>
                  <repositories>
                    <repository>
                      <id>shared</id>
                      <url>https://second.example.com/maven/</url>
                    </repository>
                  </repositories>
                </profile>
              </profiles>
            </settings>
            "#,
        )
        .unwrap();

        assert_eq!(settings.repositories.len(), 1);
        assert_eq!(
            settings.repositories[0].url,
            "https://first.example.com/maven"
        );
    }

    #[test]
    fn skips_repository_with_missing_id_or_url() {
        let settings = MavenSettings::parse(
            r#"
            <settings>
              <profiles>
                <profile>
                  <id>default</id>
                  <activation>
                    <activeByDefault>true</activeByDefault>
                  </activation>
                  <repositories>
                    <repository>
                      <url>https://no-id.example.com/maven/</url>
                    </repository>
                    <repository>
                      <id>no-url</id>
                    </repository>
                    <repository>
                      <id>ok</id>
                      <url>https://ok.example.com/maven/</url>
                    </repository>
                  </repositories>
                </profile>
              </profiles>
            </settings>
            "#,
        )
        .unwrap();

        assert_eq!(settings.repositories.len(), 1);
        assert_eq!(settings.repositories[0].name, "ok");
    }

    #[test]
    fn load_or_default_returns_default_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.xml");
        let settings = MavenSettings::load_or_default(&path).unwrap();
        assert!(settings.local_repository.is_none());
        assert!(settings.repositories.is_empty());
    }

    #[test]
    fn settings_path_lives_under_home_m2() {
        assert!(settings_path().ends_with(Path::new(".m2").join("settings.xml")));
    }

    #[test]
    fn parses_mirrors_from_settings() {
        let settings = MavenSettings::parse(
            r#"
            <settings>
              <mirrors>
                <mirror>
                  <id>my-mirror</id>
                  <url>https://mirror.example.com/maven/</url>
                  <mirrorOf>central</mirrorOf>
                </mirror>
              </mirrors>
            </settings>
            "#,
        )
        .unwrap();

        assert_eq!(settings.mirrors.len(), 1);
        assert_eq!(settings.mirrors[0].id, "my-mirror");
        assert_eq!(settings.mirrors[0].url, "https://mirror.example.com/maven");
        assert_eq!(settings.mirrors[0].mirror_of, "central");
    }

    #[test]
    fn skips_mirror_with_missing_fields() {
        let settings = MavenSettings::parse(
            r#"
            <settings>
              <mirrors>
                <mirror>
                  <url>https://mirror.example.com/maven/</url>
                  <mirrorOf>central</mirrorOf>
                </mirror>
                <mirror>
                  <id>no-url</id>
                  <mirrorOf>central</mirrorOf>
                </mirror>
                <mirror>
                  <id>no-mirror-of</id>
                  <url>https://mirror.example.com/maven/</url>
                </mirror>
                <mirror>
                  <id>ok</id>
                  <url>https://ok.example.com/maven/</url>
                  <mirrorOf>*</mirrorOf>
                </mirror>
              </mirrors>
            </settings>
            "#,
        )
        .unwrap();

        assert_eq!(settings.mirrors.len(), 1);
        assert_eq!(settings.mirrors[0].id, "ok");
    }

    #[test]
    fn deduplicates_mirrors_by_id() {
        let settings = MavenSettings::parse(
            r#"
            <settings>
              <mirrors>
                <mirror>
                  <id>shared</id>
                  <url>https://first.example.com/maven/</url>
                  <mirrorOf>central</mirrorOf>
                </mirror>
                <mirror>
                  <id>shared</id>
                  <url>https://second.example.com/maven/</url>
                  <mirrorOf>*</mirrorOf>
                </mirror>
              </mirrors>
            </settings>
            "#,
        )
        .unwrap();

        assert_eq!(settings.mirrors.len(), 1);
        assert_eq!(settings.mirrors[0].url, "https://first.example.com/maven");
    }

    #[test]
    fn mirror_matches_specific_repo_name() {
        let mirror = Mirror {
            id: "m".to_string(),
            url: "https://mirror.example.com".to_string(),
            mirror_of: "central".to_string(),
        };

        assert!(mirror.matches("central"));
        assert!(!mirror.matches("internal"));
    }

    #[test]
    fn mirror_matches_wildcard() {
        let mirror = Mirror {
            id: "m".to_string(),
            url: "https://mirror.example.com".to_string(),
            mirror_of: "*".to_string(),
        };

        assert!(mirror.matches("central"));
        assert!(mirror.matches("internal"));
        assert!(mirror.matches("anything"));
    }

    #[test]
    fn mirror_matches_comma_separated_names() {
        let mirror = Mirror {
            id: "m".to_string(),
            url: "https://mirror.example.com".to_string(),
            mirror_of: "central,internal".to_string(),
        };

        assert!(mirror.matches("central"));
        assert!(mirror.matches("internal"));
        assert!(!mirror.matches("other"));
    }

    #[test]
    fn mirror_negation_excludes_repo() {
        let mirror = Mirror {
            id: "m".to_string(),
            url: "https://mirror.example.com".to_string(),
            mirror_of: "*,!internal".to_string(),
        };

        assert!(mirror.matches("central"));
        assert!(!mirror.matches("internal"));
    }

    #[test]
    fn mirror_negation_only_excludes_when_positive_matches() {
        let mirror = Mirror {
            id: "m".to_string(),
            url: "https://mirror.example.com".to_string(),
            mirror_of: "!internal".to_string(),
        };

        assert!(!mirror.matches("central"));
        assert!(!mirror.matches("internal"));
    }

    #[test]
    fn apply_mirrors_rewrites_matching_repository_url() {
        let settings = MavenSettings::parse(
            r#"
            <settings>
              <mirrors>
                <mirror>
                  <id>my-mirror</id>
                  <url>https://mirror.example.com/maven/</url>
                  <mirrorOf>central</mirrorOf>
                </mirror>
              </mirrors>
            </settings>
            "#,
        )
        .unwrap();

        let mut repos = vec![
            Repository::new("central", "https://repo1.maven.org/maven2/"),
            Repository::new("internal", "https://nexus.example.com/maven/"),
        ];

        settings.apply_mirrors(&mut repos);

        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].name, "my-mirror");
        assert_eq!(repos[0].url, "https://mirror.example.com/maven");
        assert_eq!(repos[1].name, "internal");
        assert_eq!(repos[1].url, "https://nexus.example.com/maven");
    }

    #[test]
    fn apply_mirrors_wildcard_deduplicates_to_single_repo() {
        let settings = MavenSettings::parse(
            r#"
            <settings>
              <mirrors>
                <mirror>
                  <id>my-mirror</id>
                  <url>https://mirror.example.com/maven/</url>
                  <mirrorOf>*</mirrorOf>
                </mirror>
              </mirrors>
            </settings>
            "#,
        )
        .unwrap();

        let mut repos = vec![
            Repository::new("central", "https://repo1.maven.org/maven2/"),
            Repository::new("internal", "https://nexus.example.com/maven/"),
            Repository::new("other", "https://other.example.com/maven/"),
        ];

        settings.apply_mirrors(&mut repos);

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "my-mirror");
        assert_eq!(repos[0].url, "https://mirror.example.com/maven");
    }

    #[test]
    fn apply_mirrors_negation_preserves_excluded_repo() {
        let settings = MavenSettings::parse(
            r#"
            <settings>
              <mirrors>
                <mirror>
                  <id>my-mirror</id>
                  <url>https://mirror.example.com/maven/</url>
                  <mirrorOf>*,!internal</mirrorOf>
                </mirror>
              </mirrors>
            </settings>
            "#,
        )
        .unwrap();

        let mut repos = vec![
            Repository::new("central", "https://repo1.maven.org/maven2/"),
            Repository::new("internal", "https://nexus.example.com/maven/"),
        ];

        settings.apply_mirrors(&mut repos);

        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].name, "my-mirror");
        assert_eq!(repos[0].url, "https://mirror.example.com/maven");
        assert_eq!(repos[1].name, "internal");
        assert_eq!(repos[1].url, "https://nexus.example.com/maven");
    }

    #[test]
    fn apply_mirrors_first_match_wins() {
        let settings = MavenSettings::parse(
            r#"
            <settings>
              <mirrors>
                <mirror>
                  <id>first-mirror</id>
                  <url>https://first.example.com/maven/</url>
                  <mirrorOf>central</mirrorOf>
                </mirror>
                <mirror>
                  <id>second-mirror</id>
                  <url>https://second.example.com/maven/</url>
                  <mirrorOf>*</mirrorOf>
                </mirror>
              </mirrors>
            </settings>
            "#,
        )
        .unwrap();

        let mut repos = vec![
            Repository::new("central", "https://repo1.maven.org/maven2/"),
            Repository::new("other", "https://other.example.com/maven/"),
        ];

        settings.apply_mirrors(&mut repos);

        let central = repos.iter().find(|r| r.name == "first-mirror").unwrap();
        assert_eq!(central.url, "https://first.example.com/maven");
        let other = repos.iter().find(|r| r.name == "second-mirror").unwrap();
        assert_eq!(other.url, "https://second.example.com/maven");
    }

    #[test]
    fn apply_mirrors_noop_when_no_mirrors() {
        let settings = MavenSettings::default();
        let mut repos = vec![Repository::new(
            "central",
            "https://repo1.maven.org/maven2/",
        )];
        settings.apply_mirrors(&mut repos);
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "central");
    }
}
