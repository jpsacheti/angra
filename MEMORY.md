# MEMORY.md

## 2026-05-20 - Project Guidance

### What was decided

- Maintain `AGENTS.md` as the project instruction file for agent behavior.
- Maintain `MEMORY.md` as the durable project memory.
- Use the project context that `angra` aims to be a Java equivalent of `uv`, with full Maven compatibility, TOML-based project management, minimal overhead, and developer joy as a core design constraint.

### Why

- The project is brand new, so establishing behavior, decision logging, and product direction early keeps future work consistent.
- Maven compatibility and low overhead are foundational constraints, not incidental preferences.

### What was rejected and why

- Proceeding without durable memory was rejected because future sessions need to preserve decisions and avoid contradicting earlier project direction.
- Treating TOML support as separate from Maven compatibility was rejected because the intended direction is TOML ergonomics while remaining compatible with `pom.xml`.

## 2026-05-21 - MVP Resolver And Benchmark Direction

### What was decided

- Implement the first MVP around `angra resolve`.
- Read `angra.toml`, resolve Maven-compatible dependencies from `~/.m2/repository` and Maven Central, cache artifacts in Maven local repository layout, and write deterministic TOML `angra.lock`.
- Support both compact dependency syntax (`group:artifact:version`) and structured dependency syntax (`group`, `artifact`, `version`, `scope`, `exclusions`).
- Add benchmark infrastructure from the start, comparing Angra with Maven and Gradle for comparable dependency-resolution workloads.
- Use `mise` to run Maven and Gradle benchmarks with dynamic latest tool versions.

### Why

- Dependency resolution is the smallest useful slice that proves Angra's core value: Maven compatibility with less ceremony.
- Benchmarks need to exist alongside features so speed remains a product constraint, not a late-stage cleanup item.
- Dynamic `mise` versions make the benchmark workflow easy to keep current for local comparisons.

### What was rejected and why

- Shelling out to Maven for Angra resolution was rejected because it weakens the product premise and hides performance characteristics.
- Starting with `pom.xml` ingestion was rejected because the first compatibility target is Maven artifact/POM resolution behavior.
- Pinning Maven and Gradle versions in repo config was rejected in favor of dynamic latest versions through `mise`.

## 2026-05-21 - Session Summary

### Worked on

- Bootstrapped Angra as a Rust CLI project for Maven-compatible Java dependency resolution.
- Added project instructions, durable memory, documentation, contribution guidance, and Apache-2.0 licensing.

### Completed

- Implemented `angra resolve`.
- Implemented `angra.toml` dependency parsing for compact and structured declarations.
- Implemented Maven coordinate handling, local repository layout, Maven Central downloads, runtime graph resolution, optional dependency filtering, exclusions, basic nearest-wins conflict behavior, and deterministic TOML `angra.lock` generation.
- Added benchmark fixtures and an `angra-bench` runner comparing Angra with Maven and Gradle through `mise`.
- Added benchmark summary output showing how many times faster Angra is than Maven and Gradle.
- Added `README.md`, `CONTRIBUTING.md`, and `LICENSE`.

### In progress

- GitHub publication is pending a successful remote setup and push.
- Parent POM inheritance, BOM imports, private repositories, mirrors, authentication, version ranges, and profile handling remain unsupported.

### Decisions made

- Use Apache-2.0 for the project license.
- Keep benchmark output human-readable first, followed by raw JSON details.
- Use dynamic latest Maven and Gradle versions via `mise` for local benchmark comparisons.

### Next session priorities

- Confirm GitHub remote and publish the initial repository if not completed.
- Decide the next Maven compatibility slice: parent POM inheritance, BOM import support, or `pom.xml` ingestion.
- Add more realistic benchmark fixtures as resolver compatibility expands.
