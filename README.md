# Angra

Angra is a fast, Maven-compatible Java project tool.

The goal is to bring `uv`-style ergonomics to Java: a small TOML manifest,
minimal ceremony, fast dependency resolution, and compatibility with the Maven
artifact ecosystem.

Angra is very early. The current MVP focuses on dependency resolution and
manifest lifecycle commands.

## What Works Today

- `angra init`
- `angra import-pom`
- `angra add` / `angra remove`
- `angra lock` / `angra resolve`
- `angra resolve --frozen` — lockfile-authoritative installs with manifest drift detection and SHA-256 verification
- `angra tree` / `angra why`
- `angra outdated` — report direct dependencies with newer versions available
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
  native = { group = "com.example", artifact = "native-lib", version = "1.0.0", type = "jar", classifier = "linux-aarch64" }
  webapp = { group = "com.example", artifact = "app", version = "1.0.0", type = "war" }
  ```

- Maven local repository layout under `~/.m2/repository`
- Maven Central downloads
- Project-local repository declarations in `angra.toml`
- Deterministic TOML lockfile generation via `angra.lock`
- Runtime dependency graph resolution for compile/runtime scopes
- Current-POM and inherited parent property interpolation
- Parent POM inheritance for repository-resolved parents
- Local parent POM lookup through `<relativePath>` before repository fallback
- Dependency management and BOM imports
- Angra-native `[dependency-management]` BOM imports and managed versions
- Maven profile activation and injection for resolver-relevant POM sections
- Maven version ranges resolved from `maven-metadata.xml`
- Timestamped SNAPSHOT resolution from Maven metadata
- `jar`, `pom`, and `war` dependency artifact types
- Optional classifiers in structured dependencies and transitive POM dependencies
- SHA-1 checksum verification with repository `checksumPolicy` support
- Parallel same-depth artifact fetching during resolution
- Optional dependency filtering
- Exclusions
- Basic nearest-wins conflict behavior
- Colorized resolver errors with dependency paths for failed transitive artifacts
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

[repositories]
central = "https://repo1.maven.org/maven2"
snapshots = { url = "https://repo.example.com/snapshots", releases = false, snapshots = true, checksum-policy = "warn" }

[dependency-management]
spring = { group = "org.springframework.boot", artifact = "spring-boot-dependencies", version = "4.0.6", type = "pom", scope = "import" }

[dependencies]
slf4j = "org.slf4j:slf4j-api:2.0.13"
guava = { group = "com.google.guava", artifact = "guava", version = "33.0.0-jre" }
```

Optional Maven profile controls:

```toml
[resolver.maven]
active-profiles = ["dev"]
inactive-profiles = ["legacy"]
java-version = "21.0.2"

[resolver.maven.properties]
environment = "test"
```

Resolve dependencies:

```sh
angra lock
```

Resolve from local cache only:

```sh
angra resolve --offline
```

Force a remote re-check:

```sh
angra resolve --refresh
```

Install exactly what `angra.lock` records, without re-resolving (CI-friendly —
fails if the manifest drifted from the lockfile or an artifact does not match
its locked SHA-256):

```sh
angra resolve --frozen
```

Start from an existing Maven project:

```sh
angra import-pom pom.xml
```

Add and inspect dependencies without hand-editing TOML:

```sh
angra add com.google.guava:guava:33.0.0-jre
angra add junit:junit:4.13.2 --scope test
angra tree
angra why com.google.guava:guava
```

Check for newer versions of direct dependencies (read-only; version ranges
and SNAPSHOTs are skipped with a warning since they update through `angra lock`):

```sh
angra outdated
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

- Live `pom.xml` execution as a project manifest. `angra import-pom` is a one-way, lossy migration command.
- Private repositories
- Authentication
- Unknown Maven artifact types beyond `jar`, `pom`, and `war`
- The long tail of Maven profile/plugin/build-model behavior outside resolver-relevant dependencies, dependency management, properties, and repositories

Unsupported runtime dependency properties fail clearly instead of being guessed.

## License

Angra is licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE).
