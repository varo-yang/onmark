# Onmark

Onmark is a screenplay-first video compiler and rendering engine for people and
agents.

```text
screenplay → deterministic Timeline IR → browser frames + audio plan → MP4
```

Delivery gates one through seven are complete. No later delivery gate is
currently active. Gate one renders and independently verifies one real
screenplay through the compiler, browser protocol, Chromium, and FFmpeg. Gate
two partitions one media-bearing two-shot film into two local Render Units and
proves that their assembled decoded video and audio match the whole-film
baseline. Gate three sends those
same portable units to two concurrent arm64 Lambda workers, verifies their
immutable S3 frame artifacts against a remote whole-film capture by canonical
raw-RGBA pixels, and assembles the partitions through the shared H.264/AAC
path. Gate four carries authored music, shot-local effects, voice-over, and
imported SRT/WebVTT/ASS captions through the same local whole-film and
partitioned paths, with both canonical raw-RGBA frames and decoded audio
checked for equivalence.

Gate five admits exact-frame effects and a closed presentation temporal
capability after bounded WAAPI, GSAP, and Three.js experiments. Its exit
conformance proves that the effect-bearing presentation produces identical
canonical pixels as one whole-film unit or two independent units. Unknown
presentation code remains sequential. Deployment work remains frozen.

Gate six carries content-addressed local image, SVG, and font resources through
bounded browser readiness that names the resource still pending. It also gives
film, scene, shot, and content facts Rust-owned node identity and ownership that
remain stable across unit projections. Its Linux exit conformance proves equal
pixels across cold browsers and whole-film versus partitioned execution without
adding a general props channel, new screenplay spelling, or a second timing
system.

Gate seven admits transparent browser presentation over persistent native media
decode and composition for an explicitly separable visual capability. Its
locked experiment passes at 4.18× the Chromium-media baseline speed and 79.73%
of its peak RSS. The production executor preserves a bounded stream, exact
whole-film/partition equivalence, declared color conformance, and the existing
Chromium-media path for presentations without the capability. The real-process
exit fixture passes; this is neither new screenplay syntax nor a hidden
fallback.

The executor now also carries an independent, proved browser-frame behavior.
For placement-bounded foregrounds it captures only the first frame and solved
placement changes, then shares the immutable PNG between intervening output
frames. The real-process layered control reduces browser evaluation from 75
authored frames to one while retaining exact canonical raw-RGBA output;
browser-owned video and unknown authored effects remain per-frame.

The completed foundation includes the pure compiler and versioned wire types in
`onmark-core`; bounded ffprobe and strict SubRip/WebVTT/ASS normalization in
`onmark-media`; deterministic video and overlay presentation in
`@onmark/runtime`; authored-HTML bindings in `@onmark/authoring`; immutable
presentation artifacts in `@onmark/bundler`; the typed Chromium-to-FFmpeg
executor in `onmark-render`; and the `onmark-cli` composition root. A production
deployment workflow and infrastructure definition remain deliberately absent;
there is still no queue, lease system, scheduler, or coordinator.

## Render

The native command needs one authored HTML document and writes a no-clobber
`renders/<name>.mp4` by default. Onmark custom elements carry screenplay intent;
ordinary HTML and inline CSS carry presentation. An optional inline
`type="module" data-om-motion` script exports vendor-neutral motion
assembled from adapters such as `onmark/motion/gsap`:

```bash
onmark render film.html
onmark render film.html --output review.mp4
onmark render film.html --fps 30000/1001 --width 1920 --height 1080
onmark render film.html --subtitle captions.vtt
```

`--subtitle` imports strict UTF-8 `.srt`, `.vtt`, or `.ass` files without adding
external-format syntax to the screenplay. Invalid files produce diagnostics
against their own path and byte spans before browser or media processes start.

Presentation capabilities are not command-line assumptions. Authored HTML is
conservatively sequential, browser-composited, and captured per frame. Stronger
capabilities belong to conformance-admitted artifacts and immutable bundle
metadata; they are never inferred from source text or observed pixel equality.

The desktop artifact is admitted on macOS arm64, Linux x64, and Windows x64,
although it has not yet been published to npm. It exposes one `onmark` package
and command, carries the native CLI, `ffmpeg`, and `ffprobe` in a platform
sidecar, and installs the pinned browser into a verified private cache. Linux
uses Chrome for Testing's `chrome-headless-shell` with CDP BeginFrameControl;
macOS and Windows use ordinary Chrome's portable screenshot backend. macOS
selects Metal and verifies the active renderer through CDP; Linux and Windows
retain `SwiftShader`. Every target renders the same admitted screenplay twice
in an empty consumer before release artifacts are retained. Execution overrides
remain explicit: `--graphics software` selects the canonical software control,
and `--video-encoder-threads` can replace the stable four-thread local default
with a bounded value from 1 through 64.
Deterministic comparisons apply within the same locked browser environment,
capture mode, and graphics backend.

## Worker capture

Gate three introduced a narrow local worker entry point for already-composed
visual work:

```bash
onmark worker capture --input worker-input --output opening.onmark-frames \
  --browser /path/to/chrome-headless-shell --ffmpeg /path/to/ffmpeg
```

`worker-input` contains a versioned `request.json`, including the locked
capture-environment identity, the `bundle/` payload files named by that
request's manifest, and any frozen `assets/sha256/` bytes. The worker accepts no
screenplay and does not compile source. It publishes one checksum-verified,
no-clobber frame artifact; retry reuse requires both the planned unit and the
declared capture environment to match. This is the same portable interchange
used by the Lambda adapter; it is not a cloud coordinator or a replacement for
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
cargo xtask eval html
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
