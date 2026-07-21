# Onmark

Onmark is a screenplay-first video compiler and rendering engine for people and
agents.

```text
screenplay → deterministic Timeline IR → browser frames + audio plan → MP4
```

Delivery gates one through six are complete. Gate one renders and
independently verifies one real screenplay through the compiler, browser
protocol, Chromium, and FFmpeg. Gate two partitions one media-bearing two-shot
film into two local Render Units and proves that their assembled decoded video
and audio match the whole-film baseline. Gate three sends those same portable
units to two concurrent arm64 Lambda workers, verifies their immutable S3 frame
artifacts against a remote whole-film capture by canonical raw-RGBA pixels, and
assembles the partitions through the shared H.264/AAC path. Gate four carries
authored music, shot-local effects, voice-over, and imported SRT/WebVTT/ASS
captions through the same local whole-film and partitioned paths, with both
canonical raw-RGBA frames and decoded audio checked for equivalence.

Gate five admits exact-frame effects and a closed presentation temporal
capability after bounded WAAPI, GSAP, and Three.js experiments. Its exit
conformance proves that the effect-bearing presentation produces identical
canonical pixels as one whole-film unit or two independent units. Unknown
presentation code remains sequential. Deployment work remains frozen.

Gate six carries content-addressed local image, SVG, and font resources through
bounded browser readiness that names the resource still pending. It also gives
the existing closed overlay facts Rust-owned component identity that remains
stable across unit projections. Its Linux exit conformance proves equal pixels
across cold browsers and whole-film versus partitioned execution without adding
a general props channel, new screenplay spelling, or a second timing system.

The completed foundation includes the pure compiler and versioned wire types in
`onmark-core`; bounded ffprobe and strict SubRip/WebVTT/ASS normalization in
`onmark-media`; deterministic video and overlay presentation in
`@onmark/runtime`; semantic DOM bindings in `@onmark/authoring`; immutable
presentation artifacts in `@onmark/bundler`; the typed Chromium-to-FFmpeg
executor in `onmark-render`; and the `onmark-cli` composition root. A production
deployment workflow and infrastructure definition remain deliberately absent;
there is still no queue, lease system, scheduler, or coordinator.

## Render

The native command discovers `presentation.ts` beside the screenplay and writes
a no-clobber `renders/<name>.mp4` by default:

```bash
onmark render film.onmark
onmark render film.onmark --presentation browser.ts --output review.mp4
onmark render film.onmark --fps 30000/1001 --width 1920 --height 1080
onmark render film.onmark --subtitle captions.vtt
onmark render film.onmark --temporal-capability randomAccess
```

`--subtitle` imports strict UTF-8 `.srt`, `.vtt`, or `.ass` files without adding
external-format syntax to the screenplay. Invalid files produce diagnostics
against their own path and byte spans before browser or media processes start.

`--temporal-capability` defaults to `sequential`. Use `randomAccess` only after
conformance proves that every requested frame depends solely on immutable input
and that exact frame; the declaration changes bundle identity and permits the
Render Graph to split shot-scoped units.

Rendering requires `onmark-bundle` and its Node.js runtime, Chrome for Testing's
`chrome-headless-shell`, `ffmpeg`, and `ffprobe` to be installed. The renderer
requires headless shell's CDP BeginFrameControl and does not fall back to
ordinary Chrome. CDP does not currently support BeginFrameControl on macOS, so
native macOS rendering requires a Linux worker or container. Use the execution
override flags shown by `onmark render --help` when tools are not on the default
paths.

## Worker capture

Gate three introduced a narrow local worker entry point for already-composed
visual work:

```bash
onmark worker capture --input worker-input --output opening.onmark-frames --browser /path/to/chrome-headless-shell
```

`worker-input` contains a versioned `request.json`, including the locked
capture-environment identity, the `bundle/` payload files named by that
request's manifest, and any frozen `assets/sha256/` bytes. The worker accepts no
screenplay and does not compile source. It publishes one checksum-verified,
no-clobber frame artifact; retry reuse requires both the planned unit and the
declared capture environment to match. This command proves the future worker
interchange locally—it is not a cloud coordinator or a replacement for
`onmark render`.

## Lambda capture adapter

`deploy/aws-lambda` wraps that same worker contract with a bounded S3 download
and conditional artifact publication. Its V1 invocation and result schemas are
checked in under `schemas/`; its required environment, IAM scope, limits, and
intentional non-goals are documented in
[its deployment README](deploy/aws-lambda/README.md). The real arm64 Lambda ZIP
experiment and deterministic package command are recorded there. Infrastructure
provisioning and a published release workflow remain outside this gate.

## Repository map

- `crates/` contains Rust product code.
- `packages/` contains browser and Node product packages.
- `conformance/` contains behavior examples shared across implementations.
- `evals/` contains frozen language-admission experiments and raw model output.
- `schemas/` contains generated, versioned wire contracts.
- `scripts/` contains repository-only generation and quality checks.

## Development

Rust 1.97.0, Node.js 26.4.0, and pnpm 11.9.0 are pinned for reproducible
development.

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
cargo xtask eval audio
```

## Design documents

- [Architecture](docs/en/architecture.md)
- [Language specification](docs/en/language-specification.md)
- [Presentation contract](docs/en/presentation-contract.md)
- [Rust style guide](docs/en/rust-style-guide.md)
- [TypeScript style guide](docs/en/typescript-style-guide.md)
- [中文文档](docs/zh-CN/)

The design documents remain the project contract. Code and documentation
disagreements must be resolved explicitly rather than silently choosing one
side.
