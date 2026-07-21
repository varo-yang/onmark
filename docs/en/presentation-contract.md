# Onmark Presentation Contract

> Status: Gate-one browser authoring contract, extended and reused through Gate
> four.

Onmark uses two authored files at Gate one:

- `film.onmark` owns screenplay facts: structure, content, IDs, cues, media
  references, and timing relationships.
- `presentation.ts` owns browser effects: DOM, CSS, layout, and the runtime host
  that applies solved facts.

This split is intentional. The screenplay remains readable and compiler-owned;
the presentation receives normal TypeScript tooling without becoming a second
timing language. Rust still owns every interval. TypeScript may render solved
facts, but it must not resolve cues, infer shot durations, partition work, or
choose frame ranges.

## Minimal entry

A Gate-one presentation normally looks like this:

```ts
import { createDomPresentationBindings } from "@onmark/authoring";
import {
  PresentationRuntimeAdapter,
  installRuntimeHost,
  materializedVideoSource,
} from "@onmark/runtime";

import "./presentation.css";

const adapter = new PresentationRuntimeAdapter(
  createDomPresentationBindings({
    document,
    videoSource: materializedVideoSource,
  }),
  5_000,
);

installRuntimeHost(adapter);
```

`onmark render film.onmark` discovers `presentation.ts` next to the screenplay
unless `--presentation` names another entry. The bundler compiles that entry,
injects the pinned Onmark packages, emits one immutable browser artifact, and
records it in a Rust-owned bundle manifest.

## Public adapter lifecycle

The runtime has one browser-effect boundary. A presentation installs an
implementation through `installRuntimeHost(adapter)`:

```ts
interface RuntimeAdapter {
  load(plan: RuntimePlan): Promise<void>;
  prepare(frame: RuntimeFrame): Promise<void>;
  seek(frame: RuntimeFrame): Promise<void>;
  confirm(frame: RuntimeFrame): Promise<void>;
  dispose(): Promise<void>;
}
```

`load` receives one recursively frozen snapshot of the accepted `BrowserPlan`.
It may create resources, but it must not retain a mutable author-owned plan.
`prepare` runs exactly once at `plan.evaluation.start` and must resolve only
when resources needed at that frame are stable. `seek` runs only after a
successful prepare. It applies the requested DOM state, registers decoded-media
observers, and resolves once browser media has finished seeking; it must not
wait for compositor presentation. `confirm` runs after native capture and
resolves only when browser media reports that the staged source frame reached
the compositor before native accepts the captured payload. `dispose` is terminal
even if cleanup reports a failure.

`seek` does not accept a free time `t`. It receives a `RuntimeFrame`:

```ts
interface RuntimeFrame {
  readonly index: number;
  readonly timeSeconds: number;
}
```

`index` is the absolute, exact frame identity selected by native execution.
`timeSeconds` is a derived browser-API projection of that frame through the
Rust-owned rational rate. It is useful for media APIs only; it must not become
an alternate scheduling clock or a source of timing decisions.

## Runtime handshake

The presentation must install exactly one runtime host with
`installRuntimeHost`. `Load` creates every video and overlay node in the plan.
Imported captions are caption-role overlays; they use the same solved visibility
path rather than a second browser timing engine.
Inactive nodes retain their stable binding identity but remain outside layout
and compositing until their solved interval makes them visible. This prevents
placements outside a Render Unit from changing its pixels. After `Prepare`,
native rendering sends one awaited, visual, non-capturing BeginFrame at a fixed
pre-baseline timestamp to initialize the page surface. Real captures use a later
fixed positive compositor baseline:

```text
Load(plan) -> Prepare(evaluationStart)
  -> native surface initialization without capture
  -> (Seek(frame) -> FrameStaged(frame)
      -> [native placement-boundary commit]
      -> native BeginFrame capture
      -> Confirm(frame) -> FrameReady(frame)
      -> [native placement-boundary reconciliation capture])*
  -> Dispose
```

The split handshake is required by Chromium's decoded-media contract.
`requestVideoFrameCallback` must be registered before the compositor frame that
it observes, but waiting for that callback before issuing `BeginFrame` would
deadlock a target controlled by CDP BeginFrameControl. `FrameStaged(frame)`
therefore means browser state is ready to enter the compositor. Native then
issues one normal capture-bearing `HeadlessExperimental.beginFrame` command for
each output frame. At a video or overlay boundary, native first commits the
staged placement without a screenshot at a fixed sub-millisecond offset before
the current compositor transaction's capture tick. This gives a newly visible
layer one compositor turn without retaining unrelated inactive layers or
advancing screenplay time. The capture command then commits frame state and
captures the PNG at that tick. These compositor ticks advance strictly in
capture order; `RuntimeFrame.index` remains authored time and may move backward
or repeat. A no-damage response normally reuses
the preceding PNG, but a boundary never does so; a missing boundary or first
screenshot receives one bounded sub-millisecond retry. `Confirm(frame)` waits
for the already-registered callback. At a placement boundary that observer may
complete on the pre-capture commit; runtime media state cannot change between
that commit and exact capture. `FrameReady(frame)` therefore means the exact
capture's staged media passed decoded-media confirmation before native accepts
it. A boundary then performs one bounded reconciliation capture at the
transaction's next positive sub-millisecond tick. Chromium may omit its pixels
when confirmation caused no
further compositor damage, in which case native reuses the exact capture; new
pixels replace it. A confirmation failure discards the captured payload before
it can enter an encoder or frame artifact.

## Ownership

The boundary is strict:

| Owner         | Owns                                                                       |
| ------------- | -------------------------------------------------------------------------- |
| Screenplay and imported captions | authored structure, text, media references, cues, local delays |
| Rust compiler | parsing, normalization, reference resolution, exact timing, Timeline IR    |
| Runtime       | protocol state, frame clock, decoded video readiness, visibility intervals |
| Presentation  | DOM shape, CSS, layout, typography, visual styling                         |
| Renderer      | materialized asset paths, Chromium, capture, encoding                      |

The presentation receives placements that already contain absolute frame
intervals. It may decide how a title looks, where a CTA sits, or how a video is
styled. It may not move a title earlier, extend an overlay, reinterpret `delay`,
or derive a new media duration from the DOM.

## Authoring facade

`@onmark/authoring` provides the default semantic DOM bindings:

- `createDomPresentationBindings({ document, videoSource })` returns runtime
  bindings for videos and overlays.
- Video placements become hidden `<video>` elements with the stable
  `onmark-video` class.
- Title, CTA, and caption placements become hidden `<div>` elements with
  `onmark-overlay` plus `onmark-title`, `onmark-call-to-action`, or
  `onmark-caption`.
- The runtime toggles visibility from solved intervals; CSS owns layout.

The default facade is deliberately small. A presentation can implement
`PresentationBindings` directly for Canvas, WebGL, or custom DOM, but the same
rules apply: bindings create browser resources, `setVisible` applies visibility,
and `dispose` releases resources terminally.

More precisely, the production adapter calls `bindVideo(placement, index)` and
`bindOverlay(placement, index)` once during `load`. A video binding supplies the
browser element, its materialized source, visibility effect, and terminal
cleanup. An overlay binding supplies visibility and terminal cleanup. The
`index` is the placement's stable position in the frozen plan; it is useful for
DOM identity and is not a timing coordinate. On every `seek`, the runtime first
hides videos, selects an admitted source frame from the authoritative output
frame, presents ready videos, then applies solved overlay visibility. Bindings
own those effects, not interval arithmetic.

## Plan facts, component selection, and props

The current language does **not** have `presents`, `definePresentation`, or a
screenplay-to-presentation props channel. `onmark render` selects one
`presentation.ts` through `--presentation` or same-directory discovery. The only
dynamic facts delivered to that entry are the Rust-owned `BrowserPlan` facts
sent by `Load(plan)`: frame rate, evaluation and output intervals, video
placements, and title, CTA, or imported-caption overlay placements. Static
values imported by `presentation.ts` are bundled program code, not screenplay
props.

This absence is intentional rather than an undocumented convention. A future
presentation-selection or props feature must define, together, its screenplay
spelling, typed schema and defaults, canonical wire encoding, source spans and
diagnostics, bundle/cache identity, and interaction with temporal capability
declarations. It also needs controlled language-evaluation evidence. Until that
work exists, a presentation must not read author intent from globals, URL
parameters, a mutable side channel, or an invented `presents` attribute.

## Temporal capabilities

The public closed capability is `PresentationTemporalCapability`, owned by
`@onmark/runtime`. It currently admits `sequential` and `randomAccess`;
`warmup(n)` and wider dependency categories remain architectural ideas rather
than public values. The CLI defaults unknown code to `sequential`, while the
low-level bundler requires an explicit value. Sequential execution produces one
whole-film Render Graph region.

The public `FrameEffect` boundary is owned by `@onmark/runtime`. Authoring may
provide a `frameEffects(plan)` factory to `createDomPresentationBindings`; the
standard adapter invokes that factory once during `Load(plan)` and owns the
returned effects until terminal disposal. On each `Seek(frame)`, effects apply
in declaration order after solved video and overlay placement, and all returned
promises resolve before `FrameStaged(frame)`. Effects receive the exact
immutable `RuntimeFrame`; they do not receive a scheduler or a mutable timeline.
Disposal attempts every effect even when one cleanup operation fails.

This lifecycle is not itself a random-access declaration. A presentation may
be bundled with `randomAccess` only after conformance proves that every requested
frame depends solely on immutable inputs and that exact frame. The declaration
is explicit build metadata, never inferred from source or screenplay spelling.
Bundle V2 includes it in canonical identity, and Rust consumes it before Render
Graph partitioning. Legacy V1 bundles and omitted CLI declarations remain
`sequential`.

## Assets

The browser receives materialized assets under the unit root. Gate-one video
sources use:

```ts
materializedVideoSource(placement);
```

That helper derives `./assets/sha256/<digest>` from the frozen asset identity in
the Rust-owned browser plan. Presentations should not reconstruct native paths,
read source files, or assume a working directory. The renderer verifies bytes
before the browser sees them.

## Determinism rules

Presentation code must be deterministic under the runtime frame clock.

Allowed:

- static CSS and DOM layout;
- local browser effects driven by runtime callbacks;
- bounded resource readiness owned by the runtime adapter;
- semantic classes or custom elements whose output depends only on solved plan
  facts and bundled assets.

Not allowed:

- `Date.now()`, wall-clock timers, random values, or ambient animation progress
  deciding pixels;
- browser media clocks deciding which frame is captured;
- network fetches or mutable external state participating in output;
- cue, delay, duration, or partition logic reimplemented in TypeScript;
- unbounded waits, queues, or retained buffers.

Gate five admits animation only through measured, paused playheads driven by an
exact `RuntimeFrame`. Its initial conformance matrix covers WAAPI, GSAP, and
Three.js through the standard frame-effect lifecycle without making those
libraries runtime dependencies. Static CSS transitions that depend on load
timing, free-running library tickers, and ambient `requestAnimationFrame`
progress remain outside the deterministic contract. Passing this lifecycle
does not grant a bundle random access; capability metadata lands only with its
partitioning proof.

## Failures and cleanup

Expected browser failures are reported through runtime protocol failures. Custom
adapters should throw `RuntimeAdapterError` when they can identify an operation
or readiness failure and should include bounded pending-resource names for
readiness timeouts.

Disposal is terminal. A presentation may report cleanup failure, but it must not
return a partially disposed session to service. Resource cleanup should be
idempotent where the browser API allows it.

## Non-goals

Gate one does not provide a presentation development server, watch mode, plugin
API, component registry, screenplay-selected components or props, cross-scene
persistence, free `begin/end/until` timing, or browser-side render planning.
Those capabilities require explicit language, runtime, and evaluation evidence
before they become public contract.
