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
