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

The full local-render smoke additionally streams every output frame through
`FFmpeg`, probes the published MP4, and decodes it again:

```bash
ONMARK_CHROME=/path/to/chrome \
ONMARK_FFMPEG=/path/to/ffmpeg \
ONMARK_FFPROBE=/path/to/ffprobe \
cargo test -p onmark-render --test render \
  renders_the_gate_one_plan_to_a_verified_mp4 -- --ignored
```
