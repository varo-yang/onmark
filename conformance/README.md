# Conformance fixtures

Authored `.onmark` inputs are maintained by hand. Expected `.ast.txt`,
`.linked.txt`, `.resolved.txt`, `.timeline.txt`, and `.diagnostics.txt` files
are generated golden artifacts and are not wire formats or protocol schemas.

`subtitle/` fixtures exercise standalone subtitle normalization independently
of screenplay syntax. Their `.captions.txt` and `.errors.txt` files are test
renderings, not Timeline IR or a public caption wire format.

Files under `protocol/` are different: they are checked-in wire examples and
therefore part of versioned cross-process contracts. Browser request/response
examples are maintained through the protocol conformance test. Bundle fixture
directories also retain their payload bytes so native materialization verifies
the declared size, digest, identity, and entry document together. Review all of
these files as compatibility-sensitive data.

`protocol/bundle-v1/` preserves the legacy sequential contract.
`protocol/bundle-v2/` is the current self-contained random-access fixture used
by the native Chromium-to-FFmpeg smoke. It is generated from
`browser/video-presentation.ts` by `@onmark/bundler`, embeds the production
authoring bindings and runtime presentation adapter, and consumes materialized
media from the unit root. Gate six adds one licensed local font and one authored
SVG to this same fixture; browser preparation must decode or load both before
capture. An exact-frame effect changes the poster accent from the absolute frame
identity, so whole-film and partition capture also prove the admitted
random-access lifecycle. The bundler test recursively rebuilds and compares
every byte, so source, runtime, manifest, and nested payload cannot drift
independently.

Regenerate goldens after intentionally changing public behavior:

```bash
ONMARK_UPDATE_GOLDENS=1 cargo test -p onmark-core --test syntax_conformance
ONMARK_UPDATE_GOLDENS=1 cargo test -p onmark-core --test binding_conformance
ONMARK_UPDATE_GOLDENS=1 cargo test -p onmark-core --test resolution_conformance
ONMARK_UPDATE_GOLDENS=1 cargo test -p onmark-core --test timeline_conformance
ONMARK_UPDATE_GOLDENS=1 cargo test -p onmark-core --test protocol_conformance
```

Review the resulting diff before committing it. Normal test runs compare
current behavior with the checked-in artifacts and never rewrite them.

`browser/gate-one.html` is a real Chromium fixture, not a golden file. Build
`@onmark/runtime`, set `ONMARK_HEADLESS_SHELL` to the pinned headless-shell
executable, and run:

```bash
ONMARK_HEADLESS_SHELL=/path/to/chrome-headless-shell \
  cargo test -p onmark-render --test render \
  captures_stable_raw_rgba_frames_across_independent_browser_sessions -- --ignored
```

The smoke crosses the versioned browser protocol, captures two distinct frames,
and requires a repeated capture of the same frame to produce identical PNG bytes.

The full local-render smoke generates and probes a real H.264 source, verifies
its frozen identity during unit materialization, decodes it through Chromium,
streams every captured frame through `FFmpeg`, probes the published MP4, and
requires decoded frame hashes to prove that the result contains motion:

```bash
ONMARK_HEADLESS_SHELL=/path/to/chrome-headless-shell \
ONMARK_FFMPEG=/path/to/ffmpeg \
ONMARK_FFPROBE=/path/to/ffprobe \
cargo test -p onmark-render --test render \
  renders_the_gate_one_plan_to_a_verified_mp4 -- --ignored
```

`cli/gate-one.onmark` drives the outermost Gate-one contract. The CLI smoke
copies that screenplay and the production presentation into a private
workspace, generates its referenced video, and invokes the real `onmark`
binary twice. It verifies each independent Chromium and `FFmpeg` session's
decoded frame count, motion, stream facts, and audio placement, then proves
that a third invocation cannot replace an existing output. Canonical raw-RGBA
equality is asserted before lossy MP4 encoding, not inferred from independently
encoded output:

```bash
ONMARK_CHROME=/path/to/chrome \
ONMARK_BUNDLER=/path/to/onmark-bundle \
ONMARK_FFMPEG=/path/to/ffmpeg \
ONMARK_FFPROBE=/path/to/ffprobe \
cargo test -p onmark-cli --test render -- --ignored
```

CI runs all real-process conformance on Ubuntu 24.04 with Chrome for Testing
149.0.7827.55 and Ubuntu's `FFmpeg` 7:6.1.1-3ubuntu5. Exact executable paths
are supplied to every test; neither the runner's browser nor an ambient media
tool can silently change the measured environment.

Gate seven's layered-media candidate remains test-only. Shared Linux CI runs
its cold-repeatability, whole-versus-partition, and frozen BT.709 patch-bound
checks. The performance gate is deliberately separate because shared-runner
noise must not admit a production pixel path. On the pinned admission machine,
build the runtime and run the five alternating 1,920×1,080 baseline/candidate
samples explicitly:

```bash
pnpm --filter @onmark/runtime build
ONMARK_HEADLESS_SHELL=/path/to/pinned/chrome-headless-shell \
ONMARK_FFMPEG=/path/to/pinned/ffmpeg \
ONMARK_FFPROBE=/path/to/pinned/ffprobe \
ONMARK_CAPTURE_ENVIRONMENT=sha256:<locked-environment-digest> \
ONMARK_MEDIA_EXPERIMENT_WIDTH=1920 \
ONMARK_MEDIA_EXPERIMENT_HEIGHT=1080 \
cargo test -p onmark-render --test media_seek \
  admission::performance::meets_performance_thresholds \
  -- --exact --ignored --nocapture --test-threads=1
```

The test prints every raw timing/RSS sample, the two medians, the frozen source
digest, and the capture-environment identity. A reviewed evidence record is
required before the candidate can leave the experiment target.
