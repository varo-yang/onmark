# Competitive pipeline review

> Audit snapshot: HyperFrames
> `e2e61b0767b4b4dd282773eb50aa7ebaea98f0e5`, Remotion
> `258a191cbaf897f141525b071914fc4c92245b2e`, and Onmark
> `36d78c69c0c85594448a808258eb9142048ad3b9` on 2026-07-27.

This review records the external evidence behind Onmark's current execution
design. It is not a feature checklist or a claim that one product wins every
workload. The three systems solve overlapping but different authoring problems,
so only like-for-like measurements are treated as performance evidence.

## Pipeline shapes

### HyperFrames

HyperFrames treats authored HTML and its runtime timeline as the composition.
Its compiler normalizes HTML and timing metadata; the browser runtime seeks
supported animation libraries; the engine captures frames, optionally injects
extracted video frames, and streams or chunks them into FFmpeg. Browser pools,
static-frame deduplication, extraction caches, and parallel coordinators improve
throughput. AWS Lambda and GCP Cloud Run packages add provider-owned
orchestration and transport.

The relevant implementation spine is:

- `packages/core/src/compiler/` for HTML and timing compilation;
- `packages/core/src/runtime/` for clocks, adapters, readiness, and seek;
- `packages/engine/src/services/frameCapture.ts` and
  `streamingEncoder.ts` for capture and encoding;
- `packages/engine/src/services/videoFrameExtractor.ts` and
  `videoFrameInjector.ts` for native media assistance;
- `packages/producer/src/services/renderOrchestrator.ts` for local
  orchestration;
- `packages/aws-lambda/` and `packages/gcp-cloud-run/` for cloud surfaces.

Its strongest properties are native HTML familiarity, broad animation and media
adapter coverage, and practical production tooling. Its cost is that authored
timeline behavior, runtime adapter semantics, lints, and renderer recovery must
remain aligned across a large TypeScript surface.

### Remotion

Remotion treats React evaluation at a requested frame as the composition.
`useCurrentFrame()` and composition props feed browser rendering; renderer
workers cycle pages, collect assets, capture frames, and encode or combine
chunks. A Rust compositor handles media-heavy and low-level operations. Lambda
deploys a site and fans work into chunks before final combination.

The relevant implementation spine is:

- `packages/core/src/Composition.tsx`, `Sequence.tsx`, and
  `use-current-frame.ts` for authored timing;
- `packages/renderer/src/render-frames.ts`, `render-media.ts`, and
  `cycle-browser-tabs.ts` for browser work;
- `packages/renderer/src/assets/` and `combine-*.ts` for media and assembly;
- `packages/compositor/rust/` for native media operations;
- `packages/lambda/src/` for deployment, chunking, progress, and assembly.

Its strongest properties are React ecosystem reach, mature renderer breadth,
codec coverage, and a well-developed preview and cloud product. Its cost is that
authors still express temporal structure through frame-aware program logic, and
distributed execution includes a larger orchestration surface than Onmark
currently needs.

## Gap and decision matrix

The audit identified the following product gaps or deliberate non-goals. A
competitor feature is not automatically an Onmark requirement: each row records
the architectural disposition needed to avoid breadth without coherence.

| # | Surface | Evidence from competitors | Onmark disposition |
| --- | --- | --- | --- |
| 1 | Browser-free validation | HyperFrames exposes lint/check; Remotion validates compositions before render. | **Implemented:** `check` reaches the production Render Unit plan without Chromium or encoding and emits versioned diagnostics. |
| 2 | Machine repair loop | Both competitors expose structured command results to tooling. | **Implemented:** global `--json` keeps authored diagnostics and infrastructure failures distinct with stable exit codes. |
| 3 | Exact plan inspection | HyperFrames exposes inspect/snapshot tools; Remotion's Studio exposes composition facts. | **Implemented:** `inspect` projects Timeline IR, timing provenance, cue frames, media identities and source mappings, Render Units, capture cadence, and bundle identity without a second solver. |
| 4 | Toolchain diagnosis | HyperFrames has `doctor`; Remotion reports browser and codec setup failures. | **Implemented:** `doctor` admits exact executable paths and capture mode, then runs bounded real browser/media/bundler handshakes. It deliberately does not mistake a partial version string for a capture-environment identity. |
| 5 | Product identity | Both CLIs expose version and platform facts. | **Implemented:** `info --json` reports the released Onmark and target identity without tool discovery. |
| 6 | Long-operation feedback | Both products report render phases and progress. | **Implemented:** TTY-only phase progress never contaminates redirected or JSON output. |
| 7 | Reproducible performance measurement | HyperFrames exposes benchmark tooling; Remotion publishes renderer measurements. | **Implemented:** `benchmark` runs bounded odd samples through the complete uncached production pipeline and reports every phase plus medians. |
| 8 | Edit-friendly output | Remotion supports production codecs and HyperFrames offers alternate containers. | **Implemented:** `.mov` selects one closed ProRes 422 HQ/PCM profile; `.mp4` retains H.264/AAC. Release admission renders and probes both. |
| 9 | Agent-native workflow | HyperFrames distributes a broad skill suite; Remotion publishes agent guidance. | **Implemented, intentionally small:** one installable `onmark-video` skill delegates all facts to `check`, `inspect`, and `render` instead of duplicating policy in prompts. |
| 10 | Incremental edit turnaround | Both competitors cache work, but recent HyperFrames issues show boundary and invalidation failures. | **Onmark advantage already proved:** dependency regions, bundle identity, capture environment, and canonical raw pixels admit local and distributed reuse through the same artifact contract. |
| 11 | Native media performance | Remotion uses a Rust compositor; HyperFrames extracts/injects native frames. | **Onmark advantage already measured:** the admitted separable-overlay path is 4.18× the Chromium-media control at 79.73% of its peak RSS while preserving the shared plan and executor. |
| 12 | VFR and codec breadth | Both competitors accept broader media profiles. | **Measured prerequisite:** normalize exact frozen bytes or carry a complete timestamp map. Do not admit FFmpeg default frame selection or pretend CFR metadata describes VFR. |
| 13 | Crop, scale, picture-in-picture, and multiple videos | Both products expose rich media placement. | **Measured prerequisite:** add typed layout facts and prove whole, partitioned, and distributed raw-pixel equivalence before bypassing Chromium. |
| 14 | Alpha and alternate delivery containers | Remotion and HyperFrames support transparent outputs. | **Deferred contract:** prove alpha through browser capture, native composition, cache fingerprints, encoder pixel format, and muxing before exposing an extension. |
| 15 | Trim, hold, loop, and playback rate | Both products expose media treatments. | **Partly implemented:** a checked 20/20 live-model comparison admitted source-local `trim` and exact `speed`; Rust owns duration and source mapping, Browser Plan owns frame selection, and edited media remains browser-composited until the native path proves equivalent pixels. Hold and loop remain unadmitted. |
| 16 | Audio fades, ducking, panning, and loudness policy | Remotion has broad audio controls; HyperFrames has media-treatment workflows. | **Language-gated:** extend the existing exact sample-grid and rational-gain plan only after authored semantics and diagnostics are admitted. |
| 17 | Cross-shot transitions | Both products offer transition primitives. | **Language-gated:** transition windows and neighbor dependencies must enter Timeline IR and Render Graph before TypeScript realizes pixels. |
| 18 | Dynamic data and typed props | Remotion's props are mature; HyperFrames supports variables and data-driven compositions. | **Language-gated:** define typed schema, defaults, canonical encoding, cache identity, spans, and diagnostics together. Globals and URL parameters remain forbidden side channels. |
| 19 | Multiple caption tracks and authored caption style | Both competitors expose richer caption presentation. | **Language-gated:** retain current SRT/WebVTT/ASS import while evaluating track selection, overlap, style, and positioning as one coherent contract. |
| 20 | Studio, Player, and marketplace | Remotion leads here; HyperFrames includes Studio and a registry. | **Deliberate non-goal for this gate:** do not create a second mutable timeline, preview server, or template ecosystem before the compiler and CLI authoring loop prove a real need. |
| 21 | More cloud providers and orchestration | Both competitors ship broader cloud surfaces. | **Deliberate non-goal:** the portable Render Unit already admits provider adapters; no coordinator, database, queue, lease system, or speculative GCP package is added. |

### Onmark

Onmark separates authored intent from execution facts:

```text
semantic HTML
  → Source AST
  → Linked Film
  → Resolved Film
  → Timeline IR
  → Render Graph
  → Partition Plan
  → Render Units
  → Browser/visual/audio plans
  → capture + native media composition + one final encode
```

Rust owns timing, dependency regions, asset identity, partitioning, cache
identity, subprocesses, and assembly. TypeScript owns DOM, CSS, Canvas, WebGL,
Three.js, and animation adapters. Local and distributed workers consume the
same immutable Render Unit and executor contract. A presentation is sequential
unless it proves random access; a visual layer remains browser-composited unless
it proves separability.

This shape deliberately differs from both competitors:

- authors use native HTML and CSS without calculating frame coordinates;
- Timeline IR records exact facts once instead of asking browser code to infer
  them;
- partitions follow admitted pixel and temporal dependencies;
- unchanged contract-addressed regions can be reused locally or through the
  same stateless worker artifact;
- native video can remain decoded outside Chromium while transparent browser
  pixels are composed over it;
- placement-bounded foregrounds may reuse one immutable capture between solved
  visual changes;
- deployment stops at a portable worker plus one provider adapter, without a
  database, queue, lease system, or coordinator.

## Showcase language

The competitors' strongest public work does not rely on one reusable visual
template. HyperFrames' launch-film storyboard uses a moving infinite canvas as
a recurring spatial motif, then deliberately breaks into typography, shaders,
3D, real footage, captions, and audio. Its own design guidance recommends only
a few motivated shader transitions among otherwise direct cuts; the transition
is subordinate to the story beat, not the product. See the
[launch storyboard](https://github.com/heygen-com/hyperframes-launch-video/blob/main/STORYBOARD.md)
and
[design guidance](https://github.com/heygen-com/hyperframes/blob/main/docs/guides/claude-design-hyperframes.md).

Remotion's public showcase makes a different argument: music visualization,
captions, screencasts, year-in-review films, 3D tools, and data-driven products
share an engine without sharing a house style. Its presentation emphasizes
parameterized workflows and interactive products as much as individual motion
graphics. See the [Remotion showcase](https://www.remotion.dev/showcase) and
[product overview](https://www.remotion.dev/).

The Onmark showcase therefore avoids a single branded promo cut into twenty
cards. Each checked-in film has one continuous semantic shot and owns a distinct
composition system. Across the set, editorial typography, native footage,
captions, audio, Canvas, raw WebGL, and Three.js are primary media rather than
decorative badges. This proves breadth without introducing a template library
or hiding authoring complexity behind unpublished assets.

## Recent failure evidence

Recent HyperFrames changes expose failure classes that apply to any
browser-rendered engine:

- `020c898` forces a fresh GSAP timeline to render because `seek(0)` is a no-op;
- `4f53dd4` carries frame stride into worker results to prevent false
  distributed matches;
- `2be8a62` removes phantom duplicate compositing during oversized capture;
- `5ac3a7a` and its preceding fixes trim AAC packet padding on the exact sample
  timeline.

Onmark independently reproduced the GSAP zero-time failure and now uses a
focused adapter test plus explicit `timeline.render(...)`. Its conformance
compares whole and partitioned raw-RGBA frames, carries the capture environment
into artifact identity, and verifies exact decoded audio presentation length.

Recent Remotion changes expose adjacent classes:

- `f61d7c9d` handles long variable-duration video frames;
- `cfe4fb86` fixes transparent-video ghost trails;
- `aa83ed34` fixes opacity leaking between web-renderer layers;
- `f2686d9d` aligns Lambda ETag calculation with multipart upload behavior.

Onmark answers these classes with strict CFR admission for the current native
video path, explicit visual-layer lifetimes, content hashes over consumed bytes,
and one canonical frame-artifact format. It does not claim support for every
codec, VFR input, or cloud workflow that the mature competitors support.

## Evidence and limits

The repository keeps four different kinds of evidence:

1. `evals/html-authoring/` records a frozen 20/20 native-HTML authoring result
   against 16/20 for the earlier split screenplay/presentation surface. It also
   records 20 authored files and 13,618 bytes versus 46 files and 14,054 bytes.
2. `conformance/` proves compiler facts, browser protocol behavior, whole versus
   partitioned pixels and audio, native media layering, local incremental reuse,
   and portable worker artifacts.
3. Gate-seven's locked experiment measures the native-media path at 4.18× the
   Chromium-media control and 79.73% of its peak RSS on the same machine and
   input.
4. `showcases/` keeps twenty one-file compositions spanning HTML/CSS, media,
   subtitles, audio, GSAP, Canvas, raw WebGL, and Three.js. Every checked output
   was rendered by the public CLI.

These results establish Onmark's advantages only where the experiment controls
the workload and environment. The portable screenshot path remains slower than
native encode throughput on complex Canvas, WebGL, and media compositions.
Onmark also has less codec, adapter, preview, Studio, and provider breadth than
the two mature projects. Those are explicit product limits, not evidence that
should be hidden behind an incomparable headline benchmark.

## Design consequences

The audit supports retaining the current pipeline rather than merging the
compiler, browser runtime, and executor:

- keep exact authored timing out of TypeScript;
- keep DOM and GPU effects out of Rust;
- keep browser capture cadence and native-media admission explicit facts;
- require pixel and audio equivalence before admitting an optimization;
- prefer one portable worker contract over provider-specific engines;
- do not add a player, Studio, coordinator, or broad compatibility layer merely
  to match competitor surface area.

Future performance work should target measured portable-capture costs,
especially lossless screenshot transport and complex compositor submission.
It must preserve the current whole/partition equivalence and never introduce a
hidden fallback whose pixels differ.
