# Onmark Architecture

> Status: target architecture. This document separates foundational rules,
> ordered delivery gates, and later production capabilities.

This document is paired with the Onmark Language Specification. The language
defines authored meaning; this document defines execution. Their only contract
is the versioned Timeline IR.

Onmark is a screenplay-first, browser-rendered video compiler and execution
engine. It compiles authored intent and a frozen asset catalog into
deterministic timeline and render graphs, then executes the resulting plan
locally or across stateless workers.

## Architectural axioms

1. Source expresses intent; IR records facts.
2. The compiler is a pure function of source, frozen asset catalog, options, and
   version.
3. Local and distributed rendering execute the same `ExecutionPlan` and worker
   state machine.
4. Partitions follow pixel and temporal dependencies, not blindly chosen shot
   boundaries.
5. Chromium draws resolved frames; it does not solve time or discover work.
6. Every expensive artifact has an explicit deterministic identity; immutable
   byte artifacts also retain content hashes.

## TypeScript and Rust boundary

TypeScript owns the browser world: authoring types, DOM/CSS/Canvas/WebGL
components, the deterministic browser clock, animation adapters, and bundler
integration. Rust owns the system world: parsing, binding, timing, typed IR,
render graphs, partitioning, cache keys, scheduling, subprocess lifecycles,
media planning, workers, and CLI.

Rust does not reimplement browser layout. TypeScript does not duplicate timing
or planning logic. Optional WASM or N-API bindings wrap the Rust compiler rather
than create a second compiler.

## Representations

```text
Source AST
  → Structurally Linked Film
    → Resolved Film
      → Timeline IR
        → Render Graph
          → Execution Plan
```

- **Source AST** preserves authored structure and source spans.
- **Structurally Linked Film** contains known elements, legal ownership, and
  valid film-wide IDs while retaining unresolved authored attributes privately.
- **Resolved Film** contains typed durations, cues, asset references, and
  content start rules without exposing syntax-layer attributes.
- **Timeline IR** contains exact frame intervals and provenance for every timing
  decision.
- **Render Graph** records pixel, media, transition, history, global-layer, and
  audio dependencies.
- **Execution Plan** contains immutable work units, environments, dependencies,
  output intervals, evaluation intervals, and cache keys.

A render unit has separate `evaluation` and `output` intervals. Evaluation may
include transition preroll, animation warm-up, or history frames; only output
frames are published.

The compiler pipeline ends at Timeline IR. Execution begins at a separate
composition boundary:

```text
Timeline IR + Frozen Asset Catalog + Bundle Manifest + Render Profile
  → Render Unit
    → Browser Plan + Audio Plan + materialization requirements
      → materialize → Executable Unit + verified private root
```

This join is intentionally not another compiler phase. Timeline IR says what the
film is and when each fact holds; a presentation bundle owns how those facts
become DOM, CSS, Canvas, or WebGL; a Render Unit says which immutable inputs one
executor invocation consumes. `RenderProfile` owns pixel-affecting facts such as
viewport dimensions; process deadlines and retained-memory ceilings remain
executor limits. Materialization consumes the unit into an `ExecutableUnit`, so
the executor cannot pair a browser plan with an unrelated URL or asset root.
Gate one has exactly one whole-film unit. Gate two introduces the Render Graph
and may derive several units of the same type; it does not replace the executor
contract.

The original Gate-one `AudioPlan` established the native mixing boundary with
solved voice-over placements. Materialization copies frozen audio bytes beside
browser assets without making them browser inputs. After Chromium has encoded
the visual stream, the executor mixes the tracks and muxes AAC into the final
MP4. Every unit and complete assembled sequence retains at most 32 audio tracks,
keeping the `FFmpeg` process, input-descriptor, and filter-graph boundaries
explicitly bounded. Gate four extends the facts and sample policy at this same
boundary rather than creating a second audio engine.

The first Gate-two local assembly keeps its independently materialized units
alive while their contiguous output frames enter one continuous visual encoder
in screenplay order. Audio placements retain absolute Timeline starts until
final assembly rebases them once to the assembled artifact's output origin and
mixes them after every unit has captured its output. This avoids treating
separately AAC-muxed segments as safely concatenable, and also avoids a second
lossy visual encode. It is deliberately a correctness path, not yet a persistent
segment-cache format: cached encoded segments require a separate equivalence
proof before they become an execution artifact.

Gate four retains voice-over as a narrative Timeline node while moving its
executable asset, interval, gain, and role into the same `TimelineAudio` fact
used by music and sound effects. General audio remains a film-level
collection because a music bed may cross shot and partition boundaries. The
Render Graph assigns each placement to the one region containing its start;
that owner materializes the frozen bytes once, while the placement may extend
beyond the owner's visual output and is mixed only during final assembly.

Audio probing now retains the selected stream's positive integer sample rate
and normalized mono or stereo channel layout. Other channel counts are rejected
before FFmpeg can apply an implicit downmix. Mono is duplicated explicitly and
stereo preserves left/right identity; the fixed mix profile is 48 kHz stereo
floating-point audio before AAC encoding.
At unit composition, the exact frame length is projected once onto that sample
grid with named ceiling semantics: a sample whose timestamp precedes the
exclusive Timeline end is retained. Each input is trimmed on its source grid,
then resampled to the fixed 48 kHz mix grid. Rust projects its frame start onto
that grid with ceiling semantics, so `FFmpeg` receives an integer `adelay`
sample count rather than evaluating a decimal or floating timing expression.
The canonical rational linear gain is applied through `volume`, and `amix`
normalization is disabled so overlapping tracks do not silently rescale it.
The final AAC path trims or pads the mix to the visual frame count projected
onto the same grid and lets the visual stream close the container through
`-shortest`. A partition-owning track therefore cannot lengthen an independently
rendered unit beyond its visual output.
The checked-in audio-syntax evaluation compared semantic `<music>`/`<sfx>`
elements with a generic `<audio kind="...">` spelling across forty model
outputs. Both retained 20/20 generation reliability. Gate four therefore
admits the semantic elements: their kinds encode role and legal containment
without a second kind/parent validity matrix. Authored gain is the exact closed
range from `0%` through `100%`.

## End-to-end pipeline

```text
freeze inputs ─┬→ probe media ─→ compile ───────────────┐
               └→ bundle presentation ─────────────────┤
                                                       ▼
                         compose one whole-film Render Unit
                           → materialize Executable Unit
                             → capture/encode → mix audio → verify

Gate two inserts: Timeline IR → Render Graph → partition → Render Units
```

The compiler performs parse, structural bind, attribute/reference resolve, and
timeline solve without IO. Validation belongs to the phase that first has enough
information to decide; solve constructs Timeline IR directly. Onmark does not
add ceremonial `validate` or `lower` phases without a representation that proves
a new invariant. At Gate two, the planner computes dependency closure before
partitioning. Its first graph records each Gate-one shot as an independent
region only because the production adapter has proved that it keeps no state
across shot boundaries; the graph also records each region's direct frozen-media
dependencies. This is evidence-backed, not a general shot-boundary rule.
Transitions, persistent elements, global effects, and historical shaders must
widen or merge regions for correctness before partitioning consumes them.

Structural binding and attribute/reference resolution aggregate authored
diagnostics while building candidate outputs. An error withholds the phase value
from its report so rejected structure or recovery defaults cannot enter the next
phase as compiler facts; warnings remain non-blocking.

Input freezing separates three identities that must never be conflated:

- `AssetRef` is the screenplay-relative portable path authored in the
  screenplay;
- `FrozenAssetId` identifies the immutable bytes that were probed and compiled;
- a materialized asset is a worker-local path or browser URL for those exact
  bytes.

Timeline solving consumes a catalog from `AssetRef` to `FrozenAssetId` plus
normalized `AssetMetadata`, all owned by `onmark-core`. Metadata records exact
artifact duration and, for each selected audio or visual stream, its exact
stream duration. Visual metadata also records codec, pixel format, and either
one exact rational frame rate or variable timing, plus positive source-pixel
dimensions and any complete admitted color tuple. Single-frame streams are
represented separately because an exact reported frame count of one cannot
establish a source rate. `onmark-media` produces metadata through probing, while
a loader or composition root hashes and freezes the same bytes. ffprobe-specific
structures, source paths, and browser URLs remain outside core. Timeline IR
records `FrozenAssetId`, never the authored spelling or a mutable path. A
missing catalog entry is a typed integration failure rather than an authored
diagnostic. A media element with no authored source remains valid through static
resolution but cannot produce renderable Timeline IR and receives an authored
asset diagnostic during solving.

Gate-one `FrozenAssetId` uses SHA-256 and the canonical `sha256:<lowercase-hex>`
spelling. The hashing operation belongs at the IO freezing boundary; core owns
only the validated identity and deterministic mapping.

The bundle manifest has the same separation. Its contract identifies an
immutable presentation artifact and its entry point, runtime version, fonts,
static dependencies, and declared temporal and visual capabilities. The
current manifest also records whether browser-owned pixels may change between
solved placement boundaries, and binds all three declarations into `bundleId`.
Its identity is
`{version,entryPoint,temporalCapability,visualCapability,frameBehavior,files}`.
It uses
SHA-256 over UTF-8 compact JSON, with files sorted by portable path and each
entry ordered as
`{bytes,path,sha256}`. This is a versioned contract, not an incidental
pretty-printed representation. A manifest contains one to 99,999 payload files.
Paths are lowercase portable ASCII, at
most 1,024 bytes, cannot enter unit-owned namespaces, and cannot make one file
the directory ancestor of another. Materialization turns frozen bundle and asset
identities into local paths or browser URLs immediately before execution and
verifies their digests. Gate one assembles one content-addressed unit root:
required assets appear at `assets/sha256/<lowercase digest>` beneath the
presentation entry. The browser derives that relative location from the frozen
identity already present in `BrowserPlan`; native paths and browser URLs
therefore need no second wire protocol. The unit retains worker-local source
paths only until assembly has verified or linked the exact bytes into that root.
The normal bundle installs a neutral semantic DOM projection for solved video
and overlay facts; it contributes no layout, style, animation, or full-screen
assumption. An explicitly selected custom presentation instead owns DOM shape
and runtime installation. Both paths consume the same deterministic clock,
readiness, and media primitives. The public rules for author-owned browser code
live in the [presentation contract](presentation-contract.md).

Each Gate-three capture worker executes one state machine:

```text
materialize → launch → ready → seek/capture → fingerprint → verify → commit
```

The capture worker never owns a visual encoder: it atomically publishes one
verified frame artifact. The short-lived render owner verifies the finite set
of artifacts; the assembler owns the one continuous visual encode. Audio
follows a separate plan and is mixed outside browser frame capture.

## Deterministic browser protocol

The sole clock derives from frame index and rational timebase. Wall time and
free-running animation or media clocks may not determine output.

The runtime protocol includes `Load`, `Prepare`, `Seek`, `FrameStaged`,
`Confirm`, `FrameReady`, and `Dispose`. Native rendering selects one of two
closed capture contracts from the browser artifact and host: Linux
`chrome-headless-shell` owns CDP BeginFrameControl, while macOS, Windows,
ordinary Chrome, and Chromium use the portable screenshot path.
`Load` creates every video and overlay binding for the plan.
Inactive nodes retain stable binding identities but remain outside layout and
compositing until their solved intervals make them visible. Placements omitted
from one Render Unit therefore cannot perturb its pixels.

After `Prepare`, native execution issues one visual, non-capturing
`HeadlessExperimental.beginFrame` at a fixed pre-baseline timestamp to
initialize the page surface. It is not a warm-up tick: `noDisplayUpdates` is
false, and the command is awaited before capture begins. Real captures start at
a later fixed positive compositor baseline. From there, a session-owned clock
advances by one fixed positive step per capture transaction. The rational frame
rate remains the declared CDP frame interval, but neither it nor the authored
frame determines transaction identity. Authored time may move backward or
repeat; Chromium's compositor clock never does.
`Seek(frame)` applies browser state, registers decoded-media observers, and
returns `FrameStaged(frame)` after media seeking without waiting for compositor
presentation.

At a plan-owned video or overlay boundary, native first issues one non-capturing
visual BeginFrame at a fixed sub-millisecond offset immediately before the
current compositor transaction's capture tick. That bounded placement commit
gives a newly visible layer one compositor turn without advancing screenplay
time. Native then issues one `HeadlessExperimental.beginFrame` command that both
commits and captures a lossless PNG at the transaction tick.

Headless shell may omit `screenshotData` when the compositor reports no visual
damage. Native normally reuses the last valid PNG, but never does so at a
placement boundary. A missing boundary or first screenshot receives exactly one
retry at a bounded positive sub-millisecond offset; an empty retry fails rather
than looping.

`Confirm(frame)` waits for the pre-registered media observer before native
accepts the captured payload. At a placement boundary the observer may complete
on the pre-capture commit. After confirmation, native performs one bounded
reconciliation capture at the transaction's next positive sub-millisecond
tick. A no-damage response
reuses the exact capture without copying its PNG payload; new pixels replace it.
Only then may native execution write the payload. This closes the race in which
the media observer and exact screenshot become ready on opposite sides of the
same compositor turn.

This ordering avoids three boundary failures: waiting for
`requestVideoFrameCallback` before advancing a BeginFrame-controlled compositor
would deadlock; introducing a layer only on the capture command can produce one
stale or blank frame; and retaining unrelated future layers can make whole-film
and partition captures differ. Surface-initialization, placement-commit, and
capture-baseline timestamps never become scheduling or protocol truth.

The portable visual plan records one checked browser-capture cadence.
`everyFrame` gives each output frame one normal capture command. A
`placementBounded` plan captures the first output frame and every solved
placement boundary, then reuses that exact immutable PNG between boundaries.
The renderer skips `Seek`, `Confirm`, and screenshot work for those reused
foreground frames while still writing every output frame to the encoder or
worker artifact. Native primary video therefore continues to advance through
the existing compositor. A boundary capture still adds one non-capturing commit
and one post-confirm reconciliation capture; a missing first or exact boundary
screenshot adds at most one bounded retry.

This cadence is not inferred from screenshot equality or source inspection.
The bundle must declare `placementBounded`, that declaration requires
`randomAccess`, and visual admission must prove that Chromium does not own a
video placement. Browser-composite units containing video remain `everyFrame`
even when the bundle grants the stronger behavior. The selected cadence is
serialized with the worker visual plan and checked again against the bundle and
browser plan during materialization. This is separate from Chromium's
within-transaction no-damage response: one is a planned cross-frame fact, the
other is bounded handling of an omitted screenshot payload.

Direct rendering retains the PNG as its encoder payload. Worker capture also
decodes it into the configured exact 8-bit RGBA viewport and hashes those
canonical pixel bytes. A worker artifact records that raw-pixel hash beside each
ordered PNG record, so independent captures can be compared without treating PNG
compression bytes as visual truth. The runtime does not publish an independently
invented state hash. Inside the runtime, `RuntimeFrame` retains the exact
integral frame identity and derives floating-point seconds from the Rust-owned
rational frame rate only for browser API calls.

Future partitioning may classify component behavior as `stateless`, `warmup(n)`,
`sequential`, `global`, or `neighbor(radius)`. These are design categories, not
a public declaration API today. When that API exists, unknown components must
default to `sequential`: parallel seekability must be proven, not guessed.
Native APIs and audited adapters may provide stronger declarations; detection
may recommend an adapter but may not silently relax correctness.

Determinism is layered. Canonically encoded Timeline IR and Execution Plans must
be byte-identical once those encodings exist. The current in-memory Timeline IR
is structurally deterministic but does not yet claim canonical wire bytes. Raw
frames target identical hashes inside a locked browser/font/render environment;
per-frame worker-artifact fingerprints make that property an executable
conformance claim. Encoded containers are validated by timestamps, frame counts,
codec configuration, and decoded content; byte-identical MP4 output is an
experimental property, not a blanket promise.

## Distributed execution (production target)

A remote render is one finite DAG owned by one short-lived invocation. A parent
process or provider-native workflow may retain transient progress while workers
exchange immutable bundles, assets, and artifacts directly with object storage.
Restarting the same render identity verifies and reuses completed artifacts. The
engine therefore needs no database, durable queue, distributed lease, or Redis
lock for correctness. A future multi-tenant service may wrap this contract with
its own admission and accounting systems, but those are not Onmark engine
dependencies.

Gate three starts with a deliberately narrower interchange: a worker captures a
whole planned output interval into one bounded, checksummed frame artifact. The
artifact is a single versioned file containing an exact output interval,
render-profile, visual-plan, and locked capture-environment identities, plus an
ordered PNG stream with canonical raw-RGBA fingerprints. It is published through
a sibling staging file and no-clobber link, so a retry can verify or reuse an
existing immutable result without ever exposing a partial one. The assembler
verifies that each artifact belongs to its planned unit and capture environment,
then streams verified frames through the same one continuous visual encoder used
by Gate two and materializes and mixes all audio once at the assembled output
origin.

Artifact checksums prove storage integrity; recorded fingerprints are not trusted
as pixel evidence by themselves. Reuse and equivalence checks decode each bounded
PNG in order, recompute its canonical raw-RGBA fingerprint, and retain at most one
decoded frame while comparing the sequence.

This is intentionally not a remote-frame queue: a worker owns a contiguous unit
and publishes one artifact only after its browser session has completed. Nor is
it an encoded-segment cache. Independently AAC-muxed MP4 segments are not
assumed concatenable, and independently encoded visual segments need a separate
equivalence proof before they can replace the lossless frame interchange. Long,
expensive, randomly seekable scenes may later be divided into contiguous frame
ranges once their render dependencies prove it safe. Individual frames are never
remote tasks. CPU, memory, GPU, browser slots, encoder slots, codecs, disk, and
network are explicit scheduling resources; all internal queues are bounded.

The first implementation used the local filesystem only to prove the process and
artifact contract. After that conformance passed, `deploy/aws-lambda` became the
first narrow cloud adapter. Its Rust binary owns only versioned Lambda
invocation/result JSON, bounded S3 materialization, and conditional publication
of the existing `onmark-render` frame artifact. It uses no generic object-store
trait because S3's multipart `If-None-Match: *` completion, precondition reuse
verification against this capture's raw-RGBA sequence, and bounded conflict
retry are the actual semantics it needs. AWS types stop at that deployment
package; they never enter core or render.

One absolute 13-minute work deadline spans materialization, capture,
verification, and publication. Multipart publication observes that deadline
inside its upload owner, so expiry still attempts abort; a cleanup failure is
reported alongside, rather than instead of, the original failure. Two minutes
remain beneath Lambda's platform ceiling for abort and response delivery.

The adapter emits one structured completion event for each expensive phase,
including elapsed milliseconds and success state, under the Lambda request
identity. A direct synchronous conformance invocation disables client retries
and gives the client a read timeout longer than the worker deadline. Otherwise
the AWS CLI's shorter transport timeout can start a second, wasteful capture
while the first invocation is still running; immutable publication remains
correct, but idempotence is not permission to duplicate browser work.

The adapter has no coordinator, queue, lease database, global retry owner,
capability scheduler, infrastructure definition, or published Lambda release.
Its input chooses only an immutable worker-input S3 prefix. The deployed image
or ZIP owns the output namespace, browser payload, locked capture-environment
identity, and limits. Its handler explicitly selects
`BrowserLaunchPolicy::isolated_worker()`, which assigns process isolation to
Lambda and selects the measured single-process, no-zygote, in-process
SwiftShader topology. That choice is neither an automatic launch fallback nor
invocation-controlled. The real arm64 experiment described below establishes
this narrow launch contract. Its preferred measured form is a compact browser
archive inside the function ZIP. The handler begins Runtime API polling before
browser I/O, verifies and expands the archive during the first bounded
invocation, and retains that private installation for warm invocations. An
already-expanded executable remains a supported deployment input. The deployment
package's dedicated packager owns the reviewed deterministic ZIP shape described
below; cross-compilation, release publication, and infrastructure templates
remain separate concerns.

## Incremental rendering

Changing a base layer permits upper-layer reuse only when the render graph
proves there is no sampling or composition dependency. Blend modes, backdrop
filters, transitions, and shaders expand invalidation. Layered alpha
intermediates can improve reuse at an encoding, color, and composition cost.
Correctness outranks cache granularity; Onmark does not promise that every shot
is always independent.

## Target repository shape

Concepts start as modules, not crates. A package is split only for a distinct
runtime, dependency budget, real independent consumer, or deployment/release
artifact. Size and hypothetical reuse are not sufficient.

```text
onmark/
├── AGENTS.md  CLAUDE.md  README.md
├── crates/
│   ├── core/       # pure compiler, domain model, diagnostics, IR
│   ├── media/      # asset probing without Chromium
│   ├── render/     # browser, FFmpeg encoding, local executor
│   └── cli/        # native tool face and composition root
├── packages/
│   ├── runtime/ authoring/ motion-gsap/ bundler/ launcher/
├── scripts/     # repository-only generation and quality checks
├── deploy/aws-lambda/  # Rust Lambda/S3 adapter after artifact conformance
├── schemas/ conformance/ evals/ docs/
```

The completed Gate-one milestone contains `onmark-core`, `onmark-media`,
`onmark-render`, `@onmark/runtime`'s browser session, `@onmark/authoring`'s
semantic DOM bindings, `@onmark/bundler`'s presentation artifact boundary, and
the first `onmark-cli` whole-film composition root. Media is a separate crate
because server-side compile/lint loops need probing and standalone-subtitle
normalization without Chromium; this is both a distinct dependency budget and a
real consumer. Runtime is a separate package because it executes inside the
browser and is consumed by authoring and bundling. Authoring is a separate
browser package because user presentations consume its public DOM contract
independently while runtime must not depend upward on author-facing effects;
its only product dependency is runtime's types-only public surface. Bundler is
a separate package because it executes under Node, owns the esbuild and
filesystem dependency budget, and produces a presentation directory consumed
independently by native rendering. The CLI is a distinct release artifact that
combines core compilation, media probing, the bundler process, and native
rendering without moving their implementation details into one crate. The
private launcher package is the Node/npm process boundary for that artifact. It
may depend on Node built-ins and the pinned browser installer, and it may start
only the product bundler and native CLI; no product package depends back on it.
The optional GSAP adapter is an internal package because its vendor dependency
budget is independent of the vendor-free authoring facade. Inside the source
workspace, authors consume the `onmark/motion/gsap` facade. The root package
export map owns that mapping, and
the bundler resolves every public `onmark/*` import through the map instead of
selecting individual vendors. The source-workspace mapping alone is not a
product contract; the desktop admission below is its release owner.
`onmark-aws-lambda` is a distinct Rust release artifact
because Lambda is a different runtime and its handler owns the
`aws-config`, `aws-sdk-s3`, and `lambda_runtime` dependency budget. Its
deployment-only browser boundary additionally uses `sha2`, `tar`, and `zstd`
to verify and expand one bounded immutable payload. The package-only
`onmark-aws-lambda-package` binary adds deterministic ZIP encoding without
linking the AWS runtime; it is a deployment-operator tool, not repository
automation or an authored-video command. Those archive types and policies stop
at the adapter; `onmark-render` receives only an executable path and discovers
optional adjacent runtime sidecars for the Chromium child. The adapter may
consume `onmark-render`'s portable worker request and `onmark-core`'s canonical
bundle layout, but neither dependency knows about AWS or Lambda packaging.
Syntax, diagnostics, model, compiler, timeline, and protocol remain rectangular
modules inside core. Render graph and planning initially join core at gate two.
Worker execution belongs to render. Remote orchestration remains an external,
short-lived composition concern unless a later product proves that durable
coordination is necessary.

### Desktop release artifact

The desktop product exposes one `onmark` package, one `onmark` command, and the
`onmark/authoring` and `onmark/motion/gsap` facades. Internal workspace packages
are implementation modules, not installation steps.

The private launcher is a thin npm boundary rather than a second CLI. It selects
one optional platform package and passes explicit Node, bundler, browser
provisioner, FFmpeg, and ffprobe paths to the native command. Rust still owns
arguments, diagnostics, compilation, rendering, and exit status. Browser
provisioning begins only after authored diagnostics, and no ambient executable
is a silent fallback.

All release automation is co-located under `scripts/release/`.
`assemble-package.mjs` projects already-built TypeScript modules into the
32 MiB public package, closes internal declaration imports, and hashes every
payload except its own manifest. `cargo xtask release sidecar` admits the native
`onmark`, FFmpeg, ffprobe, source archives, build record, and licenses into one
target-specific package with a 384 MiB ceiling. Both assemblers use private
staging and one final rename; neither compiles source, installs dependencies, or
publishes.

`media-sources.json` fixes every media source by URL, byte length, and SHA-256.
The adjacent fetcher is the only network owner, while `build-media.sh` consumes
only admitted local archives with autodetection, network, shared libraries, and
nonfree components disabled. The sidecar rechecks the source manifest and build
script byte for byte before admitting target binary formats and provenance.

`packages/launcher/desktop-release.json` is the single supported-target and
browser contract. It owns the pinned Chrome for Testing build, browser product,
and archive digest; the native sidecar assembler rejects a differing target
matrix. The launcher installs the selected browser through a bounded
cross-process lock into a content-addressed cache and publishes it by atomic
rename. Each lease has an owner-specific heartbeat marker; a reclaimed owner
cannot publish cache bytes or refresh or remove its successor's lock.

The manual desktop-release workflow admits macOS arm64, Linux x64, and Windows
x64 only after installing the two produced npm tarballs into an empty consumer
and rendering the same screenplay in two independent browser sessions. It
checks exact frame count, decoded audio, canonical raw-RGBA identity,
product-import bundling, and no-clobber output. Each target artifact also
retains the two real CLI render durations with its fixed profile; shared runner
timings are evidence samples, not release thresholds. Cross-compilation and
binary format inspection alone never establish target support.

The Lambda ZIP remains a separate deployment artifact. Its bootstrap, archive
budget, `/tmp` lifecycle, and S3 contract are not reused as desktop installer
semantics.

### Product commands and language evidence

Gate one's native command is deliberately narrow: `onmark render <screenplay>`.
It bundles a neutral semantic DOM presentation plus optional same-stem
`film.css` and `film.motion.ts` files unless `--presentation` explicitly selects
custom browser code. The motion module exports one declarative `motion` value;
the generated entry owns runtime installation. It
derives the stable no-clobber output `renders/<screenplay-stem>.mp4`
unless `--output` is supplied, and exposes only exact frame rate and viewport
dimensions as ordinary render controls. Process paths are execution overrides,
not screenplay facts. Authored diagnostics are emitted before executable
preflight so an invalid screenplay never requires Chromium, Node, or `FFmpeg`
merely to explain itself. Gate three adds the deliberately separate worker entry
point `onmark worker capture`: it accepts one versioned `request.json`,
including the deployment-owned SHA-256 identity of its locked capture
environment, the `bundle/` payload files named by that manifest, and frozen
`assets/sha256/` bytes. The identity covers the deployment's Chromium, fonts,
launch configuration, and other pixel-affecting host facts; the renderer
deliberately does not guess it from one executable path or browser-version
string. The worker materializes inputs in a private root and publishes one frame
artifact. Reuse and assembly require that environment identity alongside the
unit identity. The command accepts no screenplay and never recompiles source;
the short-lived invocation owner or object-store adapter publishes requests.

The CLI reads screenplay source through an 8 MiB sentinel-bounded UTF-8 reader
before core parsing, and core independently enforces syntax byte, retained-item,
and nesting limits. Worker capture applies the same boundary discipline to its
different trust domain: request JSON is capped at 16 MiB by one render-owned
constant shared by the local command and deployment adapter. Neither entry point
uses a convenience whole-file read that can allocate from an untrusted file
length.

`onmark-cli` resolves every external executable before starting process work,
then follows one linear path: read and compile, freeze and probe referenced
assets, solve Timeline IR, bundle the presentation, compose and materialize the
whole-film unit, render, and atomically publish. Freezing streams each
referenced source into a private temporary file while hashing it, then probes
only that private copy; identity and metadata therefore describe the same
retained bytes. Hashing and probing run on explicit blocking work rather than a
Tokio worker. The CLI depends on core, media, and render as its real composition
inputs; `clap` owns argument parsing, `sha2` owns streaming SHA-256, `tempfile`
owns private lifetimes, `serde_json` decodes the Rust-owned manifest, and Tokio
owns bounded process and render async work. No CLI dependency enters the pure
core.

`evals/` is checked-in language-product evidence, not a runtime package or a
live-model CI service. It owns frozen cases, prompts, grader rules, raw outputs,
model settings, and comparison baselines. Those assets are added only from real
experiments; the repository does not create an empty framework or invent a
historical baseline when the source material is unavailable. Repository
automation may parse and rescore frozen outputs without network access; it
never turns CI into a live-model benchmark.

### Dependency budgets and module direction

Core's internal dependency DAG is CI-enforced: `model` depends on nothing;
`syntax`, `diagnostics`, and `timeline` may depend on model; `render_graph` may
depend on model and timeline; `compiler` may depend on model, syntax,
diagnostics, and timeline; `protocol` may depend on model, diagnostics, and
timeline. No domain module depends back on protocol. New edges require an
architecture change. CI performs a syntax-aware check of explicit Rust paths
with `syn`. This cooperative guard covers ordinary paths, imports, aliases, and
re-exports, but not paths generated inside macros or full rustc name resolution;
review remains responsible for those edges.

`onmark-core` uses `xmlparser` only inside `syntax` for pure, span-preserving
XML-compatible fragment tokenization. Onmark owns tree construction, nesting
checks, duplicate-attribute checks, reference decoding, and all authored
semantics. Parser errors are translated at the syntax boundary and the
dependency performs no IO. Test targets may use `proptest` for time algebra and
`syn` for the cooperative module dependency-law check; neither development
dependency is linked into library consumers or runtime artifacts.

`@onmark/runtime` remains vendor-free and owns exact frame-effect and resource
lifecycles. `@onmark/authoring` owns the semantic DOM and the vendor-neutral
`PresentationExtension` contract; its `/types` subpath exports declarations
only, so optional adapters cannot acquire authoring runtime behavior through
that dependency edge. The internal `@onmark/motion-gsap` package backs the
workspace `onmark/motion/gsap` facade. It alone owns the pinned GSAP dependency,
converts exact Rust-owned intervals to local browser seconds, suppresses
callback dispatch while seeking, and kills every playhead on terminal disposal.
It may depend only on `@onmark/authoring` and GSAP, and is consumed by authored
motion modules or custom presentations. Other engines implement the same
extension contract; neither bundler nor runtime selects vendors. Three.js
remains a repository development dependency until an equally narrow production
adapter is admitted.

The browser projection preserves film, scene, shot, and content ownership from
Timeline IR. Every projected node has a compiler-owned identity stable across
whole-film and Render Unit projections, an optional authored ID, and a solved
interval where applicable. Videos and authored overlays name their owning shot;
imported captions remain film-level. The wire remains a flat relational plan so
native validation and partition projection stay bounded, while the default
authoring adapter builds one nested semantic DOM tree. TypeScript never
reconstructs source structure or resolves timing from array order.

The `protocol` module uses `serde` for stable browser and bundle-manifest JSON
boundaries. Its optional `schema` feature exposes `schemars` only to repository
generation; product binaries do not enable it. All repository-only automation
lives under `scripts/`; it is not a product package or a miscellaneous
application layer. Its Cargo manifest exists solely to give the Rust schema
generator a pinned build-only dependency budget and a stable `cargo xtask` entry
point. That binary is consumed only by developers and CI and may depend on core
and `onmark-aws-lambda` with their `schema` features, `schemars`,
`serde`/`serde_json`, `sha2` for native release identities, and `tempfile` for
private sidecar staging; it disables the Lambda package's default runtime feature, so
schema generation does not link AWS. Product crates and packages never depend
on it. The Lambda dependency exists solely to publish that deployment
boundary's schemas, not to smuggle AWS into core. The adjacent Node generator
may use the pinned schema-to-TypeScript and validation toolchain. `cargo xtask
schema` writes every versioned schema, then invokes that generator. `cargo
xtask eval audio` independently regrades the frozen audio-language experiment.
The adjacent release scripts assemble the public npm package and admitted media;
`cargo xtask release sidecar` assembles only the native platform payload. None
of these tools installs or publishes product artifacts.
`json-schema-to-typescript` emits reviewable browser types into runtime and the
manifest type into bundler; Ajv emits standalone browser validation code at
build time. The Lambda schemas intentionally have no TypeScript codec until a
real TypeScript caller exists. Oxlint, a narrow repository-shape check, and
Prettier are repository-only development tools and never enter a product
artifact. Real-process CI uses the pinned `@puppeteer/browsers` installer to
fetch the exact Chrome for Testing headless-shell build under test; the desktop
launcher uses the same library as a production dependency to verify and expand
its admitted release archive. The browser runtime does not compile schemas
dynamically. Exact tool versions are pinned in the lockfiles and `mise.toml`,
and CI rejects stale generated artifacts.

### Media normalization boundary

`onmark-media` depends on core plus `serde` and `serde_json` for a private
ffprobe response boundary. It starts the configured ffprobe executable directly
with an argument array, never through a shell; wrappers that leave descendant
processes holding the output pipes are outside this executable contract. Process
lifetime and retained stdout/stderr bytes are explicitly bounded under that
direct-child contract, both pipes are drained concurrently, and explicit
shutdown reports process-control failures while `Drop` remains a best-effort
termination fallback. Private ffprobe response types are translated once into
core-owned `AssetMetadata`; JSON values and third-party error types do not
define the stable API, while underlying errors remain available through the
standard error source chain for debugging. Gate-one probing requests bounded
stream-level facts for every stream. Attached-picture video streams are not
renderable media. Among the remaining video streams and among audio streams,
the declared default stream wins; ties and absent defaults resolve to the
lowest reported stream index. `sample_rate` and `channels` fix the selected
audio stream's sample grid and normalized channel layout, while `nb_frames`
identifies stills. It
prefers a visual stream's duration and falls back to the container duration
when ffprobe omits that stream field; a malformed explicit stream duration is
still rejected rather than hidden by the fallback. It
classifies a visual stream as constant only when ffprobe's parseable
`avg_frame_rate` and `r_frame_rate` normalize to the same exact rational rate;
disagreement or unavailable values are conservatively variable. The one-MiB
stdout ceiling therefore remains independent of media duration.

Gate four also gives `onmark-media` the standalone-subtitle syntax boundary.
Its parsers consume caller-owned bytes under explicit input, cue-count, per-cue
text, and fixed retained-error limits, then return source-located format errors
or core-owned `CaptionTrack` facts with exact nanosecond intervals. They perform
no filesystem access, encoding guess, styling interpretation, diagnostic-code
translation, or browser layout. The initial formats are strict UTF-8 SubRip,
a lossless plain-text WebVTT subset, and a lossless plain-event ASS subset; all
accept an optional UTF-8 BOM and LF or CRLF line endings. WebVTT comments and
cue identifiers carry no rendered facts and may be discarded, while regions,
style blocks, cue settings, markup, and escapes remain explicit unsupported
errors. Plain ASS accepts `ScriptType: v4.00+`, safe script metadata, and
`Format: Start, End, Text`; it converts exact centisecond timing plus `\N` and
`\h`, while resolution, styles, layout fields, effects, override tags, drawings,
and other presentation semantics remain explicit unsupported errors. The three
formats share the same fact boundary and add no production dependency.
The CLI selects one parser from the authored file extension and translates its
format-local errors exactly once into `ONM-CAPTION-*` diagnostics before
presentation validation, media probing, or browser launch. File open and read
failures remain typed infrastructure errors.

### Browser and encoder boundary

`onmark-render` owns the heavy Chromium and `FFmpeg` dependency budget. It uses
`chromiumoxide` only as a CDP transport. Onmark launches and reaps headless
shell itself so stderr remains continuously drained into a bounded diagnostic
tail after the `DevTools` endpoint appears. It uses `base64` only to decode
CDP's required screenshot envelope, `png` only to decode a captured screenshot
into canonical RGBA for its renderer-owned fingerprint, and `tokio` and
`futures` only within this asynchronous execution boundary. `tempfile` gives
every browser session an isolated profile, gives each output a private
same-filesystem staging directory, and gives each executable unit a private
RAII resource root.

Unit-root materialization uses `serde_json` only for the Rust-owned manifest
encoding, `sha2` for streaming identity verification, and `url` for the browser
entry URL. File bounds are rejected before identity work; canonical hashing and
manifest sizing stream through fixed-memory writers, and the pretty manifest is
serialized directly into the private root. It rejects symlinks and non-files,
copies verified bytes instead of linking mutable source paths, and bounds both
retained file count and total bytes. The fixed safety envelope is 100,000 files
and one tebibyte, while each caller supplies a smaller explicit policy. Parallel
sequences therefore share neither Chrome locks nor admitted input paths, and a
completed MP4 is published with a no-clobber hard link only after both processes
finish cleanly.

The crate supplies executable paths, viewport, browser process and request
deadlines, an encoder inactivity timeout, frame/input/capture-byte ceilings,
bounded retained stderr, and explicit shutdown. Chromium, CDP, and subprocess
types are translated into stable render-owned errors. Encoder lifetime remains
bounded by finite frame and byte budgets plus timeouts on every write and
finalization operation. The exact video-encoder thread count is part of that
explicit host policy: the local CLI defaults to four threads, accepts a bounded
explicit override, and the portable worker retains one. Neither path derives it
from ambient CPU count. Time spent awaiting Chromium does not consume an encoder
inactivity budget. Browser navigation waits separately for document load and
the runtime host because the transport's navigation call does not itself
establish that lifecycle barrier.

Gate one captures one PNG at a time and writes it directly to `FFmpeg`'s
`image2pipe`; there is no frame queue or whole-video buffer. The fixed H.264
`yuv420p` profile rejects odd viewport dimensions before either process starts.
Browser capture has one closed backend choice beneath the shared runtime
protocol. `BeginFrame` atomically commits and reads a compositor transaction on
Linux `chrome-headless-shell`; `Screenshot` reads the surface with
`Page.captureScreenshot` after the same typed `Seek` readiness barrier and uses
the same post-capture `Confirm` and placement-boundary reconciliation. The
portable backend exists for macOS and Windows, where BeginFrame control is not
available. It does not introduce a second clock, timing solver, plan, encoder,
or media-selection path. The selected backend is reported and belongs to the
capture-environment identity; equality is asserted only within an equivalent
locked environment and backend.

Conformance launches a pinned Chrome for Testing browser and `FFmpeg` against
the production presentation adapter, crosses the typed
`Load`/`Prepare`/`Seek`/`Confirm` handshake, probes the resulting H.264 MP4, and
verifies decoded motion. The checked-in bundle fixture carries real payload
bytes, is rebuilt byte-for-byte in the bundler test, and crosses the generated
Node/native manifest contract through native materialization. The outermost CLI
conformance starts two independent whole-film sessions, validates each decoded
output's frame count, motion, stream facts, and audio placement, then checks
no-clobber publication. Canonical raw-RGBA equality remains a native
capture-boundary assertion; independently encoded lossy MP4 frames are not an
identity oracle. CI owns explicit browser and media-tool versions for these
tests. Linux locks the canonical BeginFrame path; desktop release admission
locks the portable screenshot path on macOS and Windows.

GitHub-hosted Ubuntu applies AppArmor user-namespace restrictions to downloaded
Chrome for Testing binaries. Desktop release admission installs a runner-local
AppArmor profile that grants `userns` only to the content-addressed Onmark
browser-cache path, preserving Chromium's own sandbox. The lower-level
real-process suite still uses a disposable runner-local `--no-sandbox` wrapper;
neither exception changes product launch policy. Product and local browser
launches keep Chromium's sandbox enabled by default. The canonical default and
every worker policy explicitly use ANGLE's `SwiftShader` backend: host GPU
availability must not silently change pixels or make whole-film and partition
captures disagree. Desktop execution on macOS may instead select the explicit
`Metal` graphics backend. It reads the active GL renderer back through CDP
before page execution, and rejects software fallback. This is a distinct capture
environment, not a faster implementation of the `SwiftShader` identity:
backend-sensitive WebGL pixels are expected to differ. An opt-in macOS
conformance test proves exact raw-RGBA repetition across independent Metal
sessions, repeated and
out-of-order seeks, and WAAPI, GSAP, and Three.js effects. It also retains the
software sequence so an accidental collapse of the two environment identities
is visible. The macOS CLI selects this verified backend and reports it beside
the capture mode. Linux and Windows retain `SwiftShader`; another native
backend requires its own admission evidence and explicit variant.

The initial locked macOS performance run used Chrome for Testing
149.0.7827.55 on an Apple M5 and the release CLI to render the checked-in
CSS/GSAP presentation at 1,920×1,080 for 45 frames. Three independent
`SwiftShader` runs took 10.80, 6.61, and 6.56 seconds; three Metal runs took
7.28, 4.66, and 4.73 seconds. The warm pair is about 29% faster. These are
end-to-end CLI samples, including compilation, bundling, browser launch,
capture, and encoding; they do not justify a wider cross-platform claim.
`--graphics software` retains the reproducible control path.

The local CLI assigns four threads to the final H.264 encoder by default.
`--video-encoder-threads` admits an explicit value from 1 through 64 for hosts
with a different CPU or memory budget. Onmark never derives this value from the
ambient core count: doing so would make encoding resources and output bytes
change silently across hosts. Portable capture workers retain their
deployment-owned one-thread policy and scale through partitions; they do not
encode the assembled MP4.

One validated local partition sequence retains one Chromium process and one
continuous encoder. Each unit still receives a fresh runtime navigation, typed
`Load`/`Prepare`/`Dispose` lifecycle, private resource policy, and empty
screenshot cache. Retiring the preceding resource guard before installing the
next root prevents process reuse from widening file access or reusing a
no-damage frame across units. Worker artifacts remain one unit per browser
process. A 640×360, 30 fps exploratory M5 run over identical 100 ms semantic-DOM
shots exposed the fixed cost: four units fell from 2.49 to 1.29 seconds and
eight from 5.22 to 2.18 seconds after process reuse. Whole-film and partitioned
decoded-frame hashes remain equal in real-process conformance; the sample
motivates the lifecycle boundary but is not a release performance threshold.

The same sequence may reuse a browser foreground capture only when the bundle
declares `placementBounded` and the admitted unit proves that Chromium owns no
video pixels. Reuse stops at every plan-owned placement boundary and never
crosses a unit navigation. Local rendering and worker capture consume the same
serialized cadence; neither path grows an independent cache policy.
The real-process layered fixture compares this path with an otherwise identical
`everyFrame` bundle: all 75 output frames enter browser capture in the control
and only one does in the placement-bounded candidate, while their canonical
raw-RGBA sequences remain exactly equal across independent processes. Bounded
retry and reconciliation readbacks remain visible in phase timing rather than
being counted as additional authored frames.

A release real-process run on an Apple M5 with Google Chrome 150.0.7871.186,
the `Screenshot` backend, `SwiftShader`, and the 75-frame 1,920×1,080 layered
fixture separated authored capture work from actual Chromium commands. The
`everyFrame` control entered capture 75 times and issued 76 pixel commands;
readback, pixel processing, and native write took 3.63 seconds, 6.7
milliseconds, and 298 milliseconds. The `placementBounded` candidate entered
capture once and issued two commands; the same phases took 367 milliseconds,
0.07 milliseconds, and 169 milliseconds. Their canonical raw-RGBA sequences
were exactly equal. The roughly one-second local browser launch in both runs is
not Lambda evidence: the current cadence has not been deployed, and historical
Lambda launch samples used different code and binaries.

An Apple M5 encoder isolation run then compared the layered path with one, two,
four, and eight x264 threads over the same 45-frame 1,920×1,080 input. Median
wall times were 1.08, 0.84, 0.68, and 0.63 seconds; observed process peak RSS
was approximately 533–541, 545, 561–577, and 605–615 MiB. Four threads retain
most of the speedup without the last step's memory cost. Repeating the complete
release CLI with that fixed policy produced byte-identical MP4s at 4.17 and
3.99 seconds after warm-up. Changing the thread policy is not itself an
identity comparison: x264 is lossy and may produce slightly different decoded
pixels even from the same canonical input, so raw RGBA remains the
deterministic visual oracle.

Local capture retains Chromium's normal multiprocess topology. An adapter may
select `BrowserLaunchPolicy::isolated_worker()` only when an independently
audited outer container or microVM owns equivalent process isolation. That
policy also keeps the renderer and SwiftShader GPU in one process and disables
the unavailable zygote rather than disabling graphics. The deployment-owned
choice is part of the locked capture environment, is never selected by authored
input or a worker invocation, and must be proven in its real execution
environment before it is treated as a production launch contract. A failed
Chromium launch never causes an automatic downgrade.

Gate-one native browser operations and decoded-video waits accept at most a
one-day deadline, keeping every platform timer inside an explicit supported
horizon.

Validation reasons remain local domain values. Once syntax has supplied source
context, the `compiler` module is the single owner that translates reasons such
as `InvalidNodeId` into source-located `Diagnostic` values, including
phase-specific messages and help. `diagnostics` owns only the generic diagnostic
representation and stable codes. Neither `model` nor `syntax` depends on
diagnostics, and the translation must not be duplicated by callers.

### TypeScript product direction

On the TypeScript side, runtime is the foundation. Authoring's root entry
consumes runtime's types-only contract and creates semantic video and overlay
DOM. The bundler-generated neutral entry composes runtime values around those
bindings. CSS and custom entries still own every visual decision. Bundler
injects the pinned authoring and runtime artifacts. Runtime never depends on
authoring or bundler. `stateless`, `warmup`, and `sequential` are architectural categories
today, not a public capability-declaration API; when that extension point
becomes real, runtime will own it. The Gate-one `RuntimeSession` owns protocol
ordering, interval-relationship checks, exact-frame projection, and terminal
disposal. It rejects concurrent commands instead of growing a hidden queue and
gives the adapter a recursively frozen snapshot of accepted plan facts.
Browser-specific work enters through one narrow adapter whose waits must be
bounded and whose expected failures are typed. The production presentation
adapter receives presentation-owned elements, sources, and visibility effects.
It owns bounded media loading, exact source-frame selection, decoded-frame
readiness, solved overlay visibility, and terminal cleanup without creating
layout or canvas state. Gate six adds a closed `image | font | texture | custom`
resource boundary at this same owner. A presentation may retain at most 256
uniquely identified resources; `Prepare` starts them concurrently under one
shared readiness policy, reports every timed-out `kind:id:prepare`, and terminal
cleanup awaits all of them in declaration order. The materialized asset
directory used by that adapter and by the bundler is generated from the Rust
bundle schema.

`@onmark/bundler` is the Node-only product build boundary, not repository
automation. It may depend on Node built-ins, the product `@onmark/authoring` and
`@onmark/runtime` entry points, and the pinned `esbuild` production dependency;
browser packages never depend back on it. Gate one compiles one ESM
presentation, substitutes the pinned authoring and runtime entries, emits a
fixed document shell, and records every presentation payload file in a stable
SHA-256 manifest. The package exposes the same operation through the narrow
`onmark-bundle` executable so the native CLI does not import Node or esbuild
types. That child process receives explicit entry, output, and
retained-byte-limit arguments, writes no success payload to stdout, and reports
a stable failure category on stderr. The native caller applies its own process
deadline, drains diagnostics continuously while retaining only a bounded tail,
and parses the resulting manifest back through the Rust-owned wire type. The
manifest shape and layout constants are generated from the Rust protocol
contract rather than handwritten again in TypeScript. The build has an explicit
retained-output byte ceiling, writes through a private sibling staging
directory, and refuses an output path observed to exist before compilation or
publication. The final directory rename prevents readers from observing a
normally completed partial build, but portable Node filesystem APIs do not make
the preceding absence check a cross-process no-clobber transaction. The current
Gate-six resource slice configures one closed `file`-loader set for local AVIF,
GIF, JPEG, PNG, SVG, WebP, OTF, TTF, WOFF, and WOFF2 imports. Esbuild emits
their original bytes beneath an opaque `resources/<hash>.<extension>` path.
The bundler normalizes esbuild's uppercase Base32 names to the bundle contract's
lowercase portable spelling and rewrites generated references at that same
boundary; the existing manifest then owns the canonical SHA-256 and
retained-byte bound.
This step performs no image or font decoding and is not browser-readiness
evidence. The boundary deliberately has no watch mode, plugin API, cache,
development server, external fetch, or general asset-transformation policy.
Esbuild's internal working memory remains governed by the pinned third-party
implementation rather than the retained-output ceiling.

`@onmark/launcher` is the private Node/npm boundary of the public desktop
artifact. It may depend only on Node built-ins and the pinned browser download,
proxy, and ZIP extraction libraries. It selects one admitted platform sidecar,
passes explicit product-tool paths into the native CLI, and owns the verified
browser cache. It does not import bundler, compiler, timing, render-graph,
browser-runtime, or authoring semantics. Its only consumers are the generated
public npm package and release conformance.

### Wire generation and limits

Rust wire types are the source of truth. `cargo xtask schema` generates
checked-in versioned JSON Schema, and CI requires regeneration to produce no
diff. A schema with a real TypeScript consumer also generates checked-in
types/codecs; the browser and bundle contracts do so today, while the Lambda
invocation deliberately has no speculative TypeScript caller. Generated files
are never hand-edited, and Rust does not regenerate a second Rust model from its
own schema. Before the first external Gate-one release, v1 is refined in place
so the initial public contract does not preserve experimental fields; after
publication, an incompatible wire change requires a new protocol version and
migration fixture. The `BrowserPlan` carries the output frame rate,
evaluation/output intervals, film, scene, and shot structure, primary-video
placements, and title, call-to-action, or imported-caption overlays consumed by
the production presentation adapter. Every projected node has a compiler-owned
identity stable across unit projections and an optional authored identity;
content names its structural parent. Video placements additionally identify
immutable bytes and the admitted CFR source rate needed to verify decoded frame
selection, while overlays carry their closed semantic role and decoded text.
Materialized URLs remain render-owned facts, while DOM structure and CSS remain
presentation-owned effects. This is the browser-facing projection of one Render
Unit, not the Render Graph or partition plan itself. It may contain only facts
consumed in the browser; output paths, cache keys, `FFmpeg` arguments, source
spans, and materialization policy remain outside it. VFR timestamp maps and
further component facts are added only when the production adapter consumes
them.

Protocol V1 carries at most 10,000 scene containers, shot containers, video
placements, and overlays of each kind; one overlay inscription carries at most
65,536 Unicode characters.
Native projection and Rust wire decoding additionally cap their combined UTF-8
text at one MiB before CDP serialization. That aggregate process budget is not
misrepresented as a JSON Schema structural constraint.
One failure carries at most 4,096 message characters and 256 pending-resource
descriptions of at most 1,024 characters each; the producer owns their
deterministic order. The runtime-host property name and these resource limits
are generated from Rust-owned schema metadata, so native execution, browser
runtime, and validation do not maintain handwritten copies.

### Lambda adapter

AWS Lambda is an adapter, not another engine. The checked-in Rust
`onmark-aws-lambda` binary owns V1 invocation/result schemas, the thin handler,
and S3 operations. It downloads one portable worker layout, checks that the
request's capture environment equals the deployment identity, materializes the
Render Unit through `onmark-render`, verifies the finished artifact, and
conditionally publishes it by its renderer-owned identity. A `412` means
"download, verify, and compare the already-published raw-RGBA artifact"; a
bounded `409` retry is only a conditional-publication transport retry, not a
distributed retry policy.

The deployment supplies either an already-expanded headless shell or one
zstd-compressed tar archive plus its canonical SHA-256 digest. Archive
materialization is bounded by compressed bytes, expanded bytes, and entry count,
and rejects traversal, duplicate paths, links, special files, digest drift, and
a non-executable shell. Optional fonts receive a private fontconfig file and
cache. The renderer scopes that file, adjacent shared libraries, and the
SwiftShader manifest to the Chromium child; no process-global environment is
mutated. Browser preparation is lazy and one-time per Lambda execution
environment, so the Runtime API starts before heavy browser I/O and warm
invocations reuse the verified private installation.

The package-only `onmark-aws-lambda-package` binary consumes a prebuilt
`provided.al2023.arm64` bootstrap, a self-contained Linux arm64 `FFmpeg`
executable, and an expanded browser root. It sorts
portable browser paths, rejects links and special files, normalizes tar
ownership, modes, and timestamps, applies a fixed single-threaded zstd policy,
and fixes ZIP order, timestamps, permissions, and compression levels. Its
sibling-staged output directory contains the ZIP and a canonical manifest with
SHA-256 identities for the bootstrap, browser archive, `FFmpeg`, and final package. A
final directory rename hides normally completed partial output, but portable
filesystem APIs cannot turn the preceding absence check into a cross-process
no-clobber transaction. The capture-environment identity conservatively covers
the bootstrap, browser archive, `FFmpeg`, target, and isolated-worker launch
policy. The bootstrap digest also owns the native composition policy compiled
into `onmark-render`. This proves identical outputs for identical locked inputs;
cross-compilation remains the job of a pinned Linux arm64 builder such as Cargo
Lambda, not of the packager. Packaging rejects non-Linux-arm64 executables and
reserves ten MiB beneath Lambda's 250 MiB unzipped-package ceiling.

The deployment config owns S3 transport budgets: a five-second connection
timeout, a 45-second attempt timeout, a 90-second operation timeout, and at most
three SDK attempts. Since `GetObject` returns a response stream after the SDK
operation has completed, every pending body read separately has a 30-second
progress deadline. This prevents a stalled stream from becoming an unbounded
worker wait without pretending that it is a scheduler or lease policy.

This JSON contract has checked-in Rust-generated schemas. It intentionally has
no generated TypeScript SDK because no TypeScript caller exists yet; creating a
remote orchestration client merely to satisfy symmetry would invent a consumer.
AWS SDK and browser-archive types may not enter core or render. One real arm64
Lambda experiment used a 92.4 MB function ZIP at 4,096 MiB to prove outer
isolation, constrained-process BeginFrame capture, and immutable reuse for a
locked 30-frame 320×180 title-only fixture. Three independent cold environments
completed in 3.005, 2.277, and 3.069 seconds with 455–457 MB peak memory; one
immediate warm reuse completed in 1.325 seconds. Repeating one cold run at two
GiB completed in 3.069 seconds with 454 MB peak, while one GiB took 5.080
seconds with 451 MB peak. Two GiB is therefore the measured latency/cost knee
for this small fixture, not yet a production default. A controlled 249 MiB
expanded browser in a fresh container-image layer instead made capture take 30.9
seconds, and pre-runtime archive expansion exhausted Lambda's ten-second
initialization window. These measurements select ZIP delivery plus
invocation-owned preparation for this environment; they do not generalize to
other workloads. The reviewed packager replaces the hand-built ZIP procedure,
but release publication and infrastructure definitions remain experimental until
separately reviewed. Other backends such as GCP, ECS, or Kubernetes follow the
same adapter rule and consume the same worker request and artifact format. They
own their own SDK, transport semantics, and release artifact; Lambda environment
variables, ZIP layout, and S3 policy are not a generic cloud interface.

### Deployment performance evidence

A separate decoded-media experiment measures the steady capture path rather than
package delivery. One 1,920×1,080 H.264 fixture produced 60 output frames at 30
fps with identical canonical raw-RGBA fingerprints across current independent
cold environments. Individual warm capture samples were 22.07 seconds at two
GiB, 13.00 seconds at four GiB, and 7.91 seconds at eight GiB; corresponding
warm costs were 47.11, 58.72, and 73.46 GB-seconds. Peak memory remained 600–616
MB, so the configured tier primarily bought CPU: two GiB minimized measured
cost, eight GiB minimized latency, and four GiB was the compromise. At eight
GiB, 60 frames spent 2.96 seconds in runtime staging and media seek, 3.83
seconds in BeginFrame screenshot readback, and 0.79 seconds in PNG decoding plus
canonical fingerprinting. Confirmation and artifact writes together remained
below 0.2 seconds. These single samples identify seek and screenshot transport
as the next optimization targets; they do not freeze a production memory tier.

The earlier 66-second observation was a correctness failure, not a cold-start
measurement: the old frame handshake waited until its deadline, then the AWS
CLI's default 60-second read timeout retried while the first invocation was
still running. Synchronous conformance disables client retries and owns a read
timeout longer than the worker deadline.

## Delivery gates

**Gate one (complete): render one real video reliably.** The completed milestone
includes the minimal language, frozen asset catalog, media probing, Rust timing,
versioned Timeline IR, immutable presentation bundle, deterministic browser
clock, frame handshake, and one whole-film Render Unit through Chromium/FFmpeg.
It executes and muxes authored voice-over rather than silently dropping it.
Native conformance compares canonical raw-RGBA fingerprints across independent
browser sessions. Release-CLI conformance renders the screenplay twice,
validates each H.264/AAC output's frame count, motion, stream facts, and audio
placement, and proves no-clobber publication. It does not mistake independently
encoded lossy MP4 output for the raw-frame identity contract.

**Gate two (complete): partition and assemble correctly.** The completed slice
renders two independent local units and assembles them through the existing
executor. Native conformance compares the whole-film and partitioned canonical
raw-RGBA sequences before encoding. Release-CLI conformance separately validates
the assembled H.264/AAC output's frame count, motion, stream facts, and
first-audio-packet placement. It introduces the Render Graph and
evaluation/output intervals. Preroll, persistent unit caching, and
dependency-based invalidation remain deferred until a real dependency or cache
consumer requires them.

**Gate three (complete): leave the machine.** The completed data-plane slice
projects the same deterministic, versioned worker requests used locally into a
bounded Lambda/S3 adapter. Its exit conformance captures one media-bearing
two-shot film as a remote whole-film reference, executes both graph partitions
concurrently on independent workers, compares canonical raw-RGBA frame
sequences, and assembles the verified artifacts through the shared H.264/AAC
path. S3 transport retries and conditional compare-and-verify publication are
bounded adapter semantics, not a distributed retry policy. Canonical Timeline IR
and Execution Plan wire encodings remain deferred until they have an external
consumer; byte-identical MP4 containers are not presumed.

The exit harness is also the gate's complete orchestration proof: one
short-lived owner uploads immutable inputs, invokes the finite set of workers,
downloads and verifies their artifacts, and assembles the result. Gate three
does not require a database, queue, lease service, or long-running coordinator.
Deployment work is frozen after this proof. Provider workflows, public remote
render commands, infrastructure definitions, release publication, and additional
cloud adapters require a later user need and are not part of gates four or five.

**Gate four (complete): authored audio and subtitles.** This gate carried
general audio and user-supplied subtitle files through the existing local
compiler and renderer without weakening exact timing or partition equivalence.
It admitted no language spelling until its evaluation assets and conformance
fixtures satisfied the language admission rule. Its exit contract was:

- narrative voice-over remains distinct from general music and sound effects;
- external TTS audio remains a normal frozen authored asset rather than an
  online generation side effect;
- SRT, WebVTT, and ASS inputs are bounded and normalized into Rust-owned caption
  facts before the browser sees them; unsupported ASS semantics are rejected
  explicitly rather than silently discarded;
- audio placement, gain, duration, subtitle timing, and caption text are exact
  compiler or media facts, never a second browser timeline;
- malformed external files produce source-located authored diagnostics while
  unavailable or unreadable files remain typed infrastructure failures;
- one local media-bearing film with a cross-shot audio bed, a shot-local sound,
  voice-over, and imported captions renders equivalently as a whole film and as
  two partitions; canonical raw-RGBA frames and decoded audio timing/content are
  both checked before the gate closed.

The pinned Linux real-process suite now exercises that complete slice through
both the native renderer and release CLI. The whole-film and two-unit paths
produce equivalent canonical raw-RGBA frames and decoded audio while carrying
film music, a shot-local effect, voice-over, and imported captions. Gate four
added no cloud conformance, deployment command, subtitle editor, speech
generation, or animation adapter.

**Gate five (complete): deterministic browser effects.** This gate began with
bounded CSS, GSAP, and Three.js experiments before admitting a production API.
Its exit contract was:

- the integral `RuntimeFrame.index` remains the sole frame identity; browser
  seconds are only a projection used to set an effect's playhead;
- paused WAAPI animation, a paused GSAP timeline, and a Three.js
  `AnimationMixer` plus explicit render all reproduce the same pixels when
  frames are requested in and out of order;
- the checked experiment repeats that non-monotonic sequence in independent
  locked Chromium processes and compares canonical raw-RGBA fingerprints;
- an admitted frame-effect boundary runs inside `Seek(frame)` and resolves
  before `FrameStaged(frame)`; it cannot create a second scheduler, free-running
  clock, hidden queue, or unbounded readiness wait;
- bundle metadata carries one closed temporal capability owned by
  `@onmark/runtime`; unknown presentation code remains sequential, while random
  access is accepted only for an adapter whose conformance proves that any
  requested frame depends solely on immutable inputs and that exact frame;
- the Render Graph consumes that capability before partitioning. Whole-film
  and multi-unit capture must produce equal canonical raw-RGBA sequences for
  every capability that permits a split;
- official WAAPI, GSAP, and Three.js integration remains vendor-specific code
  above the vendor-free runtime clock rather than dependencies of
  `onmark-core` or `@onmark/runtime`.

Gate five does not add animation spelling to the screenplay, infer capability
from source inspection, virtualize ambient wall-clock APIs, or promise that an
arbitrary component is seekable. Those require separate language or adapter
evidence after this gate.

The checked WAAPI, GSAP, and Three.js playheads all use the standard
`PresentationRuntimeAdapter`: effects bind once during `Load`, apply in declared
order during `Seek(frame)`, and finish before `FrameStaged(frame)`. Disposal is
terminal, releases effects in reverse ownership order, and attempts every owned
effect even after one cleanup failure. The
current bundle manifest binds the closed capability into content identity.
The CLI derives capability from the presentation surface it owns: semantic DOM
without authored CSS or motion is admitted for random access. Any stylesheet,
motion, or custom presentation remains sequential. Explicit capability input exists only at the
low-level conformance bundler boundary. The pinned Linux exit conformance
bundles that effect-bearing presentation, renders the same media, audio, and
caption facts as one whole-film
unit and two independent units, and compares their canonical raw-RGBA frame
sequences before assembling the shared final output.

**Gate six (completed): deterministic visual resources and component binding.**
This gate closed the browser-resource gap before performance work changes the
capture path. Local image, SVG, and font bytes become frozen bundle resources
with stable identities, declared resource facts, byte limits, and no ambient
network fetch. The browser runtime owns one typed, bounded readiness boundary
for video, image decode, font load, texture upload, and explicitly registered
custom resources. A timeout names the pending resource and phase instead of
collapsing into an anonymous presentation promise.

The native browser adapter enforces that promise with CDP request interception,
not presentation convention. Chromium may read only canonical files beneath
the materialized private Unit Root plus in-memory `data:` and `blob:` URLs;
ambient network schemes and file paths outside that root are rejected before
resolution. The same policy runs in local and worker execution.

Presentation bindings also receive Rust-assigned semantic node identities and
parent relationships that remain stable across unit projections, alongside
protocol-validated closed properties, solved intervals, and frozen asset
references.
Rust continues to own timing and resource facts; TypeScript decides only how
those facts become DOM, CSS, Canvas, or WebGL. Gate six does not introduce free
`start`/`end`, a second scheduler, arbitrary network access, or source-code
inference of temporal capability. Any new screenplay spelling for images,
component selection, or properties first requires the language-admission cases,
prompts, graders, raw outputs, and retained baseline.

The exit conformance renders one local film containing a font, image or SVG,
video, captions, authored audio, and one admitted frame effect. Independent
cold Chromium processes must produce equal canonical raw-RGBA sequences, and
every capability that permits partitioning must remain equal to whole-film
capture. Missing, changed, oversized, undecodable, or unready resources must
fail through structured bounded errors that identify the resource. The checked
bundle must remain content-addressed and self-contained.

Gate six does not add parallel browser capture, lossy screenshot transport,
hardware encoding, layered native-media composition, encoded worker segments,
new cloud deployment, transitions, playback-rate control, a component
marketplace, or Studio. Those require separate measured performance or
language gates after this resource contract is complete.

**Gate seven (complete): admitted layered native-media composition.** This gate
may change the authoritative pixel path only for a presentation that explicitly
declares a closed visual-separability capability. Source inspection, an empty
video list, or a successful transparent screenshot is not evidence of that
capability. Presentations without the declaration continue through the existing
Chromium-media path.

The candidate path keeps Rust-owned timing and placement facts unchanged.
Chromium renders only the transparent presentation layer; one persistent native
media process decodes, composites, fingerprints, and encodes the corresponding
base frames with backpressure. Browser capture and native composition form one
bounded stream. Production may not materialize a frame-indexed PNG directory,
buffer an unbounded frame sequence, start one decoder per frame, or silently
fall back between native and Chromium decode/color paths. Local and remote
workers consume the same Render Unit and executor path.

Production admission initially accepts exactly one primary video whose solved
placement equals the published interval, whose frozen source dimensions equal
the output profile, and whose complete color tuple is BT.709 limited range.
This is a layout proof, not a permanent full-screen convention: broader
`cover`, `contain`, crop, and transform behavior requires explicit typed facts
and independent evidence. Rust never infers those CSS decisions. The declared
capability permits both execution plans: materialization records
`SeparableOverlay` only when these facts prove the native path and otherwise
records `BrowserComposite`. This is deterministic planning before launch, not a
runtime fallback; workers execute the transported choice exactly.

Native frame-rate conversion may not inherit `FFmpeg`'s default `fps` rounding.
The candidate projects each source PTS from the Rust-owned source/output
rationals onto the first output frame whose midpoint selects that source frame;
`FFmpeg` may then realize those explicit PTS facts by dropping or repeating
decoded frames. The locked 24-to-30 and nonzero-partition checks prevent this
execution policy from becoming a second timing solver.

Admission is evidence, not implementation intent. One checked, locked Linux
experiment must establish all of the following before the candidate becomes a
production capability:

- two independent cold runs produce equal canonical frame fingerprints within
  the candidate path;
- whole-film and every permitted partitioning produce equal canonical frame
  sequences within that path;
- a controlled color fixture with complete declared range, primaries, transfer,
  and matrix facts stays within the frozen four-level error bound per eight-bit
  channel at all sampled patch interiors; missing, partial, or unsupported
  color facts reject the candidate path rather than guessing;
- source-frame selection remains exact for the admitted CFR profile, including
  nonzero partition starts and repeated source frames under rate conversion;
- at 1,920×1,080, 30 fps, and 60 frames on one locked machine, median end-to-end
  wall time across at least five measured runs is at most half of the existing
  Chromium-media baseline, and the median incremental process-tree peak RSS is
  at most 85% of that baseline;
- the measured interval includes browser launch, readiness, all frame transport,
  native composition, canonical fingerprinting, and final encoding. It excludes
  neither startup nor a stage merely because the stage is shared by both paths.

The experiment records tool identities, machine profile, fixture identity, raw
samples, medians, and rejection reasons. Shared CI enforces correctness and
bounds; noisy performance admission runs only in the pinned environment and is
checked in as reviewed evidence. Passing the thresholds permits a versioned,
explicit capability and its conformance fixtures. Failing any threshold leaves
the experiment opt-in and the production path unchanged.
For this path the capture-environment identity covers the pinned `FFmpeg`
binary and composition policy in addition to Chromium, fonts, launch policy,
and other pixel-affecting host facts.

The reviewed admission measurements, production commits, and closing CI evidence
live in [`conformance/layered-media-admission.md`](../../conformance/layered-media-admission.md).
The production branch retains one compositor across a local render sequence,
one capacity-one frame queue, and one explicit `FFmpeg` framesync lookahead; the
evidence record owns the historical samples and revisions that admitted it.

Gate seven did not add VFR, new codecs, HDR, hardware acceleration, lossy
screenshot transport, parallel browser capture, transitions, playback-rate
control, Studio, component marketplace, or new screenplay spelling.
Those remain separate measured or language gates.

Every gate uses the final-direction contracts but implements only fields
consumed by that gate. A failed gate blocks construction of the next gate's
skeleton.

## Open experimental questions

Layered alpha caching beyond Gate seven's bounded stream, wire encoding,
caption-style normalization, adapter seekability, Windows native-graphics
admission, desktop default policy, and environment-locking granularity require
prototypes and measurements.
Gate-three native capture has selected headless shell's CDP BeginFrameControl;
revisiting that boundary requires stronger correctness and performance evidence,
not API novelty alone. The pure compiler boundary, deterministic protocol,
dependency-driven partitioning, and local/distributed symmetry are foundational
decisions.

The first Gate-one capture spike gave positive but deliberately narrow evidence
for application-controlled `FrameReady` followed by CDP
`Page.captureScreenshot`: repeated DOM, CSS, and Canvas frames produced
identical raw RGBA hashes across independent Chrome processes on one locked
machine. Gate three replaced that provisional transport for the canonical Linux
path with `chrome-headless-shell` BeginFrameControl so compositor commit and
screenshot share one explicit frame boundary. The portable screenshot backend
is admitted on pinned macOS and Windows release targets through independent
whole-film sessions, decoded output checks, and canonical raw-RGBA comparison.
That admission does not claim pixel equality across different operating
systems, browser products, or capture modes.

The decoded-media experiment covers 30 fps CFR, `30000/1001` CFR, and an
alternating-frame-interval VFR H.264 fixture, each with a 30-frame GOP and three
B-frames. Native `<video>` seeking across the non-monotonic sequence
`17 → 3 → 29 → 17` produced byte-identical PNG captures in two independent
Chromium sessions once a pre-capture `requestVideoFrameCallback` registration
confirmed the captured source frame after `BeginFrame`. VFR expectations come
from the probed source-frame timestamps rather than assuming source and output
frames align. Independent `FFmpeg` extraction at the selected source-frame
timestamps was also byte-stable across repeated runs. Seeking to an exact CFR
frame-boundary second selected the preceding frame; sampling inside the
Rust-selected frame produced the intended decoded frame.

The two decode paths are not pixel-interchangeable. Across four 320×180 RGBA
frames, Chromium canvas output differed from `FFmpeg` raw extraction in roughly
229,000–232,000 of 921,600 channels, with mean absolute channel error 2.13–2.18
and isolated maxima 173–178 on the measured machine. Browser
seek/readiness/screenshot averaged 51–81 ms per frame; process-per-frame native
extraction averaged 18–19 ms but excluded browser injection, composition, and
final capture, so the figures are not an end-to-end speed comparison. Gate one
therefore keeps one decode/color path authoritative for a render and treats it
as part of the locked environment. Codec and color diversity, longer random
sequences, persistent native-decoder cost, and injection overhead remain open
measurements.

A later Linux arm64 A/B measured the complete pre-extraction alternative rather
than process-per-frame extraction alone. The locked v149 headless shell rendered
60 sequential 1,920×1,080 frames from one generated 30 fps H.264 source. Native
browser seek plus `BeginFrame` capture completed in 3.89 seconds with a 292 MiB
incremental process-tree RSS peak. One-pass `FFmpeg` 7.0.2 extraction produced
23.4 MB of lossless PNGs in 0.23 seconds; loading those files through one reused
browser image and capturing the same 60 frames took another 2.34–2.38 seconds,
but incremental RSS reached 944–949 MiB across repeated samples. Four sampled
frames differed in 16,665,272 of 33,177,600 RGBA channels, with mean absolute
delta 7.21 and maximum delta 198. The experiment therefore rejects pre-extracted
PNG injection as the default: its roughly one-third latency reduction does not
justify a threefold memory increase and an implicit decode/color-path change. A
future streaming native decoder may reopen the question only with bounded
browser transport, explicit color policy, and equal-or-better end-to-end
evidence.

The follow-up Linux arm64 experiment tested that streaming shape without
injecting media back into Chromium. For the same 60-frame 1,920×1,080 workload,
Chromium captured a sparse transparent presentation layer, exited, and one
single-threaded `FFmpeg` 7.0.2 process decoded the H.264 base, composited the
PNG layer, and streamed RGBA output. Transparent capture took 1.16–1.22 seconds;
native composition took 0.27–0.34 seconds; their sequential total was 1.46–1.52
seconds versus 3.77–3.84 seconds for the authoritative browser-media path. The
two stages peaked at 220–221 MiB and 215–238 MiB incremental RSS, respectively,
and the 60 transparent PNGs occupied 2.96 MB. With the same Chromium-decoded
base on both sides, straight-alpha composition differed in only 6,240 of
33,177,600 sampled channels, with mean absolute delta 0.0002 and maximum
delta 2. Explicitly tagging the source as BT.709 limited range reduced the
complete native-path mean delta from 6.82 to 0.67, but 4,938,423 sampled
channels still differed and isolated maxima reached 202 because Chromium and
`FFmpeg` do not share one decode/chroma-reconstruction implementation. The
layered path therefore proves a compelling performance and memory candidate, not
raw-pixel equivalence. Production keeps Chromium authoritative until frozen
asset metadata owns color facts and a presentation capability proves that media
and browser visuals are separable; it is never a hidden fallback.

Gate one therefore admits CFR H.264 visual assets only and uses the locked
Chromium decoder as the authoritative visual decode/color path. The adapter
seeks inside the Rust-selected frame and does not report readiness until
`requestVideoFrameCallback.mediaTime` identifies the expected source frame.
Unsupported codec or variable-frame-rate input is rejected before rendering, not
silently approximated. VFR becomes admissible only after frozen metadata and the
browser plan carry a complete timestamp map rather than one CFR rate. `FFmpeg`
exact-frame extraction remains an alternative experiment rather than a hidden
fallback that would change pixels within one render.

This policy is represented by render-owned `AdmittedVideo` proof over core-owned
metadata. It borrows the normalized facts instead of introducing a second media
model, and proves both H.264 codec support and one exact source frame rate. The
whole-film Render Unit retains that rate and lowers it into the browser
placement exactly once. The decoded-media conformance obtains the proof from the
production bounded ffprobe boundary for both accepted CFR fixtures and the
rejected VFR fixture. The whole-film executor consumes admitted video through
the production adapter and verifies the completed moving-picture artifact.
