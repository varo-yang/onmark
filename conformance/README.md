# Conformance

This directory contains executable product evidence, organized by its owner:

- `compiler/` contains source and golden artifacts for each pure compiler phase;
- `media/` contains standalone media-normalization fixtures;
- `browser/` and `cli/` contain handwritten real-process inputs;
- `protocol/` contains versioned wire examples and self-contained bundles; and
- `evidence/` records the measurements and commands that admitted production
  behavior.

Authored `.html` inputs are maintained by hand. Expected `.ast.txt`,
`.linked.txt`, `.resolved.txt`, `.timeline.txt`, and `.diagnostics.txt` files
are generated golden artifacts and are not wire formats or protocol schemas.
Each compiler phase owns a self-contained input/golden pair; identical authored
inputs may therefore appear in more than one phase without sharing mutable test
state.

`media/subtitle/` fixtures exercise standalone subtitle normalization
independently of screenplay syntax. Their `.captions.txt` and `.errors.txt`
files are test renderings, not Timeline IR or a public caption wire format.

Files under `protocol/` are different: they are checked-in wire examples and
therefore part of versioned cross-process contracts. Browser request/response
examples are maintained through the protocol conformance test. Bundle fixture
directories also retain their payload bytes so native materialization verifies
the declared size, digest, identity, and entry document together. Review all of
these files as compatibility-sensitive data.

`protocol/bundle-v1/` is the current self-contained random-access fixture used
by the native Chromium-to-FFmpeg smoke. It is generated from
`browser/video-presentation.html` by `@onmark/bundler`. It embeds the production
authored-DOM bindings and runtime presentation adapter, and consumes
materialized media from the unit root. The bundler test recursively rebuilds
and compares every byte, so source, runtime, and manifest cannot drift
independently. Local font, image, GSAP, and Three.js imports are covered by
focused bundler or temporal fixtures instead of making the primary media smoke
serve unrelated contracts. Bundle artifacts are ephemeral build products; only
the current manifest is accepted, and older artifacts are rebuilt rather than
supported by compatibility branches.

Regenerate goldens after intentionally changing public behavior:

```bash
ONMARK_UPDATE_GOLDENS=1 cargo test -p onmark-core --test syntax_conformance
ONMARK_UPDATE_GOLDENS=1 cargo test -p onmark-core --test binding_conformance
ONMARK_UPDATE_GOLDENS=1 cargo test -p onmark-core --test resolution_conformance
ONMARK_UPDATE_GOLDENS=1 cargo test -p onmark-core --test timeline_conformance
ONMARK_UPDATE_GOLDENS=1 cargo test -p onmark-core --test render_graph_conformance
ONMARK_UPDATE_GOLDENS=1 cargo test -p onmark-core --test protocol_conformance
```

Review the resulting diff before committing it. Normal test runs compare
current behavior with the checked-in artifacts and never rewrite them.

`browser/runtime-protocol.html` is a real Chromium fixture, not a golden file.
Build `@onmark/runtime`, set `ONMARK_HEADLESS_SHELL` to the pinned
headless-shell executable, and run:

```bash
ONMARK_HEADLESS_SHELL=/path/to/chrome-headless-shell \
  cargo test -p onmark-render --test render \
  captures_stable_raw_rgba_frames_across_independent_browser_sessions -- --ignored
```

The smoke crosses the versioned browser protocol, captures two distinct frames,
and requires a repeated capture of the same frame to produce identical PNG bytes.

On macOS, the opt-in platform-graphics check uses the portable screenshot
backend, reads the active renderer back through CDP, and rejects a software
fallback. It compares independent Metal sessions over out-of-order WAAPI, GSAP,
and Three.js frames, while retaining a `SwiftShader` sequence to prove the two
graphics environments are not being conflated:

```bash
ONMARK_BUNDLER=/path/to/onmark-bundle \
ONMARK_PORTABLE_CHROME=/path/to/chrome \
cargo test -p onmark-render --test render \
  seeks_dynamic_frames_deterministically_on_metal \
  -- --exact --ignored
```

The full local-render smoke generates and probes a real H.264 source, verifies
its frozen identity during unit materialization, decodes it through Chromium,
streams every captured frame through `FFmpeg`, probes the published MP4, and
requires decoded frame hashes to prove that the result contains motion:

```bash
ONMARK_HEADLESS_SHELL=/path/to/chrome-headless-shell \
ONMARK_FFMPEG=/path/to/ffmpeg \
ONMARK_FFPROBE=/path/to/ffprobe \
cargo test -p onmark-render --test render \
  renders_the_browser_plan_to_a_verified_mp4 -- --ignored
```

`cli/desktop-release.html` drives the installed desktop contract. The release
smoke copies that complete authored document into a private workspace,
generates its referenced media, and invokes the real `onmark` binary twice. It
verifies each independent Chromium and `FFmpeg` session's decoded frame count,
motion, stream facts, and audio placement, then proves that a third invocation
cannot replace an existing output. Canonical raw-RGBA equality is asserted
before lossy MP4 encoding, not inferred from independently encoded output:

```bash
ONMARK_HEADLESS_SHELL=/path/to/chrome-headless-shell \
ONMARK_BUNDLER=/path/to/onmark-bundle \
ONMARK_FFMPEG=/path/to/ffmpeg \
ONMARK_FFPROBE=/path/to/ffprobe \
cargo test -p onmark-cli --test render -- --ignored
```

CI runs all real-process conformance on Ubuntu 24.04 with the Chrome for Testing
build recorded in `packages/launcher/desktop-release.json` and Ubuntu's
`FFmpeg` 7:6.1.1-3ubuntu5. Exact executable paths
are supplied to every test; neither the runner's browser nor an ambient media
tool can silently change the measured environment.

Gate seven's production layered-media path is enabled only for the explicit
`separableOverlay` capability. Shared Linux CI runs its cold-repeatability,
whole-versus-partition, frozen BT.709 patch-bound, and real-process exit checks.
The exit fixture also builds otherwise identical `everyFrame` and
`placementBounded` foreground bundles. It requires the latter to reduce
browser capture work from 75 authored frames to one while retaining exact
canonical raw-RGBA equality.
Either headless shell or ordinary Chrome may run that portable equivalence
check; the locked Linux suite remains the release authority.
The original performance admission remains reproducible but separate because
shared-runner noise must not decide a production pixel path. On the pinned
admission machine, run the five alternating 1,920×1,080 baseline/candidate
samples explicitly:

```bash
ONMARK_HEADLESS_SHELL=/path/to/pinned/chrome-headless-shell \
ONMARK_FFMPEG=/path/to/pinned/ffmpeg \
ONMARK_FFPROBE=/path/to/pinned/ffprobe \
ONMARK_CAPTURE_ENVIRONMENT=sha256:<locked-environment-digest> \
ONMARK_ISOLATED_WORKER=1 \
ONMARK_MEDIA_EXPERIMENT_WIDTH=1920 \
ONMARK_MEDIA_EXPERIMENT_HEIGHT=1080 \
cargo test -p onmark-render --test media_seek \
  admission::performance::meets_performance_thresholds \
  -- --exact --ignored --nocapture --test-threads=1
```

The test prints every raw timing/RSS sample, the two medians, the frozen source
digest, and the capture-environment identity. The reviewed admission and
production-exit evidence is recorded in
[`evidence/layered-media-admission.md`](evidence/layered-media-admission.md).

The incremental-rendering conformance keeps temporal seekability, DOM scope,
and artifact identity separate. It proves whole-versus-region raw-RGBA
equivalence, isolates a local title edit, prevents `:has()` from observing an
omitted sibling, and exercises persistent CLI reuse plus corruption repair
through the shared assembler. The evidence and exact commands live in
[`evidence/incremental-rendering.md`](evidence/incremental-rendering.md).

The exact-review evidence extends that contract to the authoring feedback loop.
It records cold, warm, and isolated-edit measurements; proves that review
captures ordinary complete production regions; and documents the bounded
manifest, contact-sheet, and PNG integrity contract. See
[`evidence/exact-review-loop.md`](evidence/exact-review-loop.md).

The production-campaign workload exercises twenty typed variants over one
screenplay, subtitle track, frozen asset set, and partition plan. It records
field-scope cache isolation separately from independent cold pixel
repeatability; the latter remains unadmitted for the complete mixed
browser-composited presentation. See
[`evidence/variant-campaign.md`](evidence/variant-campaign.md).
