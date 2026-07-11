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
  → Linked Film
    → Timeline IR
      → Render Graph
        → Execution Plan
```

- **Source AST** preserves authored structure and source spans.
- **Linked Film** contains resolved IDs, cues, assets, and component references.
- **Timeline IR** contains exact frame intervals and provenance for every timing decision.
- **Render Graph** records pixel, media, transition, history, global-layer, and audio dependencies.
- **Execution Plan** contains immutable work units, environments, dependencies, output intervals, evaluation intervals, and cache keys.

A render unit has separate `evaluation` and `output` intervals. Evaluation may include transition preroll, animation warm-up, or history frames; only output frames are published.

## End-to-end pipeline

```text
freeze inputs → probe media → compile → bundle → render graph
  → partition → capture/encode → mix audio → verify/assemble
```

The compiler performs parse, bind, validate, solve, and lower without IO. The planner computes dependency closure before partitioning. Simple films naturally partition around shots; transitions, persistent elements, global effects, and historical shaders widen or merge units for correctness.

Each worker executes one state machine:

```text
claim → materialize → launch → ready → seek/capture → encode → verify → commit
```

Artifacts are verified and atomically published under content hashes. Audio follows a separate plan and is mixed outside browser frame capture.

## Deterministic browser protocol

The sole clock derives from frame index and rational timebase. Wall time and free-running animation or media clocks may not determine output.

The runtime protocol includes `Load`, `Prepare`, `Seek`, `FrameReady`, and `Dispose`. Components declare temporal behavior such as `stateless`, `warmup(n)`, `sequential`, `global`, or `neighbor(radius)`. Unknown components default to `sequential`: parallel seekability must be proven, not guessed. Native APIs and audited adapters may safely provide stronger declarations. Detection may recommend an adapter but may not silently relax correctness.

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
├── deploy/aws-lambda/  # introduced at delivery gate three
├── schemas/ conformance/ evals/ examples/ docs/
```

The first Rust workspace contains `onmark-core`, `onmark-media`, `onmark-render`, and `onmark-cli`. Media is separate now because server-side compile/lint loops need probing without Chromium; this is already both a distinct dependency budget and a real consumer. Render depends on core and media. Syntax, diagnostics, model, compiler, timeline, and protocol remain rectangular modules inside core. Render graph and planning initially join core at gate two. Worker execution belongs to render. A coordinator appears only at gate three.

Core's internal dependency DAG is CI-enforced: `model` depends on nothing; `syntax`, `diagnostics`, and `timeline` may depend on model; `compiler` may depend on those four; `protocol` may depend on model, diagnostics, and timeline. No domain module depends back on protocol. New edges require an architecture change. CI performs a syntax-aware check of explicit Rust paths with `syn`. This cooperative guard covers ordinary paths, imports, aliases, and re-exports, but not paths generated inside macros or full rustc name resolution; review remains responsible for those edges.

`onmark-core` uses `xmlparser` only inside `syntax` for pure, span-preserving XML-compatible fragment tokenization. Onmark owns tree construction, nesting checks, duplicate-attribute checks, reference decoding, and all authored semantics. Parser errors are translated at the syntax boundary and the dependency performs no IO. Test targets may use `proptest` for time algebra and `syn` for the cooperative module dependency-law check; neither development dependency is linked into library consumers or runtime artifacts.

Validation reasons remain local domain values. Once syntax has supplied source context, the `compiler` module is the single owner that translates reasons such as `InvalidNodeId` into source-located `Diagnostic` values, including phase-specific messages and help. `diagnostics` owns only the generic diagnostic representation and stable codes. Neither `model` nor `syntax` depends on diagnostics, and the translation must not be duplicated by callers.

On the TypeScript side, runtime is the foundation. Authoring consumes runtime's types-only public hook and capability contract; bundler injects the pinned runtime artifact. Runtime never depends on authoring or bundler. Temporal capability declarations belong to runtime as the stable third-party adapter extension point.

Rust wire types are the source of truth. `cargo xtask schema` generates checked-in versioned JSON Schema and TypeScript types/codecs. CI regenerates and requires a clean diff. Generated files are never hand-edited, and Rust does not regenerate a second Rust model from its own schema.

AWS Lambda is an adapter, not another engine. A later independently published `@onmark/aws-lambda` surface owns invocation types, infrastructure definitions, the thin handler, and a container image with the pinned Rust binary, Chromium, FFmpeg, and fonts. The handler materializes a Render Unit, calls the same `onmark-render` executor, uploads an immutable artifact, and returns a structured result. AWS SDK types may not enter core. Other backends such as ECS or Kubernetes follow the same adapter rule.

## Delivery gates

**Gate one: render one real video reliably.** Implement the minimal language, media probing, Rust timing, versioned Timeline IR, deterministic browser clock, frame handshake, and a single-process single-unit Chromium/FFmpeg path. Do not build distributed control-plane machinery.

**Gate two: partition correctly.** Render two independent local units and assemble them. Introduce the Render Graph, evaluation/output intervals, preroll, unit caching, and dependency-based invalidation.

**Gate three: leave the machine.** Execute the same plan in independent worker processes with object storage, leases, retries, idempotent publication, and capability scheduling. Validate byte-identical plans, raw-frame hashes in a locked environment, and decoded media equivalence.

Every gate uses the final-direction contracts but implements only fields consumed by that gate. A failed gate blocks construction of the next gate's skeleton.

## Open experimental questions

CDP versus WebDriver BiDi, capture mechanism, layered alpha caching, wire encoding, coordinator storage, adapter seekability, and environment-locking granularity require prototypes and measurements. The pure compiler boundary, deterministic protocol, dependency-driven partitioning, and local/distributed symmetry are foundational decisions.
