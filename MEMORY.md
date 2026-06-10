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
- Current milestone state:
  - 0.1 resolver MVP: shipped.
  - 0.2 resolver realism: shipped.
  - 0.3 manifest lifecycle: in progress, migration-first with effective concrete `import-pom`.
  - 0.4 compile and test: planned.
  - 0.5 package and run: planned.
  - 0.6 private repositories (basic auth): planned.
  - 1.0 hardening: planned.
- Basic CI (fmt/test/clippy on every PR) was pulled forward from 1.0 into 0.3; bench-on-PR remains a 1.0 item.
- Windows support is an explicit deferral past 1.0, recorded in `ROADMAP.md` Deferred.
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
- Benchmark output should be human-readable first, followed by raw JSON details. The benchmark harness should print progress while external commands run so slow Maven/Gradle/mise work is distinguishable from a hung harness.

## Maven Model Decisions

- POM parsing, property interpolation, dependency management modeling, and effective-model merging live in `src/pom.rs`.
- Resolver graph traversal stays in `src/resolver.rs`; POM fetching remains resolver-owned.
- Effective POM support includes recursive repository-resolved parent POMs, inherited properties, dependency management, managed scopes/exclusions, and BOM imports via `<type>pom</type>` plus `<scope>import</scope>`.
- Angra-native dependency management lives in `[dependency-management]` in `angra.toml`; it supports managed versions and BOM imports via `type = "pom", scope = "import"`.
- Current property interpolation supports Maven-style `${project.groupId}`, `${project.artifactId}`, `${project.version}`, matching `pom.*` aliases, user-defined `<properties>`, recursive property values, and explicit errors for unresolved or cyclic properties.
- Managed scope can make a dependency non-runtime, so runtime eligibility must be checked after dependency management is applied.
- Parent and BOM POMs are POM-only artifacts; resolver has a separate POM path and must not require jars for every coordinate.
- Maven local parent `<relativePath>` lookup is supported before repository fallback. Missing `<relativePath>` means `../pom.xml`; empty `<relativePath/>` disables local lookup.
- Maven profile activation supports resolver-relevant POM sections: manifest active/inactive profile IDs, `activeByDefault`, property, OS, JDK, and file activation. Profiles inject dependencies, dependency management, properties, and repositories.
- Angra-native manifest controls for Maven profile activation live under `[resolver.maven]`; this was chosen over Maven-like `-P`/`-D` flags for the resolver slice.
- JDK profile activation remains JVM-free. It reads `[resolver.maven].java-version` first, then `JAVA_HOME/release`; it must not spawn `java`.

## Artifact Identity And Lockfile

- Maven artifact identity is richer than plain GAV. Angra models `ArtifactCoordinate`, supported artifact types (`jar`, `pom`, `war`), and optional classifiers.
- `Coordinate` remains the plain Maven GAV descriptor used for normal POM reads.
- Classified artifacts and `war` artifacts resolve from extension-aware paths while still reading the normal unclassified POM descriptor.
- Unclassified `pom` dependencies are descriptor-only artifacts: resolve/read the POM and do not require a jar or war file.
- Unknown Maven artifact types fail explicitly until Angra models Maven artifact handlers. Do not guess extensions.
- Compact TOML strings stay `group:artifact:version`; richer identity belongs in structured dependencies.
- Lockfile artifact fields are artifact-neutral: use `artifact_path`, `artifact_sha256`, `type`, and optional `classifier`. Avoid jar-specific naming.
- Dependency paths are diagnostics only. Do not persist resolver provenance in `angra.lock`.
- Lockfiles record concrete resolved versions for ranges and timestamped SNAPSHOTs. When the requested version differs, `requested_version` records the original range or `-SNAPSHOT` request.
- `angra.lock` carries an optional `manifest_fingerprint`: SHA-256 over canonical resolver-relevant manifest intent (dependencies, dependency management, project repositories, `[resolver.maven]`). It is an input digest for `--frozen` drift detection, not resolver provenance; machine-global config and Maven settings are excluded so lockfiles stay portable.
- `angra resolve --frozen` installs exactly the locked artifacts (no metadata lookups, no graph traversal, never rewrites the lock) and verifies every artifact against its locked SHA-256 — including cached files, a deliberate frozen-only exception to the warm-cache no-reverify rule. Locked `pom_path`/`artifact_path` are informational; frozen recomputes paths from coordinates against the current local repo.

## Repository And Config Decisions

- Project-local repositories are declared in `[repositories]` inside `angra.toml`.
- Global Angra config lives at `~/.config/angra/config.toml` on Unix-like systems, including macOS. Windows uses the platform config directory.
- Project and global `[repositories]` support both compact `name = "url"` and structured `name = { url = "...", releases = true, snapshots = false, checksum-policy = "fail" }` forms.
- Structured repository declarations are the Angra-native equivalent for Maven release/snapshot and checksum policy settings.
- Repository declaration order is resolver behavior, not presentation detail. Preserve order from TOML declaration.
- Repository precedence by name: project repos override global repos, and global repos override settings repos.
- Resolver fallback order: global repositories first in declaration order, then unmatched project repositories, then unmatched Maven settings repositories. Maven Central is used only when no repositories are configured anywhere.
- Do not silently append Maven Central after configured repositories. Configured repositories are explicit.
- Do not sort repositories by name; order is semantically meaningful fallback behavior.
- A separate `--config` flag is premature.
- Additional remote repositories declared in POMs are isolated using Lexically Scoped Repositories (Approach A). Each dependency in the resolution queue carries its own list of permitted active repositories. Discovered repositories are merged down to children and descendants, preventing repository leakage across unrelated packages.
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

## Download Integrity And Policies

- Remote Maven downloads are verified against sibling `.sha1` files before writing artifacts or POMs into the local repository.
- Verified `.sha1` files are stored next to local files using Maven's `file.ext.sha1` layout.
- Parse common Maven SHA-1 checksum formats: bare hex with optional filename, uppercase hex, and `SHA1 (...) = hex`.
- Checksum mismatch or malformed checksum content is a resolver error under the default strict policy.
- Repository `checksumPolicy` supports `fail`, `warn`, and `ignore`. Angra defaults to strict `fail`; `warn` succeeds with a CLI warning and does not cache the invalid checksum; `ignore` skips checksum fetch/verification.
- Do not fall back to MD5.
- Do not reverify already-cached local files on every resolve; warm-cache speed matters. This can be revisited when Maven checksum policies are modeled.
- Full Maven checksum policy behavior is deferred until settings policy support exists.
- Repositories carry release and snapshot policies modeled as `RepositoryPolicy { enabled: bool }`.
- Maven Central defaults to releases=true, snapshots=false, matching real Maven Central behavior.
- All other repositories (angra.toml, global config, settings, POM-declared) default to both enabled unless explicitly overridden.
- SNAPSHOT detection uses the `-SNAPSHOT` suffix convention, case-sensitive. Timestamp/build-number resolution is supported through Maven metadata, including `snapshotVersions` and legacy `<snapshot>` timestamp/build number fields.
- Repository policies are parsed from POM `<releases><enabled>` and `<snapshots><enabled>` elements, and from settings.xml profile repository policy elements.
- The resolver skips repositories whose policy does not match the artifact version type before attempting any download.
- `checksumPolicy` is modeled. `updatePolicy` and other policy sub-elements remain deferred.

## Diagnostics And Performance

- HTTP 401 and 403 responses produce a dedicated `AuthenticationRequired` error. Auth errors immediately fail resolution rather than falling through to the next repository.
- Actual authentication (`<servers>`, credentials, tokens) and proxies remain deferred.
- Track dependency provenance in the resolver queue as a vector of `ArtifactCoordinate` values.
- Wrap artifact fetch, effective-POM, and dependency parse failures with the active dependency path.
- CLI resolver failures render a compact, colorized `root -> child -> failing-artifact` path.
- Avoid adding a color dependency for now; simple ANSI styling is enough.
- Same-depth resolver fetching may run concurrently. Dependencies at the same BFS depth fetch in parallel, then effective-POM parsing and graph expansion continue in deterministic queue order.
- Same-depth batching gives network parallelism without changing nearest-wins conflict semantics.
- Do not rewrite the resolver around async `reqwest` unless blocking plus bounded parallelism stops being enough.
- Do not parallelize effective-POM parent/BOM expansion until races around shared descriptor paths are explicitly designed.
- A future `tokio` adoption is under consideration for more complex async networking or filesystem I/O, but this does not override the lightweight/JVM-free resolver constraint.

## Benchmarks And Canaries

- Keep canary source checkouts uncommitted.
- Picocli's Maven examples are useful compatibility canaries, but the root picocli POM is not a useful resolver benchmark because it has no dependency graph.
- Apache Commons Compress is the current real-world resolver canary because it stresses parent POMs, inherited properties, managed dependencies, optional dependencies, and a moderate runtime graph.
- Commons Compress canary manifests, isolated Maven repos, and timing output should live under `/private/tmp`.
- Benchmark warm-cache/offline resolver behavior against Maven's runtime dependency tree when comparing overhead.
- The local Spring Boot fixture under `benches/spring-fixture` is a resolver canary for Angra-native BOM management and Maven runtime-set parity. It is included by the bench harness when present and now has Gradle runtimeClasspath support.
- Do not treat temporary Angra TOML canaries as proof of source `pom.xml` ingestion.
- Do not benchmark cold network resolution as a primary signal; network variability hides resolver overhead.
- Last known sandbox benchmark caveat: Angra completed successfully, but Maven/Gradle failed because `mise` could not fetch/write tool metadata cleanly under sandboxed network/cache constraints.

## Dependency Upgrade Notes

- `reqwest` 0.13 uses the `rustls` feature; the old `rustls-tls` feature is gone.
- Do not use `reqwest` 0.13's `rustls-no-provider` casually. It would require Angra to manage Rustls crypto-provider setup at runtime.
- Cargo ignores semver build metadata in version requirements; write `toml = "1.1.2"` rather than `1.1.2+spec-1.1.0`.
- `quick-xml` 0.40 text handling should decode text and then unescape XML entities explicitly.
- `sha1` and `sha2` 0.11 finalized digest outputs no longer implement `LowerHex` directly. Hex-encode finalized bytes locally unless a broader formatting need justifies a dependency.
- `faster-hex` was adopted for checksum hex serialization to prioritize runtime performance and code safety over compile-time savings.

## Current Open Boundaries

- Auth implementation (Maven `<servers>`, credentials, tokens) and proxies are not implemented. Auth errors are diagnosed with actionable messages.
- Snapshot timestamp/build-number resolution and Maven version ranges are implemented for resolver metadata selection.
- Maven profile support is resolver-focused, not a full Maven build-model/plugin compatibility layer.
- Source `pom.xml` ingestion is now a one-way 0.3 `import-pom` migration path, not live POM-as-manifest execution.
- Maven plugin execution is deferred as an adoption gate, not part of the 1.0 inner-loop replacement.
- The measured JVM worker spike belongs in the 0.4 compile/test milestone; do not commit to a persistent daemon before benchmarks justify it.

## Next Priorities

- Finish 0.3 validation and polish for migration-first manifest lifecycle.
- Keep `import-pom` effective and concrete while warning for Maven build/plugin/reporting/publishing behavior that Angra does not model.
- Preserve lockfile artifact-only semantics; `tree` and `why` use in-memory resolver graph data, not persisted provenance.
- Keep tests, clippy, formatting, and benchmark coverage aligned with any CLI or manifest behavior change.

## Shipped Milestone Notes

- **0.1 Resolver MVP:** `angra resolve`, Maven Central/local repository layout, runtime graph traversal, deterministic lockfile, and benchmark harness shipped.
- **0.2 Resolver realism:** Maven version ranges, timestamped SNAPSHOTs, recursive effective POMs, local parent `<relativePath>`, dependency management, BOM imports, profile activation/injection, repository/settings/mirror/policy support, artifact identity (`jar`, `pom`, `war`, classifier), SHA-1 verification, resolver diagnostics, and parallel fetch behavior shipped.
- **0.2 performance work:** Single-pass XML property parsing, effective-POM caching, local relative-parent POM caching, and continuous worker-queue artifact downloading replaced slower repeated parsing and barrier-style downloads.
- **0.2 validation:** Latest green housekeeping run had `cargo fmt --check`, `cargo test` (90 unit tests, 13 integration tests), and `cargo clippy --all-targets -- -D warnings` passing. Release benchmark ran; Angra succeeded while Maven/Gradle failed due to sandboxed `mise` constraints.

## Rejected Paths To Preserve

- Treating TOML ergonomics as separate from Maven compatibility was rejected.
- Full uv-equivalent scope before 1.0 was rejected as too broad.
- Built-in JDK management was rejected for the current roadmap.
- A flat backlog-only roadmap was rejected because it loses the narrative of what 1.0 means.
- A separate architecture RFC was rejected in favor of folding strategic architecture decisions into the roadmap.
- A full dependency graph abstraction was rejected for current failure attribution; queue path tracking is enough.
- Full source `pom.xml` ingestion as the first compatibility target was rejected; artifact/POM resolution compatibility came first.
- Maven-like `-P`/`-D` flags were rejected for the resolver slice in favor of manifest-based profile controls.
- Lock-stable range reuse was rejected for now; ranges resolve fresh and lock the concrete result.
- Treating the Spring fixture as explicit direct dependencies only was rejected because it matched artifact count but drifted managed transitive versions.
- Relying on heavy external async runtimes or thread-pooling crates for 0.2 was rejected; standard `thread::scope` and `mpsc` were enough.
- Parsing auth or mirrors in the original settings repository slice was rejected to keep review boundaries small; mirrors have since landed, auth remains deferred.
- Setting up local git pre-commit hooks was rejected for now; validation remains explicit until a CI pipeline exists.

## Decision Entry - 2026-06-02 (Memory Compaction)

- **What was decided:** Conservatively compacted `MEMORY.md` by preserving durable decisions, current state, open boundaries, validation status, and next priorities while collapsing repetitive 0.2 session detail.
- **Why:** Future 0.3 sessions need continuity without rereading the full resolver implementation diary.
- **Rejected and why:** Aggressive compaction was rejected because it might hide useful resolver constraints. Keeping session-by-session history was rejected because git history and `ROADMAP.md` already carry implementation detail.

## Decision Entry - 2026-06-03 (Tokio Reopened, Deferred)

- **What was decided:** Reopened the Tokio question and deferred the call until angra accumulates a real async workload (e.g. concurrent background indexer, watch-mode resolver, or live registry streaming). Re-evaluation trigger: a concrete async-shaped feature lands in the roadmap.
- **Why:** Current `thread::scope` + `mpsc` work-queue in `Resolver::ensure_artifacts_parallel` already hits the 5.7x-over-Maven target, and the blocking `reqwest` path has no measured cold-network pain. Adopting Tokio now would add compile-time and dependency weight without a clear bottleneck to solve.
- **Rejected and why:** Adopting Tokio speculatively (ahead of an async feature) was rejected because it would couple the project to a runtime before a real driver exists, mirroring the 0.2 rejection at MEMORY.md:185. Switching only the parallel-fetch loop to async was rejected for now because it would split the codebase into two concurrency styles with no measured benefit.

## Decision Entry - 2026-06-09 (Frozen Resolve Implementation)

- **What was decided:** Implemented `angra resolve --frozen` as the lockfile-authoritative CI install path. Drift detection uses a new optional `manifest_fingerprint` field in `angra.lock`: SHA-256 over a versioned canonical rendering of resolver-relevant manifest intent (declared dependencies, dependency management, project repositories with policies, `[resolver.maven]` controls), computed from parsed declarations rather than raw TOML. Frozen mode skips all version/metadata resolution, fetches missing locked artifacts through the existing repository machinery, verifies every artifact (cached or downloaded) against its locked SHA-256, never rewrites the lockfile, and `--frozen` conflicts with `--refresh` at the CLI.
- **Why:** Without an authoritative-lock mode the lockfile is informational; CI needs "install the lock, fail loudly on drift". Hashing parsed intent keeps formatting/comment edits from invalidating locks, and equivalent compact/structured declarations fingerprint identically. Excluding global config and settings.xml keeps lockfiles portable across machines.
- **Rejected and why:** Hashing raw manifest text was rejected (false drift on cosmetic edits). Checking that each declared root appears in the lock was rejected (cannot detect removals without provenance). Recording declared roots in the lockfile was rejected as a larger format change than needed. Two logged decisions were touched deliberately: the fingerprint is an input digest, not resolver provenance, so the artifact-only lockfile decision stands; and frozen mode re-verifies cached artifact hashes as an explicit, opt-in exception to the warm-cache no-reverify rule — normal `resolve`/`lock` behavior is unchanged.

## Decision Entry - 2026-06-09 (Roadmap Review Additions)

- **What was decided:** After a roadmap review: (1) lockfile-driven installs (`angra resolve --frozen`), `angra outdated`, `angra update [alias]`, and explicit workspace-root resolve semantics were added to 0.3 scope, with permission to split `update`/`outdated` into a named 0.3.x if they would block shipping; (2) basic test/clippy/fmt CI was pulled forward from 1.0 into 0.3; (3) scoped classpath construction (compile/`provided`, test-compile, test-runtime with Maven scope inheritance) and `angra test --filter` were added to 0.4 scope; (4) a new 0.6 milestone covers private-repository basic auth from `settings.xml` `<servers>` plus env-var credential references, moved up from the deferred Maven long tail; (5) `--json` output for `tree`/`why` joined the 1.0 CLI UX bullet; (6) Windows support became an explicit deferral past 1.0.
- **Why:** Lockfile-authoritative installs and a dependency-upgrade story are core uv ergonomics that were absent from the path. Private-repo auth is the biggest corporate adoption gate after plugin execution, and 1.0 is "no new capabilities", so auth needed its own pre-1.0 milestone. CI at 1.0 was too late given the per-milestone verification protocol already depends on those checks. The compile/test classpath split is resolver work that was invisible in the 0.4 bullets. Windows was the only platform question the roadmap was silent on.
- **Rejected and why:** Folding auth into 1.0 hardening was rejected because it contradicts 1.0's no-new-capabilities framing. Encrypted `settings-security.xml`, proxies, and token schemes stay deferred to keep 0.6 small. Committing to Windows CI/binaries now was rejected as a conscious cut pending adoption interest; nothing in the codebase blocks it later.

## Decision Entry - 2026-06-09 (0.3 Migration-First Start)

- **What was decided:** Started 0.3 as a Maven migration-first milestone. `import-pom` generates effective concrete TOML, records Maven modules as read-only `[workspace] members`, and warns for Maven build/plugin/reporting/publishing behavior rather than guessing.
- **Why:** Existing Java projects are the likely adoption path; a useful importer reduces migration friction while preserving Angra's low-ceremony TOML surface.
- **Rejected and why:** Source-shaped XML mirroring was rejected because it would carry Maven ceremony into Angra. Persisting resolver provenance in `angra.lock` was rejected to preserve the artifact-only lockfile decision; `tree` and `why` use in-memory graph data instead. Importing relative-only parent POMs as BOM imports was rejected because those parents are not repository-resolvable from `angra.toml`; Angra warns and keeps concrete dependencies instead.
