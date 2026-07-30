---
name: onmark-video
description: Create, edit, validate, inspect, snapshot, review, benchmark, or render deterministic Onmark videos from one HTML screenplay. Use for Onmark film.html files, screenplay timing, semantic video/audio/caption elements, CSS/Canvas/WebGL presentation, seekable frame motion, CLI diagnostics, exact visual feedback, incremental rendering, or delivery and ProRes exports.
---

# Onmark video

Use the Onmark CLI as the source of truth. Keep authored intent in one HTML
screenplay; never reproduce its timing solver in JavaScript or prompt text.

## Work from the project

Inspect the existing `film.html`, adjacent assets, and local conventions before
editing. In an Onmark repository checkout, read only the relevant contracts:

- `docs/en/language-specification.md` for elements, attributes, timing, and diagnostics;
- `docs/en/presentation-contract.md` for CSS, DOM, Canvas, WebGL, and motion;
- `docs/en/architecture.md` only when changing a process or package boundary.

Do not add a second presentation file, generated DOM layer, template framework,
or free `start`/`end` timeline. Do not replace semantic screenplay structure
with absolutely positioned track arithmetic.

## Author one film

Use ordinary HTML and CSS for presentation. Use `om-*` elements only for
screenplay facts:

```html
<style>
  .headline { color: white; font: 700 8vw/1 sans-serif; }
</style>

<om-film>
  <om-scene>
    <om-shot duration="3s">
      <om-title class="headline">Hello, motion.</om-title>
    </om-shot>
  </om-scene>
</om-film>
```

Prefer content-derived duration when media supplies it. Add an explicit
duration only when the screenplay genuinely owns that fact. Use cues and local
delay instead of calculating absolute coordinates.

Use one optional inline module for seekable effects:

```html
<script type="module" data-om-motion>
  import { frameMotion, interpolate } from "onmark/authoring";

  export const motion = frameMotion({
    title({ element, progress }) {
      element.style.opacity = String(interpolate(progress, [0, 0.2], [0, 1]));
    },
  });
</script>
```

Every effect must derive its complete state from the requested frame. Never
advance mutable time, read wall clocks, depend on prior frames, or use ambient
CSS animation. External libraries are acceptable only through an admitted
seekable adapter such as `onmark/motion/gsap`.

## Close the feedback loop

Run the cheapest authoritative command that answers the current question:

1. `onmark check film.html --json` after every structural or timing edit.
2. `onmark inspect film.html --json` when reasoning about frames, partitions,
   execution paths, or bundle identity.
3. `onmark snapshot film.html --frame 42 --json` to inspect an exact production
   frame before paying for a complete encode. Choose the frame from `inspect`;
   do not convert guessed seconds in prompt text.
4. `onmark review film.html --json` for a static film-wide contact sheet backed
   by exact production regions. Compare an edit with
   `--against reviews/<prior>/manifest.json`.
5. `onmark render film.html --output draft.mp4` after selected frames are sound.
6. Re-run the same command after edits; Onmark reuses only verified unchanged
   regions.
7. `onmark benchmark film.html --runs 3 --json` only for measured performance
   work.

Treat exit code 1 as authored diagnostics to fix at their reported spans. Treat
exit code 2 as an infrastructure failure; preserve its typed message instead of
rewriting the screenplay blindly. Never hide a failure by changing tools,
capture modes, frame rates, or output profiles.

Use `onmark doctor --json` to diagnose the local toolchain and `onmark info
--json` to record product identity. Treat a snapshot as pixel evidence, not a
replacement for watching the completed motion. Do not invoke Chromium directly
or construct a private FFmpeg pipeline.

## Deliver

Use `.mp4` for H.264/AAC delivery and `.mov` for edit-friendly ProRes/PCM:

```bash
onmark render film.html --output release.mp4
onmark render film.html --output edit.mov
```

Keep rational rates exact (`--fps 30000/1001`). Declare SRT, WebVTT, or ASS
tracks with film-level `om-captions` elements; use `--captions en,zh` only to
select an ordered subset. Inspect the finished video rather than claiming
visual quality from a successful exit code alone.

Before handing off, report the screenplay path, output path, exact CLI command,
diagnostics, frame count, capture mode, graphics backend, output profile, reuse
status, and phase timings. State unsupported requirements plainly; do not invent
unadmitted syntax or silent fallbacks.
