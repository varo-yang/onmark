# Competitive pipeline review

> Audit snapshot: HyperFrames
> `041614d26e35b4cb9c6302504e534fc6e940e1b9`, Remotion
> `c122c4e31cfbc094647eb243b77553053c61360d`, and the current Onmark worktree
> on 2026-07-26.

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
