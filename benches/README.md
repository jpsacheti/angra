# Benchmarks

Angra benchmark cases compare equivalent dependency-resolution work across:

- `angra resolve`
- Maven through `mise x maven@latest -- mvn ...`
- Gradle through `mise x gradle@latest -- gradle ...`

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

Benchmark results are generated artifacts and should not be committed by
default.
