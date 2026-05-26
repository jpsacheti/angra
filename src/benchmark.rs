use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use serde::Serialize;

#[derive(Debug, Clone)]
pub struct BenchmarkCase {
    pub name: String,
    pub directory: PathBuf,
    pub angra_args: Vec<String>,
    pub maven_args: Vec<String>,
    pub gradle_args: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BenchmarkResult {
    pub tool: BenchmarkTool,
    pub case: String,
    pub status: i32,
    pub duration_ms: u128,
    #[serde(skip)]
    pub duration: Duration,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BenchmarkTool {
    Angra,
    Maven,
    Gradle,
}

pub fn fixture_cases(root: &Path) -> Vec<BenchmarkCase> {
    let mut cases = ["direct", "transitive", "conflict"]
        .into_iter()
        .map(|name| BenchmarkCase {
            name: name.to_string(),
            directory: root.join("benches").join("fixtures").join(name),
            angra_args: vec!["resolve".to_string()],
            maven_args: vec!["dependency:go-offline".to_string()],
            gradle_args: Some(vec![
                "--no-daemon".to_string(),
                "dependencies".to_string(),
                "--configuration".to_string(),
                "runtimeClasspath".to_string(),
            ]),
        })
        .collect::<Vec<_>>();

    let spring_fixture = root.join("benches").join("spring-fixture");
    if spring_fixture.join("angra.toml").exists() && spring_fixture.join("pom.xml").exists() {
        cases.push(BenchmarkCase {
            name: "spring-fixture".to_string(),
            directory: spring_fixture,
            angra_args: vec!["resolve".to_string()],
            maven_args: vec![
                "dependency:list".to_string(),
                "-DincludeScope=runtime".to_string(),
                "-DoutputFile=/private/tmp/angra-spring-benchmark-runtime-deps.txt".to_string(),
            ],
            gradle_args: None,
        });
    }

    cases
}

pub fn run_case(
    case: &BenchmarkCase,
    angra_binary: &Path,
) -> Result<Vec<BenchmarkResult>, BenchmarkError> {
    let mut results = vec![
        run_command(
            BenchmarkTool::Angra,
            &case.name,
            &case.directory,
            angra_binary,
            &case.angra_args,
        )?,
        run_command(
            BenchmarkTool::Maven,
            &case.name,
            &case.directory,
            Path::new("mise"),
            &mise_args("maven", "mvn", &case.maven_args),
        )?,
    ];

    if let Some(gradle_args) = &case.gradle_args {
        results.push(run_command(
            BenchmarkTool::Gradle,
            &case.name,
            &case.directory,
            Path::new("mise"),
            &mise_args("gradle", "gradle", gradle_args),
        )?);
    }

    Ok(results)
}

fn mise_args(tool: &str, command: &str, args: &[String]) -> Vec<String> {
    let mut mise_args = vec![
        "x".to_string(),
        format!("{tool}@latest"),
        "--".to_string(),
        command.to_string(),
    ];
    mise_args.extend(args.iter().cloned());
    mise_args
}

fn run_command(
    tool: BenchmarkTool,
    case: &str,
    directory: &Path,
    program: &Path,
    args: &[String],
) -> Result<BenchmarkResult, BenchmarkError> {
    let start = Instant::now();
    let output = Command::new(program)
        .args(args)
        .current_dir(directory)
        .output()?;
    let duration = start.elapsed();

    Ok(BenchmarkResult {
        tool,
        case: case.to_string(),
        status: output.status.code().unwrap_or(-1),
        duration_ms: duration.as_millis(),
        duration,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum BenchmarkError {
    #[error("failed to run benchmark command: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_mise_latest_command_args() {
        let args = mise_args("maven", "mvn", &["dependency:go-offline".to_string()]);

        assert_eq!(
            args,
            vec!["x", "maven@latest", "--", "mvn", "dependency:go-offline"]
        );
    }
}
