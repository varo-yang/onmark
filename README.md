# Onmark

Onmark is a screenplay-first video compiler and rendering engine for people and agents.

```text
screenplay → deterministic Timeline IR → browser frames + audio plan → MP4
```

Delivery gates one and two are complete. Gate one renders and independently verifies one real screenplay through the final-direction compiler, browser protocol, Chromium, and FFmpeg. Gate two partitions one media-bearing two-shot film into two independently materialized local Render Units, executes both through the same renderer, and proves their assembled decoded video and audio match the whole-film baseline. Gate three began with a local worker-artifact conformance: independent worker processes capture bounded, verified frame artifacts, then the existing single encoder and final audio path assemble them equivalently to the whole-film baseline. Its next narrow slice is a Rust Lambda/S3 adapter that materializes the same portable worker request and conditionally publishes the same verified artifact; it is not yet a deployable Chromium image, queue, lease system, scheduler, or coordinator. The completed foundation includes the pure compiler and versioned wire types in `onmark-core`; bounded ffprobe normalization in `onmark-media`; deterministic video and overlay presentation in `@onmark/runtime`; reusable semantic DOM bindings in `@onmark/authoring`; immutable presentation artifacts in `@onmark/bundler`; the typed Chromium-to-FFmpeg executor in `onmark-render`; and the whole-film `onmark-cli` composition root. The checked-in production presentation renders video, title, and call-to-action facts without re-solving Rust-owned time; the native executor mixes solved voice-over after browser capture and muxes it into the final MP4.

## Render

The native command discovers `presentation.ts` beside the screenplay and writes a no-clobber `renders/<name>.mp4` by default:

```bash
onmark render film.onmark
onmark render film.onmark --presentation browser.ts --output review.mp4
onmark render film.onmark --fps 30000/1001 --width 1920 --height 1080
```

Rendering requires `onmark-bundle` and its Node.js runtime, Chromium or Google Chrome, `ffmpeg`, and `ffprobe` to be installed. Use the execution override flags shown by `onmark render --help` when they are not on the default paths.

## Worker capture

Gate three exposes a narrow local worker entry point for already-composed visual work:

```bash
onmark worker capture --input worker-input --output opening.onmark-frames --browser /path/to/chrome
```

`worker-input` contains a versioned `request.json`, including the locked capture-environment identity, the `bundle/` payload files named by that request's manifest, and any frozen `assets/sha256/` bytes. The worker accepts no screenplay and does not compile source. It publishes one checksum-verified, no-clobber frame artifact; retry reuse requires both the planned unit and the declared capture environment to match. This command proves the future worker interchange locally—it is not a cloud coordinator or a replacement for `onmark render`.

## Lambda capture adapter

`deploy/aws-lambda` wraps that same worker contract with a bounded S3 download
and conditional artifact publication. Its V1 invocation and result schemas are
checked in under `schemas/`; its required environment, IAM scope, limits, and
intentional non-goals are documented in
[its deployment README](deploy/aws-lambda/README.md). A production Chromium
Lambda image remains an explicit experiment rather than an implied claim.

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
- [Presentation contract](docs/en/presentation-contract.md)
- [Rust style guide](docs/en/rust-style-guide.md)
- [TypeScript style guide](docs/en/typescript-style-guide.md)
- [中文文档](docs/zh-CN/)

The design documents remain the project contract. Code and documentation disagreements must be resolved explicitly rather than silently choosing one side.
