use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    thread,
};

use sha1::{Digest, Sha1};
use tempfile::TempDir;

fn write_artifact(repo: &std::path::Path, group: &str, artifact: &str, version: &str, pom: &str) {
    let dir = repo
        .join(group.replace('.', "/"))
        .join(artifact)
        .join(version);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{artifact}-{version}.pom")), pom).unwrap();
    fs::write(
        dir.join(format!("{artifact}-{version}.jar")),
        format!("jar for {group}:{artifact}:{version}"),
    )
    .unwrap();
}

fn write_classified_artifact(
    repo: &std::path::Path,
    group: &str,
    artifact: &str,
    version: &str,
    classifier: &str,
    pom: &str,
) {
    let dir = repo
        .join(group.replace('.', "/"))
        .join(artifact)
        .join(version);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{artifact}-{version}.pom")), pom).unwrap();
    fs::write(
        dir.join(format!("{artifact}-{version}-{classifier}.jar")),
        format!("jar for {group}:{artifact}:{version}:{classifier}"),
    )
    .unwrap();
}

fn write_remote_artifact(
    repo: &std::path::Path,
    group: &str,
    artifact: &str,
    version: &str,
    pom: &str,
) {
    let dir = repo
        .join(group.replace('.', "/"))
        .join(artifact)
        .join(version);
    fs::create_dir_all(&dir).unwrap();
    write_remote_file(
        &dir.join(format!("{artifact}-{version}.pom")),
        pom.as_bytes(),
    );
    write_remote_file(
        &dir.join(format!("{artifact}-{version}.jar")),
        format!("jar for {group}:{artifact}:{version}").as_bytes(),
    );
}

fn write_remote_file(path: &std::path::Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    fs::write(
        path.with_extension(format!(
            "{}.sha1",
            path.extension().unwrap().to_string_lossy()
        )),
        hex_bytes(&hasher.finalize()),
    )
    .unwrap();
}

fn write_remote_file_with_sha1(path: &std::path::Path, bytes: &[u8], sha1: &str) {
    fs::write(path, bytes).unwrap();
    fs::write(
        path.with_extension(format!(
            "{}.sha1",
            path.extension().unwrap().to_string_lossy()
        )),
        sha1,
    )
    .unwrap();
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

fn serve_directory(root: std::path::PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let mut buffer = [0; 2048];
            let Ok(read) = stream.read(&mut buffer) else {
                continue;
            };
            let request = String::from_utf8_lossy(&buffer[..read]);
            let Some(path) = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
            else {
                continue;
            };
            let relative = path.trim_start_matches('/');
            let file = root.join(relative);
            if let Ok(bytes) = fs::read(file) {
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                )
                .unwrap();
                stream.write_all(&bytes).unwrap();
            } else {
                stream
                    .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            }
        }
    });

    format!("http://{address}")
}

#[test]
fn resolve_creates_stable_lockfile_offline() {
    let project = TempDir::new().unwrap();
    let local_repo = project.path().join(".m2").join("repository");
    write_artifact(&local_repo, "com.example", "demo", "1.0.0", "<project/>");
    fs::write(
        project.path().join("angra.toml"),
        r#"
        [dependencies]
        demo = "com.example:demo:1.0.0"
        "#,
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_angra");
    let first = Command::new(binary)
        .args([
            "resolve",
            "--offline",
            "--project-dir",
            project.path().to_str().unwrap(),
        ])
        .env("HOME", project.path())
        .output()
        .unwrap();

    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );

    let lockfile = project.path().join("angra.lock");
    let first_lock = fs::read_to_string(&lockfile).unwrap();

    let second = Command::new(binary)
        .args([
            "resolve",
            "--offline",
            "--project-dir",
            project.path().to_str().unwrap(),
        ])
        .env("HOME", project.path())
        .output()
        .unwrap();

    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(first_lock, fs::read_to_string(lockfile).unwrap());
}

#[test]
fn resolve_interpolates_pom_properties_offline() {
    let project = TempDir::new().unwrap();
    let local_repo = project.path().join(".m2").join("repository");
    write_artifact(
        &local_repo,
        "com.example",
        "root",
        "1.0.0",
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
    write_artifact(&local_repo, "com.example", "child", "1.0.0", "<project/>");
    fs::write(
        project.path().join("angra.toml"),
        r#"
        [dependencies]
        root = "com.example:root:1.0.0"
        "#,
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_angra");
    let output = Command::new(binary)
        .args([
            "resolve",
            "--offline",
            "--project-dir",
            project.path().to_str().unwrap(),
        ])
        .env("HOME", project.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let lockfile = fs::read_to_string(project.path().join("angra.lock")).unwrap();
    assert!(lockfile.contains(r#"group = "com.example""#));
    assert!(lockfile.contains(r#"artifact = "child""#));
    assert!(lockfile.contains(r#"version = "1.0.0""#));
}

#[test]
fn resolve_structured_classifier_dependency_offline() {
    let project = TempDir::new().unwrap();
    let local_repo = project.path().join(".m2").join("repository");
    write_classified_artifact(
        &local_repo,
        "com.example",
        "native",
        "1.0.0",
        "linux-aarch64",
        "<project/>",
    );
    fs::write(
        project.path().join("angra.toml"),
        r#"
        [dependencies]
        native = { group = "com.example", artifact = "native", version = "1.0.0", type = "jar", classifier = "linux-aarch64" }
        "#,
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_angra");
    let output = Command::new(binary)
        .args([
            "resolve",
            "--offline",
            "--project-dir",
            project.path().to_str().unwrap(),
        ])
        .env("HOME", project.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let lockfile = fs::read_to_string(project.path().join("angra.lock")).unwrap();
    assert!(lockfile.contains(r#"type = "jar""#));
    assert!(lockfile.contains(r#"classifier = "linux-aarch64""#));
    assert!(lockfile.contains("native-1.0.0-linux-aarch64.jar"));
    assert!(lockfile.contains("artifact_path"));
    assert!(!lockfile.contains("jar_path"));
}

#[test]
fn resolve_failure_prints_colored_dependency_path() {
    let project = TempDir::new().unwrap();
    let local_repo = project.path().join(".m2").join("repository");
    write_artifact(
        &local_repo,
        "com.example",
        "root",
        "1.0.0",
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
    fs::write(
        project.path().join("angra.toml"),
        r#"
        [dependencies]
        root = "com.example:root:1.0.0"
        "#,
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_angra");
    let output = Command::new(binary)
        .args([
            "resolve",
            "--offline",
            "--project-dir",
            project.path().to_str().unwrap(),
        ])
        .env("HOME", project.path())
        .output()
        .unwrap();

    assert!(!output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("\x1b["));
    assert!(stderr.contains("dependency path:"));
    assert!(stderr.contains("com.example:root:1.0.0"));
    assert!(stderr.contains("com.example:missing:1.0.0"));
    assert!(stderr.contains("->"));
}

#[test]
fn resolve_uses_repository_from_maven_settings() {
    let project = TempDir::new().unwrap();
    let remote = TempDir::new().unwrap();
    write_remote_artifact(remote.path(), "com.example", "demo", "1.0.0", "<project/>");
    let repository_url = serve_directory(remote.path().to_path_buf());

    fs::write(
        project.path().join("angra.toml"),
        r#"
        [dependencies]
        demo = "com.example:demo:1.0.0"
        "#,
    )
    .unwrap();

    let settings_dir = project.path().join(".m2");
    fs::create_dir_all(&settings_dir).unwrap();
    fs::write(
        settings_dir.join("settings.xml"),
        format!(
            r#"<settings>
              <profiles>
                <profile>
                  <id>default</id>
                  <activation><activeByDefault>true</activeByDefault></activation>
                  <repositories>
                    <repository>
                      <id>internal</id>
                      <url>{repository_url}</url>
                    </repository>
                  </repositories>
                </profile>
              </profiles>
            </settings>"#
        ),
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_angra");
    let output = Command::new(binary)
        .args(["resolve", "--project-dir", project.path().to_str().unwrap()])
        .env("HOME", project.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let lockfile = fs::read_to_string(project.path().join("angra.lock")).unwrap();
    assert!(lockfile.contains(r#"source = "internal""#));
    assert!(lockfile.contains("demo-1.0.0.jar"));
}

#[test]
fn resolve_uses_local_repository_from_maven_settings() {
    let project = TempDir::new().unwrap();
    let custom_repo = project.path().join("custom-m2");
    write_artifact(&custom_repo, "com.example", "demo", "1.0.0", "<project/>");

    fs::write(
        project.path().join("angra.toml"),
        r#"
        [dependencies]
        demo = "com.example:demo:1.0.0"
        "#,
    )
    .unwrap();

    let settings_dir = project.path().join(".m2");
    fs::create_dir_all(&settings_dir).unwrap();
    fs::write(
        settings_dir.join("settings.xml"),
        format!(
            r#"<settings>
              <localRepository>{}</localRepository>
            </settings>"#,
            custom_repo.display()
        ),
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_angra");
    let output = Command::new(binary)
        .args([
            "resolve",
            "--offline",
            "--project-dir",
            project.path().to_str().unwrap(),
        ])
        .env("HOME", project.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let lockfile = fs::read_to_string(project.path().join("angra.lock")).unwrap();
    let custom_path = custom_repo.display().to_string();
    assert!(
        lockfile.contains(&custom_path),
        "expected lockfile to reference custom local repo {custom_path}, got:\n{lockfile}"
    );
}

#[test]
fn resolve_uses_project_repositories() {
    let project = TempDir::new().unwrap();
    let remote = TempDir::new().unwrap();
    write_remote_artifact(remote.path(), "com.example", "demo", "1.0.0", "<project/>");
    let repository_url = serve_directory(remote.path().to_path_buf());

    fs::write(
        project.path().join("angra.toml"),
        format!(
            r#"
            [repositories]
            local = "{repository_url}"

            [dependencies]
            demo = "com.example:demo:1.0.0"
            "#
        ),
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_angra");
    let output = Command::new(binary)
        .args(["resolve", "--project-dir", project.path().to_str().unwrap()])
        .env("HOME", project.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let lockfile = fs::read_to_string(project.path().join("angra.lock")).unwrap();
    assert!(lockfile.contains(r#"source = "local""#));
    assert!(lockfile.contains("demo-1.0.0.jar"));
}

#[test]
fn resolve_locks_requested_version_for_remote_range() {
    let project = TempDir::new().unwrap();
    let remote = TempDir::new().unwrap();
    let metadata_dir = remote.path().join("com/example/lib");
    fs::create_dir_all(&metadata_dir).unwrap();
    write_remote_file(
        &metadata_dir.join("maven-metadata.xml"),
        br#"
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
    write_remote_artifact(remote.path(), "com.example", "lib", "1.5.0", "<project/>");
    let repository_url = serve_directory(remote.path().to_path_buf());

    fs::write(
        project.path().join("angra.toml"),
        format!(
            r#"
            [repositories]
            local = "{repository_url}"

            [dependencies]
            lib = "com.example:lib:[1.0,2.0)"
            "#
        ),
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_angra");
    let output = Command::new(binary)
        .args(["resolve", "--project-dir", project.path().to_str().unwrap()])
        .env("HOME", project.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let lockfile = fs::read_to_string(project.path().join("angra.lock")).unwrap();
    assert!(lockfile.contains(r#"version = "1.5.0""#));
    assert!(lockfile.contains(r#"requested_version = "[1.0,2.0)""#));
}

#[test]
fn resolve_locks_requested_version_for_timestamped_snapshot() {
    let project = TempDir::new().unwrap();
    let remote = TempDir::new().unwrap();
    let snapshot_dir = remote.path().join("com/example/lib/1.0-SNAPSHOT");
    fs::create_dir_all(&snapshot_dir).unwrap();
    write_remote_file(
        &snapshot_dir.join("maven-metadata.xml"),
        br#"
        <metadata>
          <versioning>
            <snapshotVersions>
              <snapshotVersion>
                <extension>jar</extension>
                <value>1.0-20240501.120000-3</value>
                <updated>20240501120000</updated>
              </snapshotVersion>
              <snapshotVersion>
                <extension>pom</extension>
                <value>1.0-20240501.120000-3</value>
                <updated>20240501120000</updated>
              </snapshotVersion>
            </snapshotVersions>
          </versioning>
        </metadata>
        "#,
    );
    write_remote_file(
        &snapshot_dir.join("lib-1.0-20240501.120000-3.pom"),
        b"<project/>",
    );
    write_remote_file(
        &snapshot_dir.join("lib-1.0-20240501.120000-3.jar"),
        b"jar for timestamped snapshot",
    );
    let repository_url = serve_directory(remote.path().to_path_buf());

    fs::write(
        project.path().join("angra.toml"),
        format!(
            r#"
            [repositories]
            snapshots = "{repository_url}"

            [dependencies]
            lib = "com.example:lib:1.0-SNAPSHOT"
            "#
        ),
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_angra");
    let output = Command::new(binary)
        .args(["resolve", "--project-dir", project.path().to_str().unwrap()])
        .env("HOME", project.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let lockfile = fs::read_to_string(project.path().join("angra.lock")).unwrap();
    assert!(lockfile.contains(r#"version = "1.0-20240501.120000-3""#));
    assert!(lockfile.contains(r#"requested_version = "1.0-SNAPSHOT""#));
}

#[test]
fn resolve_warns_but_succeeds_for_checksum_warn_policy() {
    let project = TempDir::new().unwrap();
    let remote = TempDir::new().unwrap();
    let repository_url = serve_directory(remote.path().to_path_buf());

    let local_repo = project.path().join(".m2").join("repository");
    write_artifact(
        &local_repo,
        "com.example",
        "root",
        "1.0.0",
        &format!(
            r#"
            <project>
              <repositories>
                <repository>
                  <id>warn-repo</id>
                  <url>{repository_url}</url>
                  <releases>
                    <checksumPolicy>warn</checksumPolicy>
                  </releases>
                </repository>
              </repositories>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>child</artifactId>
                  <version>1.0.0</version>
                </dependency>
              </dependencies>
            </project>
            "#
        ),
    );

    let child_dir = remote.path().join("com/example/child/1.0.0");
    fs::create_dir_all(&child_dir).unwrap();
    write_remote_file(&child_dir.join("child-1.0.0.pom"), b"<project/>");
    write_remote_file_with_sha1(
        &child_dir.join("child-1.0.0.jar"),
        b"jar with bad checksum",
        "0000000000000000000000000000000000000000",
    );

    fs::write(
        project.path().join("angra.toml"),
        r#"
        [dependencies]
        root = "com.example:root:1.0.0"
        "#,
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_angra");
    let output = Command::new(binary)
        .args(["resolve", "--project-dir", project.path().to_str().unwrap()])
        .env("HOME", project.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("warning:"));
    assert!(stderr.contains("checksum mismatch"));
}

#[test]
fn resolve_activates_profile_by_file_exists() {
    let project = TempDir::new().unwrap();
    let local_repo = project.path().join(".m2").join("repository");
    let marker = project.path().join("marker.txt");
    fs::write(&marker, "marker").unwrap();

    write_artifact(
        &local_repo,
        "com.example",
        "root",
        "1.0.0",
        r#"
        <project>
          <profiles>
            <profile>
              <id>file-profile</id>
              <activation>
                <file><exists>marker.txt</exists></file>
              </activation>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>child</artifactId>
                  <version>1.0.0</version>
                </dependency>
              </dependencies>
            </profile>
          </profiles>
        </project>
        "#,
    );
    write_artifact(&local_repo, "com.example", "child", "1.0.0", "<project/>");

    fs::write(
        project.path().join("angra.toml"),
        r#"
        [dependencies]
        root = "com.example:root:1.0.0"
        "#,
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_angra");
    let output = Command::new(binary)
        .args([
            "resolve",
            "--offline",
            "--project-dir",
            project.path().to_str().unwrap(),
        ])
        .env("HOME", project.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let lockfile = fs::read_to_string(project.path().join("angra.lock")).unwrap();
    assert!(lockfile.contains("child-1.0.0.jar"));
}

#[test]
fn resolve_mirror_preserves_checksum_warn_policy() {
    let project = TempDir::new().unwrap();
    let remote = TempDir::new().unwrap();
    let repository_url = serve_directory(remote.path().to_path_buf());

    let local_repo = project.path().join(".m2").join("repository");
    write_artifact(
        &local_repo,
        "com.example",
        "root",
        "1.0.0",
        &format!(
            r#"
            <project>
              <repositories>
                <repository>
                  <id>warn-repo</id>
                  <url>http://invalid.example.com</url>
                  <releases>
                    <checksumPolicy>warn</checksumPolicy>
                  </releases>
                </repository>
              </repositories>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>child</artifactId>
                  <version>1.0.0</version>
                </dependency>
              </dependencies>
            </project>
            "#
        ),
    );

    let child_dir = remote.path().join("com/example/child/1.0.0");
    fs::create_dir_all(&child_dir).unwrap();
    write_remote_file(&child_dir.join("child-1.0.0.pom"), b"<project/>");
    write_remote_file_with_sha1(
        &child_dir.join("child-1.0.0.jar"),
        b"jar with bad checksum",
        "0000000000000000000000000000000000000000",
    );

    fs::write(
        project.path().join("angra.toml"),
        r#"
        [dependencies]
        root = "com.example:root:1.0.0"
        "#,
    )
    .unwrap();

    let settings_dir = project.path().join(".m2");
    fs::create_dir_all(&settings_dir).unwrap();
    fs::write(
        settings_dir.join("settings.xml"),
        format!(
            r#"<settings>
              <mirrors>
                <mirror>
                  <id>my-mirror</id>
                  <mirrorOf>*</mirrorOf>
                  <url>{repository_url}</url>
                </mirror>
              </mirrors>
            </settings>"#
        ),
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_angra");
    let output = Command::new(binary)
        .args(["resolve", "--project-dir", project.path().to_str().unwrap()])
        .env("HOME", project.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("warning:"));
    assert!(stderr.contains("checksum mismatch"));
}

#[test]
fn resolve_bom_dependency_management_in_profile() {
    let project = TempDir::new().unwrap();
    let local_repo = project.path().join(".m2").join("repository");
    write_artifact(
        &local_repo,
        "com.example",
        "bom",
        "1.0.0",
        r#"
        <project>
          <profiles>
            <profile>
              <id>default-profile</id>
              <activation><activeByDefault>true</activeByDefault></activation>
              <dependencyManagement>
                <dependencies>
                  <dependency>
                    <groupId>com.example</groupId>
                    <artifactId>child</artifactId>
                    <version>2.0.0</version>
                  </dependency>
                </dependencies>
              </dependencyManagement>
            </profile>
          </profiles>
        </project>
        "#,
    );
    write_artifact(
        &local_repo,
        "com.example",
        "root",
        "1.0.0",
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
    write_artifact(&local_repo, "com.example", "child", "2.0.0", "<project/>");

    fs::write(
        project.path().join("angra.toml"),
        r#"
        [dependencies]
        root = "com.example:root:1.0.0"
        "#,
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_angra");
    let output = Command::new(binary)
        .args([
            "resolve",
            "--offline",
            "--project-dir",
            project.path().to_str().unwrap(),
        ])
        .env("HOME", project.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lockfile = fs::read_to_string(project.path().join("angra.lock")).unwrap();
    assert!(lockfile.contains("child-2.0.0.jar"));
}
