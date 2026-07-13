# Conformance fixtures

Authored `.onmark` inputs are maintained by hand. Expected `.ast.txt`,
`.linked.txt`, `.resolved.txt`, `.timeline.txt`, and `.diagnostics.txt` files
are generated golden artifacts and are not wire formats or protocol schemas.

Files under `protocol/` are different: they are checked-in wire examples and
therefore part of versioned cross-process contracts. Browser request/response
examples are maintained through the protocol conformance test. Bundle fixture
directories also retain their payload bytes so native materialization verifies
the declared size, digest, identity, and entry document together. Review all of
these files as compatibility-sensitive data.

`protocol/bundle-v1/` is also the self-contained executable fixture used by the
native Chromium-to-FFmpeg smoke. It is generated from
`browser/video-presentation.ts` by `@onmark/bundler`, embeds the production
runtime video adapter, and consumes materialized media from the unit root. The
bundler test rebuilds it and requires byte-for-byte equality, so source,
runtime, manifest, and payload cannot drift independently.

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
`@onmark/runtime`, set `ONMARK_CHROME` to an explicit Chrome executable, and run:

```bash
ONMARK_CHROME=/path/to/chrome cargo test -p onmark-render --test render \
  captures_stable_frames_across_the_real_browser_protocol -- --ignored
```

The smoke crosses the versioned browser protocol, captures two distinct frames,
and requires a repeated capture of the same frame to produce identical PNG bytes.

The full local-render smoke generates and probes a real H.264 source, verifies
its frozen identity during unit materialization, decodes it through Chromium,
streams every captured frame through `FFmpeg`, probes the published MP4, and
requires decoded frame hashes to prove that the result contains motion:

```bash
ONMARK_CHROME=/path/to/chrome \
ONMARK_FFMPEG=/path/to/ffmpeg \
ONMARK_FFPROBE=/path/to/ffprobe \
cargo test -p onmark-render --test render \
  renders_the_gate_one_plan_to_a_verified_mp4 -- --ignored
```

`cli/gate-one.onmark` drives the outermost Gate-one contract. The CLI smoke
copies that screenplay and the production video presentation into a private
workspace, generates its referenced video, and invokes the real `onmark`
binary twice. It requires identical decoded raw-frame hashes from the two
independent Chromium and `FFmpeg` sessions, verifies motion and stream facts,
and proves that a third invocation cannot replace an existing output:

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
