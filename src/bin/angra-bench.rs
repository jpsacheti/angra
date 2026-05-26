use std::path::PathBuf;

use angra::benchmark::{BenchmarkResult, BenchmarkTool, fixture_cases, run_case};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "angra-bench")]
#[command(about = "Run Angra dependency-resolution benchmarks against Maven and Gradle")]
struct Cli {
    /// Repository root containing benches/fixtures.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Path to the angra binary to benchmark.
    #[arg(long, default_value = "target/debug/angra")]
    angra_binary: PathBuf,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let angra_binary = cli.angra_binary.canonicalize()?;
    let mut results = Vec::new();

    for case in fixture_cases(&cli.repo) {
        results.extend(run_case(&case, &angra_binary)?);
    }

    print_summary(&results);
    println!();
    println!("Raw results:");
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}

fn print_summary(results: &[BenchmarkResult]) {
    let mut cases = results
        .iter()
        .map(|result| result.case.as_str())
        .collect::<Vec<_>>();
    cases.sort_unstable();
    cases.dedup();

    println!("Benchmark summary:");
    println!(
        "{:<12} {:>12} {:>12} {:>12} {:>18} {:>18}",
        "case", "angra", "maven", "gradle", "vs maven", "vs gradle"
    );

    for case in cases {
        let angra = duration_ms(results, case, BenchmarkTool::Angra);
        let maven = duration_ms(results, case, BenchmarkTool::Maven);
        let gradle = duration_ms(results, case, BenchmarkTool::Gradle);

        println!(
            "{:<12} {:>12} {:>12} {:>12} {:>18} {:>18}",
            case,
            format_result_duration(results, case, BenchmarkTool::Angra),
            format_result_duration(results, case, BenchmarkTool::Maven),
            format_result_duration(results, case, BenchmarkTool::Gradle),
            format_speedup(angra, maven),
            format_speedup(angra, gradle)
        );
    }
}

fn duration_ms(results: &[BenchmarkResult], case: &str, tool: BenchmarkTool) -> Option<u128> {
    results
        .iter()
        .find(|result| result.case == case && result.tool == tool && result.status == 0)
        .map(|result| result.duration_ms)
}

fn format_result_duration(results: &[BenchmarkResult], case: &str, tool: BenchmarkTool) -> String {
    match results
        .iter()
        .find(|result| result.case == case && result.tool == tool)
    {
        Some(result) if result.status == 0 => format!("{} ms", result.duration_ms),
        Some(_) => "failed".to_string(),
        None => "n/a".to_string(),
    }
}

fn format_speedup(angra_ms: Option<u128>, other_ms: Option<u128>) -> String {
    match (angra_ms, other_ms) {
        (Some(0), Some(_)) => "too fast".to_string(),
        (Some(angra), Some(other)) => format!("{:.1}x faster", other as f64 / angra as f64),
        _ => "n/a".to_string(),
    }
}
