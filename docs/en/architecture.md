# Onmark Architecture

> Status: target architecture. This document separates foundational rules, ordered delivery gates, and later production capabilities.

This document is paired with the Onmark Language Specification. The language defines authored meaning; this document defines execution. Their only contract is the versioned Timeline IR.

Onmark is a screenplay-first, browser-rendered video compiler and execution engine. It compiles authored intent and frozen asset metadata into deterministic timeline and render graphs, then executes the resulting plan locally or across stateless workers.

## Architectural axioms

1. Source expresses intent; IR records facts.
2. The compiler is a pure function of source, asset metadata, options, and version.
3. Local and distributed rendering execute the same `ExecutionPlan` and worker state machine.
4. Partitions follow pixel and temporal dependencies, not blindly chosen shot boundaries.
5. Chromium draws resolved frames; it does not solve time or discover work.
6. Every expensive artifact is content-addressed.

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

## End-to-end pipeline

```text
freeze inputs → probe media → compile → bundle → render graph
  → partition → capture/encode → mix audio → verify/assemble
```

The compiler performs parse, structural bind, attribute/reference resolve, validate, solve, and lower without IO. The planner computes dependency closure before partitioning. Simple films naturally partition around shots; transitions, persistent elements, global effects, and historical shaders widen or merge units for correctness.

Structural binding and attribute/reference resolution aggregate authored diagnostics while building candidate outputs. An error withholds the phase value from its report so rejected structure or recovery defaults cannot enter the next phase as compiler facts; warnings remain non-blocking.

Timeline solving consumes normalized `AssetMetadata` owned by `onmark-core`; Gate one initially requires exact artifact duration. `onmark-media` produces these facts through probing, while ffprobe-specific structures and failures remain outside core. A referenced asset missing from the supplied metadata map is a typed integration failure rather than an authored diagnostic. A media element with no authored frozen artifact remains valid through static resolution but cannot produce renderable Timeline IR and receives an authored asset diagnostic during solving.

Each worker executes one state machine:

```text
claim → materialize → launch → ready → seek/capture → encode → verify → commit
```

Artifacts are verified and atomically published under content hashes. Audio follows a separate plan and is mixed outside browser frame capture.

## Deterministic browser protocol

The sole clock derives from frame index and rational timebase. Wall time and free-running animation or media clocks may not determine output.

The runtime protocol includes `Load`, `Prepare`, `Seek`, `FrameReady`, and `Dispose`. `FrameReady(frame)` is only a stability barrier: after receiving it, the native executor captures the frame and hashes the exact raw RGBA bytes it consumed. The runtime does not publish an independently invented state hash. Inside the runtime, `RuntimeFrame` retains the exact integral frame identity and derives floating-point seconds from the Rust-owned rational frame rate only for browser API calls; those seconds never become scheduling or protocol truth. Components declare temporal behavior such as `stateless`, `warmup(n)`, `sequential`, `global`, or `neighbor(radius)`. Unknown components default to `sequential`: parallel seekability must be proven, not guessed. Native APIs and audited adapters may safely provide stronger declarations. Detection may recommend an adapter but may not silently relax correctness.

Determinism is layered. Timeline IR and Execution Plans must be byte-identical. Raw frames target identical hashes inside a locked browser/font/render environment. Encoded containers are validated by timestamps, frame counts, codec configuration, and decoded content; byte-identical MP4 output is an experimental property, not a blanket promise.

## Distributed execution (production target)

The coordinator stores DAG state, leases, retries, and artifact references but never proxies frames. Stateless workers exchange immutable bundles, assets, and artifacts directly with object storage. Delivery is at least once; content-addressed compare-and-commit makes publication effectively once.

Continuous encoded segments are the default unit. Long, expensive, randomly seekable scenes may be divided into frame ranges. Individual frames are not remote tasks. CPU, memory, GPU, browser slots, encoder slots, codecs, disk, and network are explicit scheduling resources; all internal queues are bounded.

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
├── deploy/aws-lambda/  # introduced at delivery gate three
├── schemas/ conformance/ evals/ examples/ docs/
```

The current milestone contains `onmark-core`, `onmark-media`, and `@onmark/runtime`'s generated protocol boundary and sequential session state machine. Gate one adds `onmark-render` and `onmark-cli` when their first consumed behavior is implemented. Media is a separate crate because server-side compile/lint loops need probing without Chromium; this is both a distinct dependency budget and a real consumer. Runtime is a separate package because it executes inside the browser and is consumed by authoring and bundling. Render depends on core and media. Syntax, diagnostics, model, compiler, timeline, and protocol remain rectangular modules inside core. Render graph and planning initially join core at gate two. Worker execution belongs to render. A coordinator appears only at gate three.

`evals/` is checked-in language-product evidence, not a runtime package or a live-model CI service. It owns frozen cases, prompts, grader rules, raw outputs, model settings, and comparison baselines. Those assets are added only from real experiments; the repository does not create an empty framework or invent a historical baseline when the source material is unavailable.

Core's internal dependency DAG is CI-enforced: `model` depends on nothing; `syntax`, `diagnostics`, and `timeline` may depend on model; `compiler` may depend on those four; `protocol` may depend on model, diagnostics, and timeline. No domain module depends back on protocol. New edges require an architecture change. CI performs a syntax-aware check of explicit Rust paths with `syn`. This cooperative guard covers ordinary paths, imports, aliases, and re-exports, but not paths generated inside macros or full rustc name resolution; review remains responsible for those edges.

`onmark-core` uses `xmlparser` only inside `syntax` for pure, span-preserving XML-compatible fragment tokenization. Onmark owns tree construction, nesting checks, duplicate-attribute checks, reference decoding, and all authored semantics. Parser errors are translated at the syntax boundary and the dependency performs no IO. Test targets may use `proptest` for time algebra and `syn` for the cooperative module dependency-law check; neither development dependency is linked into library consumers or runtime artifacts.

The `protocol` module uses `serde` for the stable JSON boundary. Its optional `schema` feature exposes `schemars` only to repository generation; product binaries do not enable it. All repository-only automation lives under `scripts/`; it is not a product package or a miscellaneous application layer. Its Cargo manifest exists solely to give the Rust schema generator a pinned build-only dependency budget and a stable `cargo xtask` entry point. That binary is consumed only by developers and CI and may depend on core with the `schema` feature, `schemars`, and `serde_json`; product crates and packages never depend on it. The adjacent Node generator may use the pinned schema-to-TypeScript and validation toolchain. `cargo xtask schema` writes the versioned schemas, then invokes that generator. `json-schema-to-typescript` emits reviewable types, Ajv emits standalone validation code at build time, and TypeScript checks the generated package. Oxlint, a narrow repository-shape check, and Prettier are repository-only development gates and never enter the browser artifact. The browser runtime does not compile schemas dynamically. Exact tool versions are pinned in the lockfiles and `mise.toml`, and CI rejects stale generated artifacts.

`onmark-media` depends on core plus `serde` and `serde_json` for a private ffprobe response boundary. It starts the configured ffprobe executable directly with an argument array, never through a shell; wrappers that leave descendant processes holding the output pipes are outside this executable contract. Process lifetime and retained stdout/stderr bytes are explicitly bounded under that direct-child contract, both pipes are drained concurrently, and explicit shutdown reports process-control failures while `Drop` remains a best-effort termination fallback. Private ffprobe response types are translated once into core-owned `AssetMetadata`; JSON values and third-party error types do not define the stable API, while underlying errors remain available through the standard error source chain for debugging.

`onmark-render` owns the heavy Chromium and `FFmpeg` dependency budget. It uses `chromiumoxide` only as a CDP transport and process launcher, with `tokio` and `futures` confined to this asynchronous execution boundary. `tempfile` gives every browser session an isolated profile and owns a private same-filesystem output staging directory, so parallel sessions do not share Chrome locks and a completed MP4 is published with a no-clobber hard link only after both processes finish cleanly. The crate supplies executable paths, viewport, process and request deadlines, frame/input/capture-byte ceilings, bounded retained stderr, and explicit shutdown; Chromium, CDP, and subprocess types are translated into stable render-owned errors. Browser navigation waits separately for document load and the runtime host because the transport's navigation call does not itself establish that lifecycle barrier. Gate one captures one PNG at a time and writes it directly to `FFmpeg`'s `image2pipe`; there is no frame queue or whole-video buffer. Conformance launches installed Chrome and `FFmpeg` against the built runtime, crosses the typed `Load`/`Prepare`/`Seek` handshake, verifies byte-identical repeat capture, probes the resulting H.264 MP4, and decodes it again. These real-process tests are opt-in until CI owns a pinned Chromium/FFmpeg image.

Validation reasons remain local domain values. Once syntax has supplied source context, the `compiler` module is the single owner that translates reasons such as `InvalidNodeId` into source-located `Diagnostic` values, including phase-specific messages and help. `diagnostics` owns only the generic diagnostic representation and stable codes. Neither `model` nor `syntax` depends on diagnostics, and the translation must not be duplicated by callers.

On the TypeScript side, runtime is the foundation. Authoring consumes runtime's types-only public hook and capability contract; bundler injects the pinned runtime artifact. Runtime never depends on authoring or bundler. Temporal capability declarations belong to runtime as the stable third-party adapter extension point. The Gate-one `RuntimeSession` owns protocol ordering, evaluation-bound checks, exact-frame projection, and terminal disposal. It rejects concurrent commands instead of growing a hidden queue. Browser-specific work enters through one narrow adapter whose waits must be bounded and whose expected failures are typed. The session, deterministic frame projection, immutable browser host, native Chromium handshake, and synthetic-frame MP4 path exist today; real media stabilization and the production DOM/media adapter remain Gate-one implementation work.

Rust wire types are the source of truth. `cargo xtask schema` generates checked-in versioned JSON Schema and TypeScript types/codecs, and CI requires regeneration to produce no diff. Generated files are never hand-edited, and Rust does not regenerate a second Rust model from its own schema. Before the first external Gate-one release, v1 is refined in place so the initial public contract does not preserve experimental fields; after publication, an incompatible wire change requires a new protocol version and migration fixture. The Gate-one `BrowserPlan` currently carries only the frame rate and evaluation/output intervals consumed by the browser clock; component and render-graph facts are added only when the runtime consumes them.

AWS Lambda is an adapter, not another engine. A later independently published `@onmark/aws-lambda` surface owns invocation types, infrastructure definitions, the thin handler, and a container image with the pinned Rust binary, Chromium, FFmpeg, and fonts. The handler materializes a Render Unit, calls the same `onmark-render` executor, uploads an immutable artifact, and returns a structured result. AWS SDK types may not enter core. Other backends such as ECS or Kubernetes follow the same adapter rule.

## Delivery gates

**Gate one: render one real video reliably.** Implement the minimal language, media probing, Rust timing, versioned Timeline IR, deterministic browser clock, frame handshake, and a single-process single-unit Chromium/FFmpeg path. Do not build distributed control-plane machinery.

**Gate two: partition correctly.** Render two independent local units and assemble them. Introduce the Render Graph, evaluation/output intervals, preroll, unit caching, and dependency-based invalidation.

**Gate three: leave the machine.** Execute the same plan in independent worker processes with object storage, leases, retries, idempotent publication, and capability scheduling. Validate byte-identical plans, raw-frame hashes in a locked environment, and decoded media equivalence.

Every gate uses the final-direction contracts but implements only fields consumed by that gate. A failed gate blocks construction of the next gate's skeleton.

## Open experimental questions

CDP versus WebDriver BiDi, capture mechanism, layered alpha caching, wire encoding, coordinator storage, adapter seekability, and environment-locking granularity require prototypes and measurements. The pure compiler boundary, deterministic protocol, dependency-driven partitioning, and local/distributed symmetry are foundational decisions.

The first Gate-one capture spike gives positive but deliberately narrow evidence for application-controlled `FrameReady` followed by CDP `Page.captureScreenshot`: repeated DOM, CSS, and Canvas frames produced identical raw RGBA hashes across independent Chrome processes on one locked machine. This selects the next experiment, not the final transport contract; decoded media, WebGL, asynchronous components, cross-environment equality, and production lifecycle remain unproven.
