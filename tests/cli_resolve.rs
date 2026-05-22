use std::{fs, process::Command};

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
