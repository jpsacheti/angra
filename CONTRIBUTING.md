# Contributing To Angra

Angra is early-stage software. Contributions should preserve the project shape:
Maven compatibility, low overhead, TOML ergonomics, and a fast developer
experience.

## Development

Run the test suite:

```sh
cargo test
```

Run Clippy with warnings denied:

```sh
cargo clippy --all-targets -- -D warnings
```

Format code:

```sh
cargo fmt
```

## Benchmarks

Every meaningful feature should include or update a benchmark path when it is
reasonable to compare the behavior with Maven and Gradle.

Run benchmarks:

```sh
cargo build
cargo run --bin angra-bench -- --repo . --angra-binary target/debug/angra
```

The benchmark harness uses:

```sh
mise x maven@latest -- mvn ...
mise x gradle@latest -- gradle ...
```

Do not commit generated benchmark results by default.

## Design Principles

- Prefer explicit behavior over hidden framework magic.
- Keep abstractions small until repetition or complexity justifies them.
- Optimize for Maven ecosystem compatibility without copying Maven ceremony.
- Treat performance as a product feature.
- Fail clearly for unsupported Maven behavior.
- Avoid YAML unless there is a strong compatibility reason.

## Pull Request Checklist

- Tests pass with `cargo test`.
- Clippy passes with `cargo clippy --all-targets -- -D warnings`.
- Code is formatted with `cargo fmt`.
- User-facing behavior is documented when it changes.
- Benchmarks are added or updated for new comparable features.
- Unsupported Maven behavior fails clearly.
