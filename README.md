# Onmark

Onmark is a screenplay-first video compiler and rendering engine for people and agents.

```text
screenplay → deterministic Timeline IR → browser frames → encoded video
```

The project is currently in design and delivery gate one: rendering one real video reliably through the final-direction compiler and browser protocol. The implemented foundation includes the pure compiler and versioned wire types in `onmark-core`; bounded ffprobe normalization in `onmark-media`; deterministic browser execution in `@onmark/runtime`; immutable presentation artifacts in `@onmark/bundler`; the typed Chromium-to-FFmpeg executor in `onmark-render`; and the first whole-film `onmark-cli` composition root. Production presentation authoring remains Gate-one work.

## Render

The native command discovers `presentation.ts` beside the screenplay and writes a no-clobber `renders/<name>.mp4` by default:

```bash
onmark render film.onmark
onmark render film.onmark --presentation browser.ts --output review.mp4
onmark render film.onmark --fps 30000/1001 --width 1920 --height 1080
```

Gate one requires `onmark-bundle`, Chromium or Google Chrome, `ffmpeg`, and `ffprobe` to be installed. Use the execution override flags shown by `onmark render --help` when they are not on the default paths.

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
