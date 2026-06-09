use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
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
            gradle_args: Some(vec![
                "--no-daemon".to_string(),
                "dependencies".to_string(),
                "--configuration".to_string(),
                "runtimeClasspath".to_string(),
            ]),
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
    eprintln!(
        "benchmark: starting {case}/{tool}: {} {}",
        program.display(),
        args.join(" ")
    );

    let start = Instant::now();
    let mut child = Command::new(program)
        .args(args)
        .current_dir(directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut next_progress_at = Duration::from_secs(5);
    loop {
        if child.try_wait()?.is_some() {
            break;
        }

        let elapsed = start.elapsed();
        if elapsed >= next_progress_at {
            eprintln!(
                "benchmark: still running {case}/{tool} after {:.1}s",
                elapsed.as_secs_f64()
            );
            next_progress_at += Duration::from_secs(5);
        }

        thread::sleep(Duration::from_millis(250));
    }

    let output = child.wait_with_output()?;
    let duration = start.elapsed();
    let status = output.status.code().unwrap_or(-1);

    if status == 0 {
        eprintln!(
            "benchmark: finished {case}/{tool} in {:.1}s",
            duration.as_secs_f64()
        );
    } else {
        eprintln!(
            "benchmark: failed {case}/{tool} with status {status} after {:.1}s",
            duration.as_secs_f64()
        );
    }

    Ok(BenchmarkResult {
        tool,
        case: case.to_string(),
        status,
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

impl std::fmt::Display for BenchmarkTool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Angra => "angra",
            Self::Maven => "maven",
            Self::Gradle => "gradle",
        };
        formatter.write_str(name)
    }
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

    #[test]
    fn formats_benchmark_tool_names() {
        assert_eq!(BenchmarkTool::Angra.to_string(), "angra");
        assert_eq!(BenchmarkTool::Maven.to_string(), "maven");
        assert_eq!(BenchmarkTool::Gradle.to_string(), "gradle");
    }
}
