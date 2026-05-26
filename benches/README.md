# Benchmarks

Angra benchmark cases compare equivalent dependency-resolution work across:

- `angra resolve`
- Maven through `mise x maven@latest -- mvn ...`
- Gradle through `mise x gradle@latest -- gradle ...`

Gradle is included only for fixtures that contain a Gradle build. The
Spring Boot fixture under `benches/spring-fixture` is an Angra-vs-Maven
canary because it mirrors a real Maven-generated project and imports the
Spring Boot BOM through Angra-native `[dependency-management]`.

The Rust benchmark harness lives in `src/benchmark.rs`, and `angra-bench`
runs the checked-in fixture matrix while emitting structured JSON results.

Typical local run:

```sh
cargo build
cargo run --bin angra-bench -- --repo . --angra-binary target/debug/angra
```

The Maven and Gradle commands are executed as:

```sh
mise x maven@latest -- mvn dependency:go-offline
mise x gradle@latest -- gradle --no-daemon dependencies --configuration runtimeClasspath
```

The Spring fixture uses Maven `dependency:list -DincludeScope=runtime`
instead of `dependency:go-offline` so its Maven leg measures the same runtime
resolution surface as Angra.

Benchmark results are generated artifacts and should not be committed by
default.
