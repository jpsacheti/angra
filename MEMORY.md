# MEMORY.md

Durable project memory for Angra. Keep this file compact: record decisions that should constrain future work, current unresolved boundaries, and next priorities. Do not use it as a changelog; git history and `ROADMAP.md` carry implementation detail.

## Product Direction

- Angra aims to be the Java equivalent of `uv`: fast, low-ceremony, Maven-compatible, and pleasant for Java developers and DevOps engineers.
- Angra uses TOML as its native project management format while staying compatible with Maven artifacts and `pom.xml` behavior.
- Maven compatibility and low overhead are foundational constraints. Avoid changes that make Angra slower than Maven or more ceremonious than Maven/Gradle.
- The normal resolver and CLI path should remain JVM-free. Start Java only for compile, test, run, package, or future plugin compatibility.
- Toolchains are delegated to `mise` / SDKMan through the 1.0 roadmap. Built-in JDK download/management is out of scope for now.
- Apache-2.0 is the project license.

## Roadmap Shape

- Keep `ROADMAP.md` as the checked-in roadmap and keep it in sync when milestone scope changes.
- The 0.1 to 1.0 path ends at build and run:
  - 0.1 resolver MVP: shipped.
  - 0.2 resolver realism: in progress.
  - 0.3 manifest lifecycle: planned.
  - 0.4 compile and test: planned.
  - 0.5 package and run: planned.
  - 1.0 hardening: planned.
- Use versioned milestones with plain text status tags such as `[shipped]`, `[in progress]`, `[planned]`, and `[deferred]`.
- Each milestone should exit with tests and clippy green, a fixture project working end to end, benchmark harness updates where relevant, and a memory entry for durable decisions.
- Publishing, IDE plugins, built-in JDK management, and arbitrary Maven plugin execution are deferred beyond the 1.0 inner-loop target unless explicitly reprioritized.

## Resolver Baseline

- The first useful slice is `angra resolve`.
- `angra resolve` reads `angra.toml`, resolves Maven-compatible dependencies from the local Maven repository and configured remote repositories, caches artifacts in Maven local repository layout, and writes deterministic TOML `angra.lock`.
- Supported dependency declaration forms:
  - Compact string: `group:artifact:version`.
  - Structured dependency with `group`, `artifact`, `version`, `scope`, `exclusions`, and richer artifact identity fields where supported.
- Do not shell out to Maven for Angra resolution. That weakens the product premise and hides performance characteristics.
- Starting with full `pom.xml` ingestion was intentionally rejected. The first compatibility target is Maven artifact/POM resolution behavior; source project POM import can come later.
- Benchmarks compare Angra with Maven and Gradle using `mise` with dynamic latest Maven/Gradle versions. Do not pin those tool versions in repo config unless the benchmark design changes.
- Benchmark output should be human-readable first, followed by raw JSON details.

## Maven Model Decisions

- POM parsing, property interpolation, dependency management modeling, and effective-model merging live in `src/pom.rs`.
- Resolver graph traversal stays in `src/resolver.rs`; POM fetching remains resolver-owned.
- Effective POM support includes recursive repository-resolved parent POMs, inherited properties, dependency management, managed scopes/exclusions, and BOM imports via `<type>pom</type>` plus `<scope>import</scope>`.
- Current property interpolation supports Maven-style `${project.groupId}`, `${project.artifactId}`, `${project.version}`, matching `pom.*` aliases, user-defined `<properties>`, recursive property values, and explicit errors for unresolved or cyclic properties.
- Managed scope can make a dependency non-runtime, so runtime eligibility must be checked after dependency management is applied.
- Parent and BOM POMs are POM-only artifacts; resolver has a separate POM path and must not require jars for every coordinate.
- Fully modeling Maven local parent `<relativePath>` lookup is deferred. Repository-resolved parent POMs are enough for the current Maven Central compatibility slice.
- Profile behavior remains incomplete. Settings profile activation currently supports listed active profiles and `activeByDefault`; property, OS, JDK, and file activation remain deferred.

## Artifact Identity And Lockfile

- Maven artifact identity is richer than plain GAV. Angra models `ArtifactCoordinate`, supported artifact types (`jar`, `pom`, `war`), and optional classifiers.
- `Coordinate` remains the plain Maven GAV descriptor used for normal POM reads.
- Classified artifacts and `war` artifacts resolve from extension-aware paths while still reading the normal unclassified POM descriptor.
- Unclassified `pom` dependencies are descriptor-only artifacts: resolve/read the POM and do not require a jar or war file.
- Unknown Maven artifact types fail explicitly until Angra models Maven artifact handlers. Do not guess extensions.
- Compact TOML strings stay `group:artifact:version`; richer identity belongs in structured dependencies.
- Lockfile artifact fields are artifact-neutral: use `artifact_path`, `artifact_sha256`, `type`, and optional `classifier`. Avoid jar-specific naming.
- Dependency paths are diagnostics only. Do not persist resolver provenance in `angra.lock`.

## Repository And Config Decisions

- Project-local repositories are declared in `[repositories]` inside `angra.toml`.
- Global Angra config lives at `~/.config/angra/config.toml` on Unix-like systems, including macOS. Windows uses the platform config directory.
- Global config supports `[repositories]` using the same name equals URL shape as project TOML.
- Repository declaration order is resolver behavior, not presentation detail. Preserve order from TOML declaration.
- Repository precedence by name: project repos override global repos, and global repos override settings repos.
- Resolver fallback order: global repositories first in declaration order, then unmatched project repositories, then unmatched Maven settings repositories. Maven Central is used only when no repositories are configured anywhere.
- Do not silently append Maven Central after configured repositories. Configured repositories are explicit.
- Do not sort repositories by name; order is semantically meaningful fallback behavior.
- A separate `--config` flag is premature.
- Additional remote repositories declared in POMs are isolated using Lexically Scoped Repositories (Approach A). Each dependency in the resolution queue carries its own list of permitted active repositories. Discovered repositories are merged down to children and descendants, preventing "repository leakage/pollution" across unrelated packages. This was chosen over global dynamic appending for safety and predictability, matching modern tools like Cargo and uv.
- Settings mirrors are applied dynamically to POM-discovered repositories before they are used to fetch artifacts.

## Maven Settings Decisions

- `src/settings.rs` owns read-only Maven settings support.
- Read only user settings at `~/.m2/settings.xml`. Do not read Maven global settings from `${maven.home}/conf/settings.xml`; Angra must not require a Maven install.
- Settings support currently covers `<localRepository>`, active-profile `<repositories>`, and `<mirrors>`.
- Local repository precedence: explicit `ResolveOptions.local_repo` > settings `<localRepository>` > `~/.m2/repository`.
- Settings repositories are compatibility tail entries. Legacy Maven config must not silently shadow explicit Angra-native project or global configuration.
- Mirrors from settings are applied after project/global/settings repository merging and before creating the resolver.
- Mirror matching supports `*`, comma-separated repository IDs, and `!` negation. First matching mirror wins.
- Mirror application rewrites the matched repository name and URL, then deduplicates by name so wildcard mirrors do not cause redundant requests.
- The resolver should remain mirror-unaware; settings concepts are applied before the repository list reaches resolver fetching.
- `<servers>`, `<proxies>`, and auth are deferred.
- Maven `external:*` mirror semantics and broader glob/regex `mirrorOf` patterns are deferred; current repositories are HTTP(S)-oriented.

## Download Integrity

- Remote Maven downloads are verified against sibling `.sha1` files before writing artifacts or POMs into the local repository.
- Verified `.sha1` files are stored next to local files using Maven's `file.ext.sha1` layout.
- Parse common Maven SHA-1 checksum formats: bare hex with optional filename, uppercase hex, and `SHA1 (...) = hex`.
- Checksum mismatch or malformed checksum content is a resolver error.
- Do not fall back to MD5.
- Do not reverify already-cached local files on every resolve; warm-cache speed matters. This can be revisited when Maven checksum policies are modeled.
- Full Maven checksum policy behavior is deferred until settings policy support exists.

## Repository Policy Decisions

- Repositories carry release and snapshot policies modeled as `RepositoryPolicy { enabled: bool }`.
- Maven Central defaults to releases=true, snapshots=false, matching real Maven Central behavior.
- All other repositories (angra.toml, global config, settings, POM-declared) default to both enabled unless explicitly overridden.
- SNAPSHOT detection uses the `-SNAPSHOT` suffix convention (case-sensitive). Full SNAPSHOT timestamp/build-number resolution (e.g. `1.0-20240101.120000-1`) remains deferred.
- Repository policies are parsed from POM `<releases><enabled>` and `<snapshots><enabled>` elements, and from settings.xml profile repository policy elements.
- The resolver skips repositories whose policy does not match the artifact version type before attempting any download.
- No `updatePolicy`, `checksumPolicy`, or other policy sub-elements are modeled yet.

## Auth Diagnostics

- HTTP 401 (Unauthorized) and 403 (Forbidden) responses are intercepted and produce a dedicated `AuthenticationRequired` error.
- The error message explicitly states that Angra does not yet support authenticated repositories and suggests configuring a mirror or using a public repository.
- Auth errors immediately fail resolution rather than falling through to the next repository, since a 401/403 from a repo that should have the artifact is a real signal.
- Actual authentication (`<servers>`, credentials, tokens) remains deferred.

## Resolver Diagnostics And Performance

- Track dependency provenance in the resolver queue as a vector of `ArtifactCoordinate` values.
- Wrap artifact fetch, effective-POM, and dependency parse failures with the active dependency path.
- CLI resolver failures render a compact, colorized `root -> child -> failing-artifact` path.
- Avoid adding a color dependency for now; simple ANSI styling is enough.
- Same-depth resolver fetching may run concurrently. Dependencies at the same BFS depth fetch in parallel, then effective-POM parsing and graph expansion continue in deterministic queue order.
- Same-depth batching gives network parallelism without changing nearest-wins conflict semantics.
- Do not rewrite the resolver around async `reqwest` unless blocking plus bounded parallelism stops being enough.
- Do not parallelize effective-POM parent/BOM expansion until races around shared descriptor paths are explicitly designed.

## Benchmarks And Canaries

- Keep canary source checkouts uncommitted.
- Picocli's Maven examples are useful compatibility canaries, but the root picocli POM is not a useful resolver benchmark because it has no dependency graph.
- Apache Commons Compress is the current real-world resolver canary because it stresses parent POMs, inherited properties, managed dependencies, optional dependencies, and a moderate runtime graph.
- Commons Compress canary manifests, isolated Maven repos, and timing output should live under `/private/tmp`.
- Benchmark warm-cache/offline resolver behavior against Maven's runtime dependency tree when comparing overhead.
- Do not treat temporary Angra TOML canaries as proof of source `pom.xml` ingestion.
- Do not benchmark cold network resolution as a primary signal; network variability hides resolver overhead.

## Dependency Upgrade Notes

- `reqwest` 0.13 uses the `rustls` feature; the old `rustls-tls` feature is gone.
- Do not use `reqwest` 0.13's `rustls-no-provider` casually. It would require Angra to manage Rustls crypto-provider setup at runtime.
- Cargo ignores semver build metadata in version requirements; write `toml = "1.1.2"` rather than `1.1.2+spec-1.1.0`.
- `quick-xml` 0.40 text handling should decode text and then unescape XML entities explicitly.
- `sha1` and `sha2` 0.11 finalized digest outputs no longer implement `LowerHex` directly. Hex-encode finalized bytes locally unless a broader formatting need justifies a dependency.

## Current Open Boundaries

- Auth implementation (Maven `<servers>`, credentials, tokens) and proxies are not implemented. Auth errors are diagnosed with actionable messages.
- Snapshot timestamp/build-number resolution, version ranges, and full Maven profile activation remain unresolved.
- Local parent POM `<relativePath>` behavior remains unresolved.
- Source `pom.xml` ingestion remains separate from artifact/POM resolution.
- Maven plugin execution is deferred as an adoption gate, not part of the 1.0 inner-loop replacement.
- The measured JVM worker spike belongs in the 0.4 compile/test milestone; do not commit to a persistent daemon before benchmarks justify it.

## Next Priorities

- Continue 0.2 resolver realism with the remaining Maven compatibility gaps: snapshots, version ranges, and profile activation.
- Re-run the Commons Compress canary after settings/mirror-related resolver changes.
- Keep tests, clippy, and benchmark coverage aligned with any resolver behavior change.

## Rejected Paths To Preserve

- No durable memory was rejected; future sessions need decision continuity.
- Treating TOML ergonomics as separate from Maven compatibility was rejected.
- Full uv-equivalent scope before 1.0 was rejected as too broad.
- Built-in JDK management was rejected for the current roadmap.
- A flat backlog-only roadmap was rejected because it loses the narrative of what 1.0 means.
- A separate architecture RFC was rejected in favor of folding strategic architecture decisions into the roadmap.
- A full dependency graph abstraction was rejected for current failure attribution; queue path tracking is enough.
- A manual bit-flipping hex-encoding function was initially implemented but subsequently rejected in favor of the highly optimized, SIMD-accelerated 'faster-hex' library to prioritize maximum runtime speed and code safety over compile-time savings.
- Parsing auth or mirrors in the original settings repository slice was rejected to keep review boundaries small; mirrors have since landed, auth remains deferred.
