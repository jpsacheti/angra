# Angra Roadmap

Angra aims to be a fast, Maven-compatible Java project tool — `uv` ergonomics for Java, TOML manifests, minimal ceremony.

This roadmap covers milestones **0.1 through 1.0**, ending at a usable replacement for everyday Maven workflows (resolve, add deps, build, test, run, package). Publishing, built-in JDK management, IDE plugins, and the long tail of Maven feature parity are deferred — see the [Deferred](#deferred) section. The decision log lives in [MEMORY.md](MEMORY.md).

- **Last updated:** 2026-05-21
- **Current milestone:** 0.2 (planned — work has not started)

## Status legend

| Tag | Meaning |
| --- | --- |
| **[shipped]** | Capability is in `master` and covered by tests. |
| **[in progress]** | Active development on this milestone. |
| **[planned]** | Agreed scope; not yet started. |
| **[deferred]** | Out of the 0.1 → 1.0 path. May return later. |

---

## 0.1 — Resolver MVP **[shipped]**

In `master` today:

- `angra resolve` with compact and structured deps — `src/manifest.rs`
- Maven Central download + `~/.m2` layout — `src/maven.rs`, `src/resolver.rs`
- Runtime graph traversal: nearest-wins, exclusions, optional filtering — `src/resolver.rs`
- Deterministic `angra.lock` with SHA-256 — `src/lockfile.rs`
- Comparative bench harness vs Maven and Gradle through `mise` — `src/benchmark.rs`, `src/bin/angra-bench.rs`

**Exit criteria:** met.

---

## 0.2 — Resolver realism **[planned]**

**Goal:** Resolve any real-world Java project (Spring Boot starters, Jackson, Guava, Netty) without surprises. Today the resolver bails on `${...}` properties — that is the next wall every user hits.

**Scope:**

- POM property interpolation: `${project.version}`, user-defined `<properties>`. Replace the `UnsupportedPomProperty` bailout in `src/resolver.rs`.
- Parent POM inheritance: recursive `<parent>` resolution, merged properties + `<dependencyManagement>`.
- `<dependencyManagement>` honored in the transitive graph for version pinning.
- BOM imports (`<scope>import</scope>` inside `<dependencyManagement>`).
- Classifier + packaging (`pom`, `jar`, `war`).
- Mirror + repository config from `~/.m2/settings.xml` (read-only — auth deferred).
- Parallel artifact downloads. Current `download()` in `src/resolver.rs` is sequential blocking; introduce a worker pool or switch to async `reqwest`.
- Checksum verification against `.sha1` sibling files on Maven Central.
- Failure attribution: when a coord fails, print the dependency path that pulled it in.

**Critical files:** `src/resolver.rs` (most of the change), `src/maven.rs` (`.sha1` URLs, classifiers), likely a new `src/pom.rs` extracted from inline POM types in `resolver.rs`.

**Exit criteria:** A `spring-boot-starter-web` fixture resolves cleanly and matches Maven's resolution set. Bench harness gains a `spring-boot` case.

---

## 0.3 — Manifest lifecycle **[planned]**

**Goal:** Make `angra.toml` editable through commands the way `uv` edits `pyproject.toml`. After 0.3 a developer can start fresh, add/remove deps, inspect the graph, and migrate from Maven without ever hand-editing the manifest.

**Scope:**

- `angra init` — scaffold an `angra.toml`.
- `angra add <coord> [--scope=runtime|test|provided]` — append, re-resolve, update lockfile.
- `angra remove <alias>` — remove, re-resolve.
- `angra lock` — re-resolve without manifest changes.
- `angra tree` — print resolved graph.
- `angra why <coord>` — print the path that brought a coord in (reuses path-tracking from 0.2).
- `angra import-pom <path>` — one-way migration from existing `pom.xml`. Lossy by design; flagged in output.
- Workspace primitive: `[workspace] members = [...]`. Read-only at this stage — orchestration arrives in 0.4.

**Critical files:** `src/main.rs` (subcommands), likely a new `src/commands/` module tree, `src/manifest.rs` (writer in addition to current reader), new `src/workspace.rs`.

**Exit criteria:** A developer can `angra init`, `angra add com.google.guava:guava:33.0.0-jre`, `angra add junit:junit:4.13.2 --scope=test`, `angra tree`, `angra why com.google.guava:guava` — all without opening the TOML. `angra import-pom` on the bench `pom.xml` fixtures produces an equivalent `angra.toml`.

---

## 0.4 — Compile & test **[planned]**

**Goal:** Replace `mvn compile` and `mvn test` for a conventional Java project. First milestone where Angra runs a JVM.

**Scope:**

- `[toolchain] jdk = "21"` in `angra.toml`. Angra invokes `mise x java@21 -- javac ...` with SDKMan fallback. Clear error if neither tool is installed.
- Conventional source layout: `src/main/java`, `src/test/java`, `src/main/resources`, `src/test/resources` — overridable in `[build]`.
- `angra build` — compile `src/main/java` against the resolved compile classpath, output to `target/classes`.
- `angra test` — compile `src/test/java`, then run JUnit 5 via `org.junit.platform.console.ConsoleLauncher` (auto-added to test scope). Non-zero exit on failure.
- `angra clean` — wipe `target/`.
- Incremental compile: skip `javac` when no `.java` source is newer than its `.class` output. Coarse per-module is fine.
- Annotation processors: `[build] annotation-processors = ["org.projectlombok:lombok:1.18.34"]` → `-processorpath`. Required for Lombok/MapStruct.
- Workspace build: `angra build` at workspace root builds members in topological order.

**Critical files:** New `src/build.rs`, `src/test.rs`, `src/toolchain.rs`. `src/main.rs` gains `build`/`test`/`clean`. `src/manifest.rs` extended with `[toolchain]` and `[build]`.

**Exit criteria:** A small project — Spring Boot starter or hand-rolled — compiles and tests with `angra build && angra test`. Bench harness compares `angra test` vs `mvn test` on at least one fixture. Lombok-using fixture works end-to-end.

---

## 0.5 — Package & run **[planned]**

**Goal:** Close the inner loop. After 0.5, a developer can scaffold → add deps → compile → test → package → run, all through Angra.

**Scope:**

- `angra package` — produce `target/<artifact>-<version>.jar` with compiled classes + `src/main/resources`. Manifest includes `Main-Class` when `[run].main-class` is set.
- `angra run [-- arg1 arg2]` — launch via the JDK toolchain. `[run]` table: `main-class`, `args`, `jvm-args`, `env`.
- `angra run --class <fqcn>` — ad-hoc main override without editing the manifest.
- Optional fat-jar mode: `[package] kind = "uber"` flattens runtime classpath. Conservative v0 — concatenate entries; document the `META-INF/services` limitation.
- Resource copying: `src/main/resources` → jar verbatim. No `${...}` filtering at this stage (called out explicitly — Maven users will ask).

**Critical files:** New `src/package.rs`, `src/run.rs`. `src/manifest.rs` gains `[run]` and `[package]`.

**Exit criteria:** A "hello world" project goes from `angra init` through `angra run` in one session. `angra package` produces a runnable jar. A slim Spring Boot fixture builds and runs (uber-jar with Spring Boot's nested-jar layout is deferred).

---

## 1.0 — Hardening **[planned]**

**Goal:** What's shipped is reliable, documented, and packaged. No new capabilities — purely about earning the version number.

**Scope:**

- `angra.lock` format frozen; documented compatibility policy.
- Error message audit — every error variant gives an actionable next step.
- CLI UX consistency: `--help` for every subcommand, consistent flag naming, documented exit codes.
- Documentation: real `docs/` with per-command pages and a Maven migration guide.
- Bench fixtures expanded: spring-boot-starter-web, jackson, netty, a 100-dep stress fixture. Summary in README.
- CI: GitHub Actions running `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, bench on every PR (non-gating).
- Prebuilt binaries: macOS arm64/x86_64, Linux x86_64/arm64. Homebrew tap.
- Resolver edge-case bug bash: empty POMs, malformed POMs, cycles, large fan-out, classifier collisions.

**Critical files:** `.github/workflows/`, `docs/`, `README.md`, `CONTRIBUTING.md`. No new source modules.

**Exit criteria:** Someone unfamiliar with the project can `brew install angra`, follow the docs, migrate a small Maven project, and ship it. No `TODO`/`FIXME` comments referencing 0.x behavior remain in `src/`.

---

## Deferred

Intentionally not on the 0.1 → 1.0 path:

- **Publishing**: `angra publish` to Maven Central / private repos, GPG signing, staging.
- **Built-in JDK management**: delegation to `mise` / SDKMan is the long-term answer through 1.0.
- **IDE integration**: IntelliJ plugin, LSP server, Eclipse `.classpath` export beyond `angra tree --machine-readable`.
- **Maven feature parity (long tail)**: profiles, version ranges (`[1.0,2.0)`), mirrors with auth, encrypted `settings-security.xml`, snapshot timestamping nuance.
- **Maven plugin execution**: arbitrary `<build><plugins>` from imported POMs. Angra will not host Maven's plugin model. Users needing Surefire/Failsafe/Shade behavior beyond what 0.4/0.5 covers should keep Maven for those modules.
- **Multi-language**: Kotlin, Scala, Groovy, mixed-source modules.
- **Reproducible builds**: bit-for-bit jar reproducibility, normalized timestamps.

---

## Verification (per milestone)

Each milestone exits when **all** of the following pass:

1. `cargo test` and `cargo clippy --all-targets -- -D warnings` green.
2. A fixture project demonstrating the exit criteria builds end-to-end via `cargo run -- <subcommand>`.
3. `cargo run --bin angra-bench -- --repo . --angra-binary target/release/angra` includes the milestone's fixture; Angra is not slower than the prior milestone on existing fixtures.
4. `MEMORY.md` has a dated entry summarizing what shipped, what was rejected, and remaining gaps — matching the pattern at `MEMORY.md` lines 21–42.

End-to-end smoke for the whole roadmap once 1.0 lands: take a real open-source Maven project (proposal: `picocli` or a small Spring Boot sample), run `angra import-pom`, then `angra build && angra test && angra package && angra run`. Confirm parity with `mvn` output.

---

## Update protocol (for future sessions)

This document is intended to be edited as work happens. When updating:

1. **Status tags:** When a milestone is in active work, switch its tag to **[in progress]**. When it ships, switch to **[shipped]** and update the "Current milestone" line at the top.
2. **Scope drift:** If scope shifts during a milestone, edit the bullets in-place rather than leaving the doc aspirational. Items pushed out of a milestone move to the next one or to [Deferred](#deferred) — do not silently drop them.
3. **Last updated:** Bump the `Last updated` line on every meaningful edit.
4. **Decision log:** Any non-trivial change (milestone reordering, exit-criteria change, deferral) requires a corresponding dated entry in [MEMORY.md](MEMORY.md) following the existing `What was decided / Why / What was rejected and why` structure.
5. **Exit-criteria honesty:** Don't flip a milestone to **[shipped]** until the four verification checks above hold. If only part of the scope landed, edit the scope list to match reality and either bump the milestone version or open a follow-up milestone.
6. **Deferred is not a graveyard:** Items in [Deferred](#deferred) can return to a planned milestone later — when that happens, move the bullet up rather than duplicating it.
