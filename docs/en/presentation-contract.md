# Onmark Presentation Contract

> Status: Gate-one browser authoring contract.

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

## Runtime handshake

The presentation must install exactly one runtime host with `installRuntimeHost`.
Native rendering waits for that host, then sends the versioned browser protocol:

```text
Load(plan) -> Prepare(evaluationStart) -> Seek(frame)* -> Dispose
```

`FrameReady(frame)` means the browser state is stable for that exact frame. It
does not mean the presentation computed time. It means the runtime applied the
Rust-solved plan and the native executor may capture the frame.

## Ownership

The boundary is strict:

| Owner | Owns |
| --- | --- |
| Screenplay | element structure, text, media references, cues, local delays |
| Rust compiler | parsing, binding, reference resolution, exact timing, Timeline IR |
| Runtime | protocol state, frame clock, decoded video readiness, visibility intervals |
| Presentation | DOM shape, CSS, layout, typography, visual styling |
| Renderer | materialized asset paths, Chromium, capture, encoding |

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
- Title and CTA placements become hidden `<div>` elements with
  `onmark-overlay` plus `onmark-title` or `onmark-call-to-action`.
- The runtime toggles visibility from solved intervals; CSS owns layout.

The default facade is deliberately small. A presentation can implement
`PresentationBindings` directly for Canvas, WebGL, or custom DOM, but the same
rules apply: bindings create browser resources, `setVisible` applies visibility,
and `dispose` releases resources terminally.

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

CSS animation is deferred until the runtime has a declared temporal capability
model for it. Static CSS transitions that depend on load timing are not a
deterministic Gate-one output contract.

## Failures and cleanup

Expected browser failures are reported through runtime protocol failures.
Custom adapters should throw `RuntimeAdapterError` when they can identify an
operation or readiness failure and should include bounded pending-resource names
for readiness timeouts.

Disposal is terminal. A presentation may report cleanup failure, but it must not
return a partially disposed session to service. Resource cleanup should be
idempotent where the browser API allows it.

## Non-goals

Gate one does not provide a presentation development server, watch mode,
plugin API, component registry, cross-scene persistence, free `begin/end/until`
timing, or browser-side render planning. Those capabilities require explicit
language, runtime, and evaluation evidence before they become public contract.
