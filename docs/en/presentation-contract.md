# Onmark Presentation Contract

> Status: browser authoring contract through Gate seven.

`film.html` is the complete authored entry. Onmark custom elements own
screenplay facts—structure, IDs, cues, media references, and timing
relationships—while ordinary HTML and inline CSS own presentation. An optional
inline module marked `type="module" data-om-motion` exports browser effects.
There is no parallel stylesheet, motion file, generated DOM, template, or
custom-entry mode.

The browser receives the author's unchanged DOM. Rust still owns every
interval: HTML, CSS, Canvas, WebGL, GSAP, Three.js, and other browser libraries
may render solved facts, but they must not resolve cues, infer shot durations,
partition work, or choose frame ranges.

## Minimal entry

Static films need no authored JavaScript:

```bash
onmark render film.html
```

CSS is ordinary inline HTML:

```html
<style>
  .headline {
    color: white;
    font: 700 8vw/1 sans-serif;
  }
</style>

<om-film>
  <om-scene>
    <om-shot duration="3s">
      <om-title class="headline">Native HTML. Exact time.</om-title>
    </om-shot>
  </om-scene>
</om-film>

<script type="module" data-om-motion>
  import { gsapMotion } from "onmark/motion/gsap";

  export const motion = gsapMotion({
    title({ element, timeline }) {
      timeline.from(element, { opacity: 0, y: 40, duration: 0.4 });
    },
  });
</script>
```

`gsapMotion` accepts one semantic motion definition. Named handlers such as
`title` address every target of that kind; entries under `selectors` address
matching authored IDs or classes. The kind handler runs before matching
selectors, and all handlers contribute to one target-owned paused timeline, so
authors do not need an element-ID switch.

Element-local motion may consume only the interval attached to its semantic
target. Cross-shot transitions are not admitted: their windows and neighbor
dependencies must first become Rust-owned Render Graph facts rather than a
second timing policy in TypeScript.

The bundler extracts that one optional module, compiles its imports, restores
the runtime script at the same source position, and otherwise preserves the
authored HTML bytes. Authors do not construct a runtime adapter, register a
global timeline, or own infrastructure cleanup. The immutable artifact and its
capabilities remain Rust-owned manifest facts.

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
each frame selected by the checked capture cadence. `perFrame` selects every
output frame; `placementBounded` selects the first output frame and every solved
placement boundary, while intervening output frames reuse the exact preceding
PNG without another runtime transaction. At a video or overlay boundary, native
first commits the staged placement without a screenshot at a fixed
sub-millisecond offset before the current compositor transaction's capture
tick. This gives a newly visible layer one compositor turn without retaining
unrelated inactive layers or advancing screenplay time. The capture command
then commits frame state and captures the PNG at that tick. These compositor
ticks advance strictly in capture order; `RuntimeFrame.index` remains authored
time and may move backward or repeat. A no-damage response normally reuses the
preceding PNG, but a boundary never does so; a missing boundary or first
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

| Owner                                    | Owns                                                                       |
| ---------------------------------------- | -------------------------------------------------------------------------- |
| Screenplay and imported captions         | authored structure, text, media references, cues, local delays             |
| Rust compiler                            | parsing, normalization, reference resolution, exact timing, Timeline IR    |
| Runtime                                  | protocol state, frame clock, decoded video readiness, visibility intervals |
| Authored HTML and motion                  | DOM shape, layout, typography, and browser effects                         |
| Renderer                                 | materialized asset paths, Chromium, capture, encoding                      |

The presentation receives placements that already contain absolute frame
intervals. It may decide how a title looks, where a CTA sits, or how a video is
styled. It may not move a title earlier, extend an overlay, reinterpret `delay`,
or derive a new media duration from the DOM.

## Authoring facade

`@onmark/authoring` binds solved facts onto the authored semantic elements:

- `createDomPresentationBindings({ document, videoSource, motion? })`
  is the infrastructure facade installed by the bundle entry.
- `<om-film>`, `<om-scene>`, `<om-shot>`, `<video>`,
  `<om-title>`, and `<om-cta>` remain the exact authored elements.
- Every bound node temporarily carries `data-om-node`; authored IDs remain
  ordinary HTML IDs.
- Compiler node identity is whole-film renderable-semantic preorder. Unit
  projections retain that identity, so a later partition binds the correct
  elements in the unchanged complete document.
- Imported captions are the only DOM nodes created by the facade because they
  do not exist in the authored document.
- The runtime toggles container and content visibility from solved intervals;
  CSS owns layout and visual design.

More precisely, the production adapter binds film, scene, and shot containers
before content, then calls `bindVideo(placement)`, `bindOverlay(placement)`, and
the asynchronous `bindExtensions(plan)` once during `load`. An extension returns
the resources it needs prepared and the exact-frame effects it owns. A video
binding supplies the browser element, its materialized source, visibility
effect, and terminal cleanup. An overlay binding supplies visibility and
terminal cleanup. The compiler-owned node identity remains stable when an
earlier element is absent from a partition. On every
`seek`, the runtime first hides videos, selects an admitted source frame from
the authoritative output frame, presents ready videos, then applies solved
overlay visibility. Bindings own those effects, not interval arithmetic.

## Plan facts, component selection, and props

The current language does **not** have `presents`, `definePresentation`, or a
separate screenplay-to-presentation props channel. The dynamic facts delivered
to authored HTML are the Rust-owned `BrowserPlan` facts
sent by `Load(plan)`: frame rate, evaluation and output intervals, semantic
structure and ownership, video placements, and title, CTA, or imported-caption
overlay placements. Stylesheet
rules and static values imported by the inline module are presentation code,
not screenplay props.

Those existing facts are the closed built-in component contract: `nodeId` is
stable projection identity, optional `authoredId` supports semantic selection,
`kind` selects title, CTA, or caption, and `text` is that component's only
authored property. This does not create a
generic props channel or allow presentation code to reinterpret screenplay
structure.

This absence is intentional rather than an undocumented convention. A future
presentation-selection or props feature must define, together, its screenplay
spelling, typed schema and defaults, canonical wire encoding, source spans and
diagnostics, bundle/cache identity, and interaction with temporal capability
declarations. It also needs controlled language-evaluation evidence. Until that
work exists, a presentation must not read author intent from globals, URL
parameters, a mutable side channel, or an invented `presents` attribute.

## Temporal capabilities

The bundle contract carries the closed `PresentationTemporalCapability`, owned
by `@onmark/runtime`. It currently admits `sequential` and `randomAccess`;
`warmup(n)` and wider dependency categories remain architectural ideas rather
than public values. It is not a user CLI option. Authored HTML is conservatively
sequential until conformance admits a stronger presentation artifact. The
low-level conformance bundler requires an explicit value when constructing an
already-proved artifact.

The low-level `FrameEffect` and `PresentationResource` boundaries are owned by
`@onmark/runtime`. `@onmark/authoring` exposes the vendor-neutral
`PresentationExtension` contract. A single adapter is exported directly;
`combineMotion(...)` exists only when independent adapters must be composed in
declaration order.
`onmark/motion/gsap` is one optional adapter backed by an internal dependency
package: it turns semantic hooks into paused GSAP timelines without introducing
GSAP into runtime or authoring.
Three.js, Lottie, or an application-local engine can implement the same
contract; neither the bundler nor the runtime contains a vendor branch. Each
GSAP hook receives a semantic element, its compiler-owned duration, and an
adapter-owned paused timeline measured in local seconds. The adapter seeks that
timeline with callbacks suppressed and owns terminal cleanup. On each
`Seek(frame)`, effects apply
in declaration order after solved video and overlay placement, and all returned
promises resolve before `FrameStaged(frame)`. Effects receive the exact
immutable `RuntimeFrame`; they do not receive a scheduler or a mutable timeline.
Disposal releases effects in reverse ownership order and attempts every effect
even when one cleanup operation fails.

This lifecycle is not itself a random-access declaration. Only conformance may
admit a stronger adapter capability after proving that every requested frame
depends solely on immutable inputs and that exact frame. Capability is immutable
build metadata, never inferred from source or screenplay spelling. The bundle
manifest includes it in canonical identity, and Rust consumes it before Render
Graph partitioning.

## Visual capabilities

`PresentationVisualCapability` states which pixels Chromium may own. It is build
metadata, not screenplay spelling, and is never inferred from authored browser
code. The CLI conservatively declares `browserComposite`. The low-level
conformance bundler requires an explicit value for an already-proved artifact.

- `browserComposite` means Chromium owns the complete frame, including primary
  video. It is the conservative capability for unknown presentation code.
- `separableOverlay` means Chromium produces only a transparent foreground that
  is independent of primary-video pixels. Native execution may decode and place
  primary video before source-over compositing that foreground.

A `separableOverlay` presentation must remain correct when browser video
placements are omitted. It may use solved intervals, overlay facts, exact frame
identity, and immutable visual resources. It must not sample a video into
Canvas or WebGL, inspect media pixels, use backdrop-dependent filters or blend
modes, or otherwise make foreground pixels depend on the primary image beneath
them. The declaration is admitted by conformance, not trusted because a source
scan happened to find no forbidden token.

The current native path is deliberately narrower than the presentation promise:
one primary video must cover the complete published interval, its frozen source
dimensions must equal the output profile, and its complete color tuple must be
the admitted BT.709 limited-range profile. These checks avoid reconstructing
CSS layout in Rust. Capability is permission, not an execution command: planning
selects `separableOverlay` only when these facts prove the native profile and
otherwise records `browserComposite`. The resulting execution plan is immutable;
a worker never changes paths after launch, and a transported plan that exceeds
its capability still fails validation.

The current bundle manifest places temporal and visual capabilities in canonical
bundle identity together with the frame behavior below. Bundles are
reproducible build products rather than authored data; only the current
manifest version is accepted, and older bundles are rebuilt.

## Frame behavior

`PresentationFrameBehavior` states whether browser-owned pixels may change
between Rust-owned placement boundaries:

- `perFrame` is the conservative value. Chromium may need to evaluate and
  capture every authored frame.
- `placementBounded` proves that browser pixels remain identical until a video,
  overlay, or structural placement boundary changes the visible facts.

This declaration is independent of visual separability. The CLI conservatively
declares `perFrame`. The stronger behavior requires `randomAccess`: native may
skip intermediate `Seek` and `Confirm` calls only when later boundary frames
can be evaluated directly.

Capability remains permission rather than a cache instruction. Planning records
`placementBounded` capture only when Chromium owns no video pixels. A
browser-composite unit containing video stays `everyFrame`; a native-video
`separableOverlay` unit and a static browser-only unit may use the stronger
cadence. Native captures the first output frame and every solved boundary, then
shares the exact encoded PNG payload between intervening output frames while
still writing each frame to the encoder or worker artifact.

Frame behavior is immutable bundle metadata included in `bundleId`. It is never
inferred from source tokens, observed pixel equality, compositor damage, or
screenplay spelling. The worker request carries the admitted cadence and
rejects any value that disagrees with the bundle declaration or materialized
visual plan.

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

The inline motion module may import local AVIF, GIF, JPEG, PNG, SVG, WebP, OTF,
TTF, WOFF, WOFF2, or a local CSS module that references those formats. The
bundler copies imported bytes under opaque `resources/` paths and includes them
in the bounded, content-addressed manifest. A raw relative URL in authored HTML
or inline `<style>` is not a bundle import. Bundling proves byte identity only;
browser readiness requires explicit registration:

```ts
interface PresentationResource {
  readonly kind: "image" | "font" | "texture" | "custom";
  readonly id: string;
  prepare(): void | Promise<void>;
  dispose(): void | Promise<void>;
}
```

An extension's `bind()` result contains at most 256 resources. Their `kind:id` identities
must be unique, nonblank, trimmed, and bounded. `Prepare` starts every resource
concurrently under the adapter's shared readiness deadline, waits for every
bounded outcome, and reports all timed-out identities as
`<kind>:<id>:prepare`. Untyped preparation failures are contained behind the
same identity. Terminal disposal awaits every resource in declaration order
and retains the first cleanup failure without skipping later resources.
Any failed `Prepare` makes both the runtime session and presentation adapter
terminal: only `Dispose` remains valid. This prevents an uncancellable, late
resource preparation from overlapping a second preparation attempt.
The factory retains ownership of effects created before it returns; if it
throws partway through construction, it must release those partial effects.
The runtime takes ownership only of the returned collection.
The same result may contain at most 10,000 exact-frame effects; exceeding that
bound rejects the presentation and releases both returned collections.

The resource owns the meaning of ready: an image waits for successful decode, a
font waits for the exact face it will render, and a texture waits for upload to
the presentation's graphics context. `dispose` must cancel preparation that is
still pending after a deadline where the platform exposes cancellation, and it
must always prevent a late completion from reinstalling disposed state.
Registering an arbitrary promise without an owned browser resource does not
satisfy this contract.

`@onmark/authoring` provides `createImageResource({ document, id, source })`
and `createFontResource({ face, fonts, id })`. The image helper exposes its
owned element for authored layout and gates readiness on `decode()`. The font
helper loads the exact `FontFace` before adding it to the supplied
`FontFaceSet`; a completion after disposal cannot add the face back.

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

The native browser boundary also enforces the network rule. It admits only
canonical files beneath the private Unit Root and in-memory `data:` or `blob:`
URLs; HTTP, WebSocket, and file paths outside that root are blocked by CDP.

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
Once `Load` enters author bindings, any load, prepare, seek, or confirmation
failure is terminal; only `Dispose` remains valid. Wire validation that rejects
a request before author code runs does not consume the empty session.

## Non-goals

Gate one does not provide a presentation development server, watch mode, plugin
API, visual template, component registry, screenplay-selected components or
props, cross-scene persistence, free `begin/end/until` timing, or browser-side
render planning.
Those capabilities require explicit language, runtime, and evaluation evidence
before they become public contract.
