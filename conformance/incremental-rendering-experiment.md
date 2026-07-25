# Incremental rendering conformance

This record admits shot-scoped browser projection and persistent local
`FrameArtifact` reuse. It keeps three independent facts separate:

- temporal capability decides whether a presentation can evaluate a requested
  frame without replaying earlier frames;
- document scope records whether a bundle contains the whole film or one Render
  Graph region;
- artifact identity commits the exact region plan, browser inputs, render
  profile, capture policy, and capture environment.

No capability is inferred by scanning source tokens.

## Retained cases

The checked real-process suite covers the following boundaries:

| Case | Required result |
| --- | --- |
| whole film versus two region bundles | canonical raw-RGBA sequences are equal |
| edit only the closing title | opening region identity and pixels are unchanged; closing region changes |
| closing shot gains a class observed by `om-scene:has(...)` | opening pixels remain unchanged because the closing sibling is absent from its region document |
| global style, motion, or imported resource changes | every consuming region receives a new bundle identity |
| audio-only or compiler-only authored facts change | visual identity remains stable unless solved browser or partition facts change |
| cold CLI render followed by an unchanged render | verified artifacts are reused through the shared assembler |
| one shot changes between CLI processes | exactly one new region artifact is published |
| a requested cached artifact is corrupted | validation removes it, capture recreates it, and final output matches the clean revision |
| CLI incremental report | cold, one-shot edit, and corruption repair report their verified reused regions and frames |

The bundler compiles generated modules and resources once. It publishes a
`wholeFilm` root plus one `renderRegion` root per shot, hard-linking immutable
generated payloads into each region. Region HTML contains the selected shot and
its scene and film shells; semantic siblings are omitted. Presentation bytes
follow the narrowest semantic owner containing them: shot, scene, or whole
film. Browser Plan node IDs are dense and local to that projected document.

The desktop launcher derives a conservative environment seed from its pinned
browser artifact, host platform and OS release, and bounded font inventory.
Native capture adds capture mode, graphics backend, and composition version.
Passing an explicit browser disables persistent reuse because that executable is
outside the launcher-owned identity. Cache entries are count- and byte-bounded,
read without a global lease once valid, published atomically under a
cross-process lock, and never evicted while they may be in use. The same lock
owns corruption repair.

## Local admission run

- date: 2026-07-25
- base revision: `d969536`
- host: macOS 26.5.2 arm64
- browser: Google Chrome 150.0.7871.186
- FFmpeg: 8.1.2
- result: region isolation, selector isolation, temporal artifact assembly,
  media whole-versus-partition equality, and cross-process CLI repair passed

Build the bundler and native CLI, then run the retained renderer cases:

```bash
ONMARK_BUNDLER=/path/to/onmark-bundle \
ONMARK_HEADLESS_SHELL=/path/to/chrome-headless-shell \
ONMARK_FFMPEG=/path/to/ffmpeg \
ONMARK_FFPROBE=/path/to/ffprobe \
cargo test -p onmark-render --test render \
  authored_html_edit \
  -- --ignored --nocapture
```

Run the CSS isolation case separately:

```bash
ONMARK_BUNDLER=/path/to/onmark-bundle \
ONMARK_HEADLESS_SHELL=/path/to/chrome-headless-shell \
ONMARK_FFMPEG=/path/to/ffmpeg \
cargo test -p onmark-render --test render \
  shot_projection_blocks_cross_partition_css_observation \
  -- --exact --ignored --nocapture
```

Run the release-CLI cache boundary with a browser discoverable through the
normal product path:

```bash
ONMARK_CLI=/path/to/onmark \
ONMARK_BUNDLER=/path/to/onmark-bundle \
ONMARK_FFMPEG=/path/to/ffmpeg \
ONMARK_FFPROBE=/path/to/ffprobe \
cargo test -p onmark-cli --test render \
  reuses_only_unchanged_regions_across_cli_processes \
  -- --exact --ignored --nocapture
```

Locked Linux CI remains the release authority. It must run the same cases with
the browser build recorded in `packages/launcher/desktop-release.json`; a local
macOS pass is supporting evidence, not a replacement.

## Consequence

Shot-local authored edits now have a complete reuse path:

```text
source edit
  -> deterministic region projection
  -> unchanged region bundle and Browser Plan identity
  -> verified persistent FrameArtifact hit
  -> shared artifact assembler
  -> final video
```

This does not promise that every future dependency is local. Cross-shot
transitions, backdrop sampling, shared mutable browser state, or new component
models must first become explicit Render Graph or presentation-capability facts.
Unknown future browser components remain sequential by default.
