# Onmark

Onmark is a screenplay-first video compiler and rendering engine for people and agents.

```text
screenplay → deterministic Timeline IR → browser frames → encoded video
```

The project is currently in design and delivery gate one: rendering one real video reliably through the final-direction compiler and browser protocol. The implemented foundation includes the pure domain model, structured authored diagnostics, span-preserving screenplay syntax, binding, typed resolution, timeline solving, and versioned browser and bundle wire types in `onmark-core`; bounded ffprobe metadata normalization in `onmark-media`; deterministic frame projection and the browser protocol session in `@onmark/runtime`; deterministic presentation artifacts in `@onmark/bundler`; and a typed `RenderUnit → ExecutableUnit` materialization boundary plus a Chromium-to-FFmpeg MP4 path in `onmark-render`. Production presentation authoring and the CLI composition root remain Gate-one work.

## Repository map

- `crates/` contains Rust product code.
- `packages/` contains browser and Node product packages.
- `conformance/` contains behavior examples shared across implementations.
- `schemas/` contains generated, versioned wire contracts.
- `scripts/` contains repository-only generation and quality checks.

## Development

Rust 1.97.0, Node.js 26.4.0, and pnpm 11.9.0 are pinned for reproducible development.

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
cargo fmt --all -- --check
pnpm install --frozen-lockfile
pnpm format:check
pnpm lint
pnpm typecheck
pnpm test
cargo xtask schema --check
```

## Design documents

- [Architecture](docs/en/architecture.md)
- [Language specification](docs/en/language-specification.md)
- [Rust style guide](docs/en/rust-style-guide.md)
- [TypeScript style guide](docs/en/typescript-style-guide.md)
- [中文文档](docs/zh-CN/)

The design documents remain the project contract. Code and documentation disagreements must be resolved explicitly rather than silently choosing one side.
