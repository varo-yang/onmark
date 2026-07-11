# Onmark

Onmark is a screenplay-first video compiler and rendering engine for people and agents.

```text
screenplay → deterministic Timeline IR → browser frames → encoded video
```

The project is currently in design and delivery gate one: rendering one real video reliably through the final-direction compiler and browser protocol. The implemented foundation currently includes the pure domain model, structured authored diagnostics, span-preserving screenplay syntax, and the compiler-facing parse report in `onmark-core`.

## Development

Rust 1.97.0 is pinned for both standard Rust tooling and mise users.

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
cargo fmt --all -- --check
```

## Design documents

- [Architecture](docs/en/architecture.md)
- [Language specification](docs/en/language-specification.md)
- [Rust style guide](docs/en/rust-style-guide.md)
- [中文文档](docs/zh-CN/)

The design documents remain the project contract. Code and documentation disagreements must be resolved explicitly rather than silently choosing one side.
