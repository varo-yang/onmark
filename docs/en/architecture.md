# Onmark Architecture

> Status: target architecture. This document separates foundational rules, ordered delivery gates, and later production capabilities.

This document is paired with the Onmark Language Specification. The language defines authored meaning; this document defines execution. Their only contract is the versioned Timeline IR.

Onmark is a screenplay-first, browser-rendered video compiler and execution engine. It compiles authored intent and a frozen asset catalog into deterministic timeline and render graphs, then executes the resulting plan locally or across stateless workers.

## Architectural axioms

1. Source expresses intent; IR records facts.
2. The compiler is a pure function of source, frozen asset catalog, options, and version.
3. Local and distributed rendering execute the same `ExecutionPlan` and worker state machine.
4. Partitions follow pixel and temporal dependencies, not blindly chosen shot boundaries.
5. Chromium draws resolved frames; it does not solve time or discover work.
6. Every expensive artifact has an explicit deterministic identity; immutable
   byte artifacts also retain content hashes.

## TypeScript and Rust boundary

TypeScript owns the browser world: authoring types, DOM/CSS/Canvas/WebGL components, the deterministic browser clock, animation adapters, and bundler integration. Rust owns the system world: parsing, binding, timing, typed IR, render graphs, partitioning, cache keys, scheduling, subprocess lifecycles, media planning, workers, and CLI.

Rust does not reimplement browser layout. TypeScript does not duplicate timing or planning logic. Optional WASM or N-API bindings wrap the Rust compiler rather than create a second compiler.

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
- **Structurally Linked Film** contains known elements, legal ownership, and valid film-wide IDs while retaining unresolved authored attributes privately.
- **Resolved Film** contains typed durations, cues, asset references, and content start rules without exposing syntax-layer attributes.
- **Timeline IR** contains exact frame intervals and provenance for every timing decision.
- **Render Graph** records pixel, media, transition, history, global-layer, and audio dependencies.
- **Execution Plan** contains immutable work units, environments, dependencies, output intervals, evaluation intervals, and cache keys.

A render unit has separate `evaluation` and `output` intervals. Evaluation may include transition preroll, animation warm-up, or history frames; only output frames are published.

The compiler pipeline ends at Timeline IR. Execution begins at a separate
composition boundary:

```text
Timeline IR + Frozen Asset Catalog + Bundle Manifest + Render Profile
  → Render Unit
    → Browser Plan + Audio Plan + materialization requirements
      → materialize → Executable Unit + verified private root
```

This join is intentionally not another compiler phase. Timeline IR says what
the film is and when each fact holds; a presentation bundle owns how those
facts become DOM, CSS, Canvas, or WebGL; a Render Unit says which immutable
inputs one executor invocation consumes. `RenderProfile` owns pixel-affecting
facts such as viewport dimensions; process deadlines and retained-memory
ceilings remain executor limits. Materialization consumes the unit into an
`ExecutableUnit`, so the executor cannot pair a browser plan with an unrelated
URL or asset root. Gate one has exactly one whole-film unit. Gate two introduces
the Render Graph and may derive several units of the same type; it does not
replace the executor contract.

Gate-one `AudioPlan` contains only solved voice-over placements. Materialization
copies their frozen bytes beside browser assets without making them browser
inputs. After Chromium has encoded the visual stream, the executor resets each
audio input to its origin, applies its exact rational frame delay, mixes the
tracks, and muxes AAC into the final MP4. Gain, fades, resampling policy, and
general audio effects remain deferred rather than being implied by this first
mixing contract. One Gate-one unit retains at most 512 audio tracks, keeping
the `FFmpeg` argument and filter-graph boundary explicitly bounded.

The first Gate-two local assembly keeps its independently materialized units
alive while their contiguous output frames enter one continuous visual encoder
in screenplay order. Voice-over placements retain absolute Timeline starts
until final assembly rebases them once to the assembled artifact's output
origin and mixes them after every unit has captured its output. This avoids
treating separately AAC-muxed segments as safely concatenable, and also avoids
a second lossy visual encode. It is deliberately a correctness path, not yet a
persistent segment-cache format: cached encoded segments require a separate
equivalence proof before they become an execution artifact.

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
timeline solve without IO. Validation belongs to the phase that first has
enough information to decide; solve constructs Timeline IR directly. Onmark
does not add ceremonial `validate` or `lower` phases without a representation
that proves a new invariant. At Gate two, the planner computes dependency
closure before partitioning. Its first graph records each Gate-one shot as an
independent region only because the production adapter has proved that it keeps
no state across shot boundaries; the graph also records each region's direct
frozen-media dependencies. This is evidence-backed, not a general
shot-boundary rule. Transitions, persistent elements, global effects, and
historical shaders must widen or merge regions for correctness before
partitioning consumes them.

Structural binding and attribute/reference resolution aggregate authored diagnostics while building candidate outputs. An error withholds the phase value from its report so rejected structure or recovery defaults cannot enter the next phase as compiler facts; warnings remain non-blocking.

Input freezing separates three identities that must never be conflated:

- `AssetRef` is the screenplay-relative portable path authored in the screenplay;
- `FrozenAssetId` identifies the immutable bytes that were probed and compiled;
- a materialized asset is a worker-local path or browser URL for those exact
  bytes.

Timeline solving consumes a catalog from `AssetRef` to `FrozenAssetId` plus
normalized `AssetMetadata`, all owned by `onmark-core`. Metadata records exact
artifact duration and, for each selected audio or visual stream, its exact
stream duration. Visual metadata also records codec, pixel format, and either
one exact rational frame rate or variable timing. Single-frame
streams are represented separately because an exact reported frame count of one
cannot establish a source rate.
`onmark-media` produces metadata through probing, while a loader or composition
root hashes and freezes the same bytes.
ffprobe-specific structures, source paths, and browser URLs remain outside
core. Timeline IR records `FrozenAssetId`, never the authored spelling or a
mutable path. A missing catalog entry is a typed integration failure rather
than an authored diagnostic. A media element with no authored source remains
valid through static resolution but cannot produce renderable Timeline IR and
receives an authored asset diagnostic during solving.

Gate-one `FrozenAssetId` uses SHA-256 and the canonical
`sha256:<lowercase-hex>` spelling. The hashing operation belongs at the IO
freezing boundary; core owns only the validated identity and deterministic
mapping.

The bundle manifest has the same separation. Its target contract identifies an
immutable presentation artifact and its entry point, runtime version, fonts,
static dependencies, and declared temporal capabilities. Gate one's current
manifest records only the fixed entry document and the exact retained files;
their hashes already bind the injected runtime and compiled CSS. Additional
fields appear only when authoring or execution consumes them. The manifest does
not contain timing rules. Its `bundleId` is SHA-256 over the UTF-8 compact JSON
identity `{version,entryPoint,files}`; files are sorted by portable path and
each identity entry is ordered as `{bytes,path,sha256}`. This encoding is a
versioned contract, not an incidental pretty-printed manifest representation.
V1 contains one to 99,999 payload files. Paths are lowercase portable ASCII, at most
1,024 bytes, cannot enter unit-owned namespaces, and cannot make one file the
directory ancestor of another.
Materialization turns frozen bundle and asset identities into local paths or
browser URLs immediately before execution and verifies their digests.
Gate one assembles one content-addressed unit root: required assets appear at
`assets/sha256/<lowercase digest>` beneath the presentation entry. The browser
derives that relative location from the frozen identity already present in
`BrowserPlan`; native paths and browser URLs therefore need no second wire
protocol. The unit retains worker-local source paths only until assembly has
verified or linked the exact bytes into that root.
The presentation entry owns layout and installs the runtime adapter; the
runtime supplies deterministic clock, readiness, and media primitives. Onmark
does not synthesize an implicit full-screen DOM from Timeline IR. The public
rules for author-owned browser code live in the
[presentation contract](presentation-contract.md).

Each Gate-three capture worker executes one state machine:

```text
materialize → launch → ready → seek/capture → fingerprint → verify → commit
```

The capture worker never owns a visual encoder: it atomically publishes one
verified frame artifact. A later coordinator owns claims and leases; the
assembler owns the one continuous visual encode. Audio follows a separate plan
and is mixed outside browser frame capture.

## Deterministic browser protocol

The sole clock derives from frame index and rational timebase. Wall time and free-running animation or media clocks may not determine output.

The runtime protocol includes `Load`, `Prepare`, `Seek`, `FrameReady`, and `Dispose`. `FrameReady(frame)` is a logical stability barrier: after receiving it, the native executor waits two bounded animation-frame turns for Chromium to commit the selected state to its compositor, then captures the frame. Direct rendering retains that PNG as its encoder payload. Worker capture additionally decodes it into the configured exact 8-bit RGBA viewport and hashes those canonical pixel bytes. A worker artifact records that raw-pixel hash beside each ordered PNG record, so independent captures can be compared without treating PNG compression bytes as visual truth. The commit barrier never selects a frame or contributes a clock; it only closes the logical-runtime-to-native-capture race. The runtime does not publish an independently invented state hash. Inside the runtime, `RuntimeFrame` retains the exact integral frame identity and derives floating-point seconds from the Rust-owned rational frame rate only for browser API calls; those seconds never become scheduling or protocol truth. Components declare temporal behavior such as `stateless`, `warmup(n)`, `sequential`, `global`, or `neighbor(radius)`. Unknown components default to `sequential`: parallel seekability must be proven, not guessed. Native APIs and audited adapters may safely provide stronger declarations. Detection may recommend an adapter but may not silently relax correctness.

Determinism is layered. Canonically encoded Timeline IR and Execution Plans
must be byte-identical once those encodings exist. The current in-memory
Timeline IR is structurally deterministic but does not yet claim canonical wire
bytes. Raw frames target identical hashes inside a locked browser/font/render
environment; per-frame worker-artifact fingerprints make that property an
executable conformance claim. Encoded containers are validated by timestamps,
frame counts, codec configuration, and decoded content; byte-identical MP4
output is an experimental property, not a blanket promise.

## Distributed execution (production target)

The coordinator stores DAG state, leases, retries, and artifact references but never proxies frames. Stateless workers exchange immutable bundles, assets, and artifacts directly with object storage. Delivery is at least once; deterministic contract identity plus compare-and-verify publication makes it effectively once.

Gate three starts with a deliberately narrower interchange: a worker captures a
whole planned output interval into one bounded, checksummed frame artifact. The
artifact is a single versioned file containing an exact output interval,
render-profile, visual-plan, and locked capture-environment identities, plus an
ordered PNG stream with canonical raw-RGBA fingerprints. It is published
through a sibling staging file and no-clobber link, so a retry can verify or
reuse an existing immutable result without ever exposing a partial one. The
assembler verifies that each artifact belongs to its planned unit and capture
environment, then streams verified frames through the same one continuous visual
encoder used by Gate two and materializes and mixes all audio once at the
assembled output origin.

This is intentionally not a remote-frame queue: a worker owns a contiguous
unit and publishes one artifact only after its browser session has completed.
Nor is it an encoded-segment cache. Independently AAC-muxed MP4 segments are
not assumed concatenable, and independently encoded visual segments need a
separate equivalence proof before they can replace the lossless frame
interchange. Long, expensive, randomly seekable scenes may later be divided
into contiguous frame ranges once their render dependencies prove it safe.
Individual frames are never remote tasks. CPU, memory, GPU, browser slots,
encoder slots, codecs, disk, and network are explicit scheduling resources; all
internal queues are bounded.

The first implementation used the local filesystem only to prove the process
and artifact contract. After that conformance passed, `deploy/aws-lambda`
became the first narrow cloud adapter. Its Rust binary owns only versioned
Lambda invocation/result JSON, bounded S3 materialization, and conditional
publication of the existing `onmark-render` frame artifact. It uses no generic
object-store trait because S3's multipart `If-None-Match: *` completion,
precondition reuse verification against this capture's raw-RGBA sequence, and
bounded conflict retry are the actual semantics it needs. AWS types stop at
that deployment package; they never enter core or render.

The adapter has no coordinator, queue, lease database, global retry owner,
capability scheduler, infrastructure definition, or production Lambda image.
Its input chooses only an immutable worker-input S3 prefix. The deployed image
owns the output namespace, browser binary, locked capture-environment identity,
and limits. Its handler explicitly selects `ChromiumSandbox::Disabled` as a
candidate for Lambda's outer isolation boundary; that choice is neither an
automatic launch fallback nor invocation-controlled. A real Lambda experiment
must establish the corresponding process-isolation and launch contract before
an image or infrastructure template is added.

## Incremental rendering

Changing a base layer permits upper-layer reuse only when the render graph proves there is no sampling or composition dependency. Blend modes, backdrop filters, transitions, and shaders expand invalidation. Layered alpha intermediates can improve reuse at an encoding, color, and composition cost. Correctness outranks cache granularity; Onmark does not promise that every shot is always independent.

## Target repository shape

Concepts start as modules, not crates. A package is split only for a distinct runtime, dependency budget, real independent consumer, or deployment/release artifact. Size and hypothetical reuse are not sufficient.

```text
onmark/
├── AGENTS.md  CLAUDE.md  README.md
├── crates/
│   ├── core/       # pure compiler, domain model, diagnostics, IR
│   ├── media/      # asset probing without Chromium
│   ├── render/     # browser, FFmpeg encoding, local executor
│   └── cli/        # native tool face and composition root
├── packages/
│   ├── runtime/ authoring/ bundler/
├── scripts/     # repository-only generation and quality checks
├── deploy/aws-lambda/  # Rust Lambda/S3 adapter after artifact conformance
├── schemas/ conformance/ evals/ examples/ docs/
```

The completed Gate-one milestone contains `onmark-core`, `onmark-media`, `onmark-render`, `@onmark/runtime`'s browser session, `@onmark/authoring`'s semantic DOM bindings, `@onmark/bundler`'s presentation artifact boundary, and the first `onmark-cli` whole-film composition root. Media is a separate crate because server-side compile/lint loops need probing without Chromium; this is both a distinct dependency budget and a real consumer. Runtime is a separate package because it executes inside the browser and is consumed by authoring and bundling. Authoring is a separate browser package because user presentations consume its public DOM contract independently while runtime must not depend upward on author-facing effects; its only product dependency is runtime's types-only public surface. Bundler is a separate package because it executes under Node, owns the esbuild and filesystem dependency budget, and produces a presentation directory consumed independently by native rendering. The CLI is a distinct release artifact that combines core compilation, media probing, the bundler process, and native rendering without moving their implementation details into one crate. `onmark-aws-lambda` is a distinct Rust release artifact because Lambda is a different runtime and its handler owns the `aws-config`, `aws-sdk-s3`, and `lambda_runtime` dependency budget. It may consume `onmark-render`'s portable worker request and `onmark-core`'s canonical bundle layout, but neither dependency knows about AWS. Syntax, diagnostics, model, compiler, timeline, and protocol remain rectangular modules inside core. Render graph and planning initially join core at gate two. Worker execution belongs to render. A coordinator may appear only after the remote artifact adapter has passed conformance.

Gate one's native command is deliberately narrow: `onmark render <screenplay>`. It discovers `presentation.ts` beside the screenplay unless `--presentation` is supplied, derives the stable no-clobber output `renders/<screenplay-stem>.mp4` unless `--output` is supplied, and exposes only exact frame rate and viewport dimensions as ordinary render controls. Process paths are execution overrides, not screenplay facts. Authored diagnostics are emitted before executable preflight so an invalid screenplay never requires Chromium, Node, or `FFmpeg` merely to explain itself. Gate three adds the deliberately separate worker entry point `onmark worker capture`: it accepts one versioned `request.json`, including the deployment-owned SHA-256 identity of its locked capture environment, the `bundle/` payload files named by that manifest, and frozen `assets/sha256/` bytes. The identity covers the image's Chromium, fonts, launch configuration, and other pixel-affecting host facts; the renderer deliberately does not guess it from one executable path or browser-version string. The worker materializes inputs in a private root and publishes one frame artifact. Reuse and assembly require that environment identity alongside the unit identity. The command accepts no screenplay and never recompiles source; a coordinator or object-store adapter owns request publication later.

`onmark-cli` resolves every external executable before starting process work, then follows one linear path: read and compile, freeze and probe referenced assets, solve Timeline IR, bundle the presentation, compose and materialize the whole-film unit, render, and atomically publish. Freezing streams each referenced source into a private temporary file while hashing it, then probes only that private copy; identity and metadata therefore describe the same retained bytes. Hashing and probing run on explicit blocking work rather than a Tokio worker. The CLI depends on core, media, and render as its real composition inputs; `clap` owns argument parsing, `sha2` owns streaming SHA-256, `tempfile` owns private lifetimes, `serde_json` decodes the Rust-owned manifest, and Tokio owns bounded process and render async work. No CLI dependency enters the pure core.

`evals/` is checked-in language-product evidence, not a runtime package or a live-model CI service. It owns frozen cases, prompts, grader rules, raw outputs, model settings, and comparison baselines. Those assets are added only from real experiments; the repository does not create an empty framework or invent a historical baseline when the source material is unavailable.

Core's internal dependency DAG is CI-enforced: `model` depends on nothing; `syntax`, `diagnostics`, and `timeline` may depend on model; `render_graph` may depend on model and timeline; `compiler` may depend on model, syntax, diagnostics, and timeline; `protocol` may depend on model, diagnostics, and timeline. No domain module depends back on protocol. New edges require an architecture change. CI performs a syntax-aware check of explicit Rust paths with `syn`. This cooperative guard covers ordinary paths, imports, aliases, and re-exports, but not paths generated inside macros or full rustc name resolution; review remains responsible for those edges.

`onmark-core` uses `xmlparser` only inside `syntax` for pure, span-preserving XML-compatible fragment tokenization. Onmark owns tree construction, nesting checks, duplicate-attribute checks, reference decoding, and all authored semantics. Parser errors are translated at the syntax boundary and the dependency performs no IO. Test targets may use `proptest` for time algebra and `syn` for the cooperative module dependency-law check; neither development dependency is linked into library consumers or runtime artifacts.

The `protocol` module uses `serde` for stable browser and bundle-manifest JSON boundaries. Its optional `schema` feature exposes `schemars` only to repository generation; product binaries do not enable it. All repository-only automation lives under `scripts/`; it is not a product package or a miscellaneous application layer. Its Cargo manifest exists solely to give the Rust schema generator a pinned build-only dependency budget and a stable `cargo xtask` entry point. That binary is consumed only by developers and CI and may depend on core and `onmark-aws-lambda` with their `schema` features, `schemars`, and `serde_json`; it disables the Lambda package's default runtime feature, so schema generation does not link AWS. Product crates and packages never depend on it. The Lambda dependency exists solely to publish that deployment boundary's schemas, not to smuggle AWS into core. The adjacent Node generator may use the pinned schema-to-TypeScript and validation toolchain. `cargo xtask schema` writes every versioned schema, then invokes that generator. `json-schema-to-typescript` emits reviewable browser types into runtime and the manifest type into bundler; Ajv emits standalone browser validation code at build time. The Lambda schemas intentionally have no TypeScript codec until a real TypeScript caller exists. Oxlint, a narrow repository-shape check, and Prettier are repository-only development gates and never enter the browser artifact. The browser runtime does not compile schemas dynamically. Exact tool versions are pinned in the lockfiles and `mise.toml`, and CI rejects stale generated artifacts.

`onmark-media` depends on core plus `serde` and `serde_json` for a private ffprobe response boundary. It starts the configured ffprobe executable directly with an argument array, never through a shell; wrappers that leave descendant processes holding the output pipes are outside this executable contract. Process lifetime and retained stdout/stderr bytes are explicitly bounded under that direct-child contract, both pipes are drained concurrently, and explicit shutdown reports process-control failures while `Drop` remains a best-effort termination fallback. Private ffprobe response types are translated once into core-owned `AssetMetadata`; JSON values and third-party error types do not define the stable API, while underlying errors remain available through the standard error source chain for debugging. Gate-one probing requests bounded stream-level facts for every stream: `codec_type` records audio presence and selects the first visual stream, while `nb_frames` identifies stills. It classifies a visual stream as constant only when ffprobe's parseable `avg_frame_rate` and `r_frame_rate` normalize to the same exact rational rate; disagreement or unavailable values are conservatively variable. The one-MiB stdout ceiling therefore remains independent of media duration.

`onmark-render` owns the heavy Chromium and `FFmpeg` dependency budget. It uses
`chromiumoxide` only as a CDP transport and process launcher, `png` only to
decode a captured screenshot into canonical RGBA for its renderer-owned
fingerprint, and `tokio` and `futures` only within this asynchronous execution
boundary. `tempfile` gives
every browser session an isolated profile, owns a private same-filesystem output
staging directory, and retains one private RAII unit root.

Unit-root materialization uses `serde_json` only for the Rust-owned manifest
encoding, `sha2` for streaming identity verification, and `url` for the browser
entry URL. File bounds are rejected before identity work; canonical hashing and
manifest sizing stream through fixed-memory writers, and the pretty manifest is
serialized directly into the private root. It rejects symlinks and non-files,
copies verified bytes instead of linking mutable source paths, and bounds both
retained file count and total bytes. The fixed safety envelope is 100,000 files
and one tebibyte, while each caller supplies a smaller explicit policy.
Parallel sessions therefore share neither Chrome locks nor admitted input
paths, and a completed MP4 is published with a no-clobber hard link only after
both processes finish cleanly.

The crate supplies executable paths, viewport, browser process and request
deadlines, an encoder inactivity timeout, frame/input/capture-byte ceilings,
bounded retained stderr, and explicit shutdown. Chromium, CDP, and subprocess
types are translated into stable render-owned errors. Encoder lifetime remains
bounded by finite frame and byte budgets plus timeouts on every write and
finalization operation; time spent awaiting Chromium does not consume an
encoder inactivity budget. Browser navigation waits separately for document
load and the runtime host because the transport's navigation call does not
itself establish that lifecycle barrier.

Gate one captures one PNG at a time and writes it directly to `FFmpeg`'s
`image2pipe`; there is no frame queue or whole-video buffer. The fixed H.264
`yuv420p` profile rejects odd viewport dimensions before either process starts.
Conformance launches Chrome for Testing and `FFmpeg` against the production
presentation adapter, crosses the typed `Load`/`Prepare`/`Seek` handshake,
probes the resulting H.264 MP4, and verifies decoded motion. The checked-in bundle fixture
carries real payload bytes, is rebuilt byte-for-byte in the bundler test, and
crosses the generated Node/native manifest contract through native
materialization. The outermost CLI conformance starts two independent
whole-film sessions and compares their complete decoded raw-frame hash
sequences before checking no-clobber publication. CI owns explicit browser and
media-tool versions for these tests; local execution remains opt-in because it
requires those external processes.

GitHub-hosted Ubuntu does not expose a usable Chromium sandbox to the installed
Chrome for Testing binary. The real-process job therefore supplies a
runner-local launcher that adds `--no-sandbox`; this exception belongs only to
the disposable CI worker. Product and local browser launches keep Chromium's
sandbox enabled by default. An adapter may explicitly disable Chromium's
internal sandbox only when an independently audited outer container or microVM
owns equivalent process isolation. That deployment-owned choice is part of the
locked capture environment, is never selected by authored input or a worker
invocation, and must be proven in its real execution environment before it is
treated as a production launch contract. A failed Chromium launch never causes
an automatic downgrade.

Gate-one native browser operations and decoded-video waits accept at most a one-day deadline, keeping every platform timer inside an explicit supported horizon.

Validation reasons remain local domain values. Once syntax has supplied source context, the `compiler` module is the single owner that translates reasons such as `InvalidNodeId` into source-located `Diagnostic` values, including phase-specific messages and help. `diagnostics` owns only the generic diagnostic representation and stable codes. Neither `model` nor `syntax` depends on diagnostics, and the translation must not be duplicated by callers.

On the TypeScript side, runtime is the foundation. Authoring consumes runtime's types-only public hook and adapter contract, creates semantic video and overlay DOM, and leaves CSS and layout with the presentation entry. Bundler injects the pinned authoring and runtime artifacts. Runtime never depends on authoring or bundler. `stateless`, `warmup`, and `sequential` are architectural categories today, not a public capability-declaration API; when that extension point becomes real, runtime will own it. The Gate-one `RuntimeSession` owns protocol ordering, interval-relationship checks, exact-frame projection, and terminal disposal. It rejects concurrent commands instead of growing a hidden queue and gives the adapter a recursively frozen snapshot of accepted plan facts. Browser-specific work enters through one narrow adapter whose waits must be bounded and whose expected failures are typed. The production presentation adapter receives presentation-owned elements, sources, and visibility effects. It owns bounded media loading, exact source-frame selection, decoded-frame readiness, solved overlay visibility, and terminal cleanup without creating layout or canvas state. The materialized asset directory used by that adapter and by the bundler is generated from the Rust bundle schema.

`@onmark/bundler` is the Node-only product build boundary, not repository automation. It may depend on Node built-ins, the public `@onmark/authoring` and `@onmark/runtime` entry points, and the pinned `esbuild` production dependency; browser packages never depend back on it. Gate one compiles one ESM presentation, substitutes the pinned authoring and runtime entries, emits a fixed document shell, and records every presentation payload file in a stable SHA-256 manifest. The package exposes the same operation through the narrow `onmark-bundle` executable so the native CLI does not import Node or esbuild types. That child process receives explicit entry, output, and retained-byte-limit arguments, writes no success payload to stdout, and reports a stable failure category on stderr. The native caller applies its own process deadline, drains diagnostics continuously while retaining only a bounded tail, and parses the resulting manifest back through the Rust-owned wire type. The manifest shape and layout constants are generated from the Rust protocol contract rather than handwritten again in TypeScript. The build has an explicit retained-output byte ceiling, writes through a private sibling staging directory, and refuses an output path observed to exist before compilation or publication. The final directory rename prevents readers from observing a normally completed partial build, but portable Node filesystem APIs do not make the preceding absence check a cross-process no-clobber transaction. The current boundary deliberately has no watch mode, plugin API, cache, development server, or asset materialization policy. Esbuild's internal working memory remains governed by the pinned third-party implementation rather than the retained-output ceiling.

Rust wire types are the source of truth. `cargo xtask schema` generates checked-in versioned JSON Schema, and CI requires regeneration to produce no diff. A schema with a real TypeScript consumer also generates checked-in types/codecs; the browser and bundle contracts do so today, while the Lambda invocation deliberately has no speculative TypeScript caller. Generated files are never hand-edited, and Rust does not regenerate a second Rust model from its own schema. Before the first external Gate-one release, v1 is refined in place so the initial public contract does not preserve experimental fields; after publication, an incompatible wire change requires a new protocol version and migration fixture. The `BrowserPlan` carries the output frame rate, evaluation/output intervals, primary-video placements, and title/call-to-action overlays consumed by the production presentation adapter. Video placements identify immutable bytes, an absolute visible interval, and the admitted CFR source rate needed to verify decoded-frame selection; overlay placements carry only their semantic role, decoded text, and compiler-solved absolute interval. Materialized URLs remain render-owned facts, while DOM structure and CSS remain presentation-owned effects. This is the browser-facing projection of one Render Unit, not the Render Graph or partition plan itself. It may contain only facts consumed in the browser; output paths, cache keys, `FFmpeg` arguments, source spans, and materialization policy remain outside it. VFR timestamp maps and further component facts are added only when the production adapter consumes them.

Protocol V1 carries at most 10,000 video placements and 10,000 overlay placements; one overlay inscription carries at most 65,536 Unicode characters. One failure carries at most 4,096 message characters and 256 pending-resource descriptions of at most 1,024 characters each; the producer owns their deterministic order. The runtime-host property name and these resource limits are generated from Rust-owned schema metadata, so native execution, browser runtime, and validation do not maintain handwritten copies.

AWS Lambda is an adapter, not another engine. The checked-in Rust
`onmark-aws-lambda` binary owns V1 invocation/result schemas, the thin handler,
and S3 operations. It downloads one portable worker layout, checks that the
request's capture environment equals the deployed image identity, materializes
the Render Unit through `onmark-render`, verifies the finished artifact, and
conditionally publishes it by its renderer-owned identity. A `412` means
"download, verify, and compare the already-published raw-RGBA artifact"; a
bounded `409` retry is only a conditional-publication transport retry, not a
distributed retry policy.

The deployment config owns S3 transport budgets: a five-second connection
timeout, a 45-second attempt timeout, a 90-second operation timeout, and at
most three SDK attempts. Since `GetObject` returns a response stream after the
SDK operation has completed, every pending body read separately has a
30-second progress deadline. This prevents a stalled stream from becoming an
unbounded worker wait without pretending that it is a scheduler or lease
policy.

This JSON contract has checked-in Rust-generated schemas. It intentionally has
no generated TypeScript SDK because no TypeScript caller exists yet; creating a
coordinator client merely to satisfy symmetry would prebuild a later Gate-three
control plane. AWS SDK types may not enter core or render. Container-image and
infrastructure definitions remain experimental until Lambda proves the outer
isolation and Chromium launch contract. Other backends such as ECS or Kubernetes
follow the same adapter rule and consume the same worker request and artifact
format.

## Delivery gates

**Gate one (complete): render one real video reliably.** The completed milestone includes the minimal language, frozen asset catalog, media probing, Rust timing, versioned Timeline IR, immutable presentation bundle, deterministic browser clock, frame handshake, and a single-process whole-film Render Unit through Chromium/FFmpeg. It executes and muxes authored voice-over rather than silently dropping it. Its exit conformance builds the release CLI, renders the same screenplay through two independent Chromium/FFmpeg sessions, compares decoded video and audio frame hashes, verifies motion and stream facts, and proves no-clobber publication.

**Gate two (complete): partition and assemble correctly.** The completed slice renders two independent local units and assembles them through the existing executor. Its exit conformance renders one media-bearing two-shot film both as a whole-film unit and as two independently materialized units, then compares decoded video and audio hashes plus first-audio-packet placement. It introduces the Render Graph and evaluation/output intervals. Preroll, persistent unit caching, and dependency-based invalidation remain deferred until a real dependency or cache consumer requires them.

**Gate three: leave the machine.** First prove that independent worker processes
publish and assemble bounded, verified frame artifacts through the existing
encoder and audio path. Then add object storage, leases, retries, idempotent
publication, and capability scheduling around that proven interchange. Validate
byte-identical plans, raw-frame hashes in a locked environment, and decoded
media equivalence; do not presume byte-identical MP4 containers.

Every gate uses the final-direction contracts but implements only fields consumed by that gate. A failed gate blocks construction of the next gate's skeleton.

## Open experimental questions

CDP versus WebDriver BiDi, capture mechanism, layered alpha caching, wire encoding, coordinator storage, adapter seekability, and environment-locking granularity require prototypes and measurements. The pure compiler boundary, deterministic protocol, dependency-driven partitioning, and local/distributed symmetry are foundational decisions.

The first Gate-one capture spike gives positive but deliberately narrow evidence for application-controlled `FrameReady` followed by CDP `Page.captureScreenshot`: repeated DOM, CSS, and Canvas frames produced identical raw RGBA hashes across independent Chrome processes on one locked machine. This selects the next experiment, not the final transport contract; decoded media, WebGL, asynchronous components, cross-environment equality, and production lifecycle remain unproven.

The decoded-media experiment covers 30 fps CFR, `30000/1001` CFR, and an
alternating-frame-interval VFR H.264 fixture, each with a 30-frame GOP and three
B-frames. Native `<video>` seeking across the non-monotonic sequence
`17 → 3 → 29 → 17` produced byte-identical PNG captures in two independent
Chromium sessions once `requestVideoFrameCallback.mediaTime` confirmed the
source frame selected at each output-frame midpoint. VFR expectations come
from the probed source-frame timestamps rather than assuming source and output
frames align. Independent `FFmpeg` extraction at the selected source-frame
timestamps was also byte-stable across repeated runs. Seeking to an exact CFR
frame-boundary second selected the preceding frame; sampling inside the
Rust-selected frame produced the intended decoded frame.

The two decode paths are not pixel-interchangeable. Across four 320×180 RGBA
frames, Chromium canvas output differed from `FFmpeg` raw extraction in roughly
229,000–232,000 of 921,600 channels, with mean absolute channel error
2.13–2.18 and isolated maxima 173–178 on the measured machine. Browser
seek/readiness/screenshot averaged 51–81 ms per frame; process-per-frame native
extraction averaged 18–19 ms but excluded browser injection, composition, and
final capture, so the figures are not an end-to-end speed comparison. Gate one
therefore keeps one decode/color path authoritative for a render and treats it
as part of the locked environment. Codec and color diversity, longer random
sequences, persistent native-decoder cost, and injection overhead remain open
measurements.

Gate one therefore admits CFR H.264 visual assets only and uses the locked
Chromium decoder as the authoritative visual decode/color path. The adapter
seeks inside the Rust-selected frame and does not report readiness until
`requestVideoFrameCallback.mediaTime` identifies the expected source frame.
Unsupported codec or variable-frame-rate input is rejected before rendering,
not silently approximated. VFR becomes admissible only after frozen metadata
and the browser plan carry a complete timestamp map rather than one CFR rate.
`FFmpeg` exact-frame extraction remains an alternative experiment rather than
a hidden fallback that would change pixels within one render.

This policy is represented by render-owned `AdmittedVideo` proof over
core-owned metadata. It borrows the normalized facts instead of introducing a
second media model, and proves both H.264 codec support and one exact source
frame rate. The whole-film Render Unit retains that rate and lowers it into the
browser placement exactly once. The decoded-media conformance obtains the proof from the
production bounded ffprobe boundary for both accepted CFR fixtures and the
rejected VFR fixture. The whole-film executor consumes admitted video through
the production adapter and verifies the completed moving-picture artifact.
