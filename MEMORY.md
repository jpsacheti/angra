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

## 2026-05-21 - Roadmap Established

### What was decided

- Persist the project roadmap as `ROADMAP.md` at the repo root, kept in sync by future sessions per the Update Protocol section in that file.
- Scope of the 0.1 → 1.0 path ends at build & run: 0.1 resolver MVP (shipped), 0.2 resolver realism, 0.3 manifest lifecycle, 0.4 compile & test, 0.5 package & run, 1.0 hardening.
- JDK toolchains are delegated to `mise` / SDKMan rather than managed internally through 1.0.
- Use versioned milestones with plain-text status tags (`[shipped]` / `[in progress]` / `[planned]` / `[deferred]`).
- Every milestone exits on four checks: tests + clippy green, fixture project working end-to-end, bench harness updated, and a dated entry in this file.

### Why

- A checked-in roadmap is discoverable by future sessions and by anyone reading the repo. An ephemeral plan file is not.
- Capping the roadmap at build & run keeps the 1.0 target finite. Publishing, IDE plugins, and built-in JDK management each carry enough complexity to deserve their own roadmap later.
- Delegating JDK management removes a large pillar of work and matches how Java developers already manage toolchains in 2026.

### What was rejected and why

- Full uv-equivalent scope (publish, JDK management, IDE) — rejected as too broad for a 1.0 target with finite execution.
- Built-in JDK download/management — rejected in favor of delegating to `mise` / SDKMan, since reproducibility through those tools is already common in the Java ecosystem.
- Time-bucketed quarters as the roadmap structure — rejected in favor of versioned milestones; pace is too uncertain for calendar pressure and milestones map cleanly onto release artifacts.
- A flat prioritized backlog — rejected because it loses the narrative of what `1.0` means.

## 2026-05-21 - Resolver Property Interpolation Started

### What was decided

- Start milestone 0.2 with current-POM property interpolation in `src/resolver.rs`.
- Support Maven-style `${project.groupId}`, `${project.artifactId}`, `${project.version}`, matching `pom.*` aliases, and user-defined `<properties>` values when resolving dependency coordinates and exclusions.
- Allow recursive property values, while continuing to fail clearly on unresolved or cyclic properties.

### Why

- Property interpolation is the smallest 0.2 slice that removes the resolver's immediate real-world blocker without pulling parent POM inheritance, dependency management, and BOM imports into the same change.
- Keeping the POM parser inline for now avoids a premature module split; parent POM and dependency management will likely justify extracting `src/pom.rs`.

### What was rejected and why

- Implementing inherited parent properties in this branch was rejected because it belongs with recursive parent POM resolution.
- Adding dependency management or BOM behavior was rejected because property interpolation can land independently and gives a cleaner review boundary.

## 2026-05-21 - Picocli Canary Reconnaissance

### What was decided

- Treat `benches/canary` as a local-only picocli source checkout and do not commit it.
- Use picocli's Maven example projects as compatibility canaries, not as benchmark proof yet.
- The root picocli `pom.xml` is a published-artifact POM with no dependencies, so it is not useful for resolver benchmarking.

### Why

- Picocli is mostly a Gradle source tree; Angra's current 0.2 work is Maven resolver behavior, so the small Maven examples are the relevant surface.
- The simplest Maven example's runtime graph resolves to `info.picocli:picocli:4.7.7`, and Angra can match that when represented as a temporary `angra.toml`.

### What was rejected and why

- Treating picocli root as a dependency-resolution benchmark was rejected because it has no dependency graph.
- Treating the canary as a committed fixture was rejected because it is a full external source checkout and should remain local-only.

## 2026-05-21 - Commons Compress Canary Benchmark

### What was decided

- Replace the local-only `benches/canary` checkout with Apache Commons Compress for resolver compatibility and timing experiments.
- Benchmark resolution in warm-cache/offline mode against Maven's runtime dependency tree.
- Keep the Commons Compress canary uncommitted; use temporary files under `/private/tmp` for Angra manifests, local Maven repos, and benchmark output.
- Fix runtime graph traversal so non-runtime scoped dependencies are skipped before coordinate interpolation.

### Why

- Commons Compress has useful real-world Maven pressure: `commons-parent`, inherited properties, managed test dependency versions, optional direct dependencies, and a moderate runtime graph.
- Maven's runtime tree for the current canary resolves seven compile artifacts: `zstd-jni`, Brotli `dec`, `xz`, `commons-codec`, `asm`, `commons-io`, and `commons-lang3`.
- Angra matched that seven-artifact runtime graph when represented as a temporary `angra.toml`.
- Warm-cache process timing over seven runs: Angra release binary resolved in 27-29 ms; Maven 4.0.0-rc-5 `dependency:tree -Dscope=runtime` resolved in 1315-1350 ms. This is a canary result, not a claim of full Maven parity.

### What was rejected and why

- Benchmarking cold network resolution was rejected because network variability would hide resolver overhead.
- Treating the temporary Angra TOML as equivalent to `pom.xml` ingestion was rejected because parent POM inheritance and `import-pom` are not implemented yet.

## 2026-05-21 - Effective POM Support

### What was decided

- Extract POM parsing, property interpolation, dependency management modeling, and effective-model merging into `src/pom.rs`.
- Resolve parent POMs recursively and merge inherited properties plus dependency management into an effective POM.
- Apply `<dependencyManagement>` to dependencies that omit versions, including managed scopes and exclusions.
- Support BOM imports via dependency management entries with `<type>pom</type>` and `<scope>import</scope>`.
- Keep resolver graph traversal in `src/resolver.rs`, with POM fetching remaining resolver-owned.

### Why

- Parent POM inheritance, dependency management, and BOM imports share the same effective-POM concept; keeping that model outside `resolver.rs` avoids making graph traversal responsible for XML/model-building details.
- Managed scope can make a dependency non-runtime, so runtime eligibility must be checked after dependency management is applied.
- Parent and BOM POMs are POM-only artifacts, so the resolver now has a separate `ensure_pom` path rather than requiring jars for every coordinate.

### What was rejected and why

- Fully modeling Maven's local `<relativePath>` parent lookup was rejected for this slice; repository-resolved parent POMs are enough for Maven Central compatibility and transitive artifacts.
- Treating this as complete 0.2 resolver realism was rejected because classifiers, packaging beyond POM imports, mirrors/settings, checksums, parallel downloads, and failure attribution still remain.

## 2026-05-22 - Roadmap Folded Strategic Architecture Themes

### What was decided

- Fold the new strategic ideas into the existing versioned roadmap instead of creating a separate architecture RFC.
- Make the Rust-driver/JVM-worker split explicit: Angra keeps normal CLI and resolution work JVM-free, and starts Java only for compile, test, run, package, or future plugin compatibility.
- Keep async/parallel dependency resolution and possible metadata indexing in 0.2, because they reinforce Angra's speed premise without changing the user workflow.
- Add a measured JVM worker spike to 0.4 compile/test, rather than committing to a persistent daemon before benchmarks prove it is worth the complexity.
- Treat Maven plugin execution as a deferred but important adoption gate, not part of the 1.0 inner-loop replacement.

### Why

- The roadmap already has a useful milestone structure, so folding the ideas into it keeps the plan executable.
- Maven correctness and clear compatibility boundaries matter more than a broad architecture rewrite.
- A persistent JVM daemon can improve repeated compile/test loops, but it adds lifecycle complexity and should be justified by benchmarks.

### What was rejected and why

- Creating a separate architecture RFC was rejected because the user explicitly chose to skip it and update the roadmap instead.
- Built-in JDK download/management was not reintroduced because the roadmap already delegates toolchains to `mise` / SDKMan through 1.0.
- Supporting arbitrary Maven plugins before 1.0 was rejected because it would turn the roadmap into a full Maven host instead of a fast inner-loop tool.

## 2026-05-22 - Classifier And Packaging Resolver Support

### What was decided

- Extend Maven artifact identity beyond `group:artifact:version` by adding `ArtifactCoordinate`, supported artifact types (`jar`, `pom`, `war`), and optional classifiers.
- Preserve `Coordinate` as the plain Maven GAV descriptor used for normal POM reads.
- Resolve classified artifacts and `war` artifacts from extension-aware paths while still reading the normal unclassified POM descriptor.
- Treat unclassified `pom` dependencies as descriptor-only artifacts: resolve/read the POM and do not require a jar or war file.
- Rename lockfile artifact fields from jar-specific names to `artifact_path` and `artifact_sha256`, with `type` and optional `classifier` recorded.

### Why

- Maven artifact identity includes type and classifier in practice; collapsing everything to GAV would make native artifacts, source/javadoc-style classifiers, WARs, and POM-only dependencies ambiguous.
- Keeping normal POM descriptor lookup separate from artifact lookup matches Maven's layout and keeps classified artifact resolution simple.
- Angra's lockfile is still pre-1.0, so now is the right time to remove jar-specific names before more resolver behavior depends on them.

### What was rejected and why

- Guessing unknown Maven types was rejected for this slice; unsupported types fail explicitly until Angra models Maven artifact handlers.
- Extending compact TOML strings beyond `group:artifact:version` was rejected to preserve the current ergonomic shorthand and keep richer identity in structured dependencies.
- Implementing the future `angra package` command was rejected because this work is resolver packaging/type support, not project packaging output.

## 2026-05-22 - Dependency Failure Attribution

### What was decided

- Track dependency provenance in the resolver queue as a vector of `ArtifactCoordinate` values.
- Wrap artifact fetch, effective-POM, and dependency parse failures with the dependency path active at the failure point.
- Render resolver CLI failures with colorized terminal output and a compact `root -> child -> failing-artifact` path.
- Keep path tracking out of the lockfile and successful resolution output.

### Why

- Maven compatibility debugging needs to answer why a coordinate was being resolved, not just which artifact failed.
- The path vector is simple, deterministic, and reusable later for `angra tree` and `angra why` without introducing a graph abstraction yet.
- ANSI output gives the CLI a clearer human surface without adding a dependency or changing machine-readable artifacts.

### What was rejected and why

- Building a full dependency graph model now was rejected as unnecessary ceremony for failure attribution; the queue path is enough for this slice.
- Adding a color crate was rejected because a few ANSI styles cover the current CLI output and avoid dependency churn.
- Persisting dependency paths in `angra.lock` was rejected because lockfiles should describe resolved artifacts, not resolver diagnostics.

## 2026-05-22 - Strict SHA-1 Download Verification

### What was decided

- Verify Maven Central downloads against the sibling `.sha1` file before writing the artifact or POM into the local repository.
- Store the fetched `.sha1` next to the verified local file using Maven's `file.ext.sha1` layout.
- Parse common Maven checksum formats: bare hex with optional filename, uppercase hex, and `SHA1 (...) = hex` style content.
- Treat checksum mismatch or malformed checksum content as resolver errors.

### Why

- Maven compatibility is not just graph shape; downloaded bytes must be trusted before Angra puts them into `~/.m2/repository`.
- Verifying in memory before writing avoids leaving known-bad artifacts in the local repository.
- Strict failure is simpler and safer until Angra implements Maven settings policies such as warn/ignore/fail.

### What was rejected and why

- Implementing Maven's full checksum policy matrix was rejected for this slice; settings.xml support has not landed yet.
- Falling back to MD5 was rejected because Maven Central provides SHA-1 for this compatibility target and MD5 is a legacy fallback.
- Verifying already-cached local files on every resolve was rejected to keep warm-cache resolution fast; this slice covers remote downloads.

## 2026-05-22 - Commons Compress Baseline And Parallel Fetching

### What was decided

- Benchmark Commons Compress resolver behavior using an equivalent temporary Angra TOML and the real Commons Compress Maven POM.
- Keep all benchmark manifests and local repositories under `/private/tmp`, leaving `benches/canary` untracked.
- Add same-depth parallel artifact fetching to the resolver: dependencies at the same BFS depth fetch concurrently, then effective-POM parsing and graph expansion continue in deterministic queue order.

### Why

- Angra does not ingest source `pom.xml` files yet, so the benchmark isolates resolver performance rather than import behavior.
- Same-depth batching gives useful network parallelism without changing nearest-wins conflict resolution semantics.
- Keeping effective-POM expansion sequential after each fetch batch avoids racing parent/BOM descriptor writes while still overlapping independent artifact downloads.

### Benchmark Notes

- Commons Compress warm-cache baseline before parallel fetching: Angra release binary resolved the seven-artifact runtime graph in 30-31 ms over seven offline runs.
- Maven `dependency:tree -Dscope=runtime` on the real Commons Compress POM, using a warm isolated temp Maven repo, took 1350-1599 ms over seven runs.
- After parallel fetching, Angra warm-cache steady-state remained 30-32 ms after one first-run outlier; one cold network resolve into a fresh temp repo completed in 1222 ms.

### What was rejected and why

- Rewriting the resolver around async `reqwest` was rejected for this slice because the blocking resolver can gain first-order parallel fetch behavior with less churn.
- Parallelizing effective-POM parent/BOM expansion was rejected for now because shared descriptor paths can race and the current bottleneck target is artifact download concurrency.
- Committing Commons Compress as a fixture was rejected again; it remains a local benchmark/canary input.

## 2026-05-24 - Project-Local Angra Repositories

### What was decided

- Add Angra-managed project repositories through a `[repositories]` table in `angra.toml`.
- Keep Maven Central as the default repository when `[repositories]` is omitted.
- Record the repository name as the lockfile `source` for artifacts downloaded through project-local repositories.
- Keep global Angra config in the roadmap as a later follow-up, separate from Maven `settings.xml`.

### Why

- Angra needs a repository story it owns directly, instead of making users depend on Maven settings for ordinary non-Central resolution.
- Project-local repository declarations are explicit, portable, and match Angra's low-ceremony TOML model.
- A later global Angra config can reduce repetition across projects without importing Maven's settings/mirror/auth complexity too early.

### What was rejected and why

- Starting with Maven `settings.xml` was rejected as the primary path because it makes Angra's normal workflow depend on Maven-owned configuration.
- Adding auth in this slice was rejected; unauthenticated repositories are enough to establish the model and keep the review boundary small.
- Adding global config immediately was rejected in favor of project-local support first, because precedence rules and config discovery deserve their own slice.
