# Angra

Angra is a fast, Maven-compatible Java project tool.

The goal is to bring `uv`-style ergonomics to Java: a small TOML manifest,
minimal ceremony, fast dependency resolution, and compatibility with the Maven
artifact ecosystem.

Angra is very early. The current MVP focuses on dependency resolution.

## What Works Today

- `angra resolve`
- `angra.toml` as the project manifest
- Compact Maven coordinates:

  ```toml
  [dependencies]
  guava = "com.google.guava:guava:33.0.0-jre"
  ```

- Structured dependency declarations:

  ```toml
  [dependencies]
  jackson = { group = "com.fasterxml.jackson.core", artifact = "jackson-databind", version = "2.17.2", scope = "runtime" }
  ```

- Maven local repository layout under `~/.m2/repository`
- Maven Central downloads
- Deterministic TOML lockfile generation via `angra.lock`
- Runtime dependency graph resolution for compile/runtime scopes
- Optional dependency filtering
- Exclusions
- Basic nearest-wins conflict behavior
- Comparative benchmark harness against Maven and Gradle through `mise`

## Install From Source

```sh
cargo build
```

Run the local binary:

```sh
target/debug/angra --help
```

## Usage

Create an `angra.toml`:

```toml
[project]
group = "com.example"
artifact = "demo"
version = "0.1.0"

[dependencies]
slf4j = "org.slf4j:slf4j-api:2.0.13"
guava = { group = "com.google.guava", artifact = "guava", version = "33.0.0-jre" }
```

Resolve dependencies:

```sh
angra resolve
```

Resolve from local cache only:

```sh
angra resolve --offline
```

Force a remote re-check:

```sh
angra resolve --refresh
```

## Benchmarks

Angra includes a benchmark runner that compares dependency-resolution fixtures
against Maven and Gradle.

Maven and Gradle are executed through `mise` using dynamic latest versions:

```sh
cargo build
cargo run --bin angra-bench -- --repo . --angra-binary target/debug/angra
```

The benchmark output starts with a summary like:

```text
Benchmark summary:
case                angra        maven       gradle           vs maven          vs gradle
direct               7 ms      1834 ms      2253 ms      262.0x faster      321.9x faster
```

Raw JSON results are printed after the summary.

## Current Limitations

The MVP intentionally does not support every Maven feature yet. In particular:

- `pom.xml` ingestion as a project manifest
- Private repositories
- Mirrors
- Authentication
- BOM imports
- Version ranges
- Maven profiles
- Broad parent-POM inheritance
- Inherited dependency property interpolation

Unsupported runtime dependency properties fail clearly instead of being guessed.

## License

Angra is licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
