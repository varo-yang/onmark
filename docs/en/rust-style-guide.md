# The Onmark Rust Code Constitution

> Baseline: Rust 1.97.0 stable (2026-07-09), language edition 2024, style edition 2024.
> This document is canonical and must be re-audited when the pinned toolchain changes.

Beautiful Rust is not code with the fewest characters. It is code whose ownership, failure modes, and state transitions are visible in its types and control flow. Onmark is a deterministic video compiler and renderer; this guide optimizes for correctness under change, reproducibility across machines, and resource-bounded execution.

## The three layers of this constitution

Do not mix three different meanings of “Rust style”:

1. **Official formatting** is delegated to `rustfmt` using style edition 2024. Humans do not invent alignment rules.
2. **Idiomatic design** governs call-site clarity, ownership, typestate, errors, traits, and resource lifetimes.
3. **Onmark engineering law** adds deterministic time, canonical plans, bounded media buffers, and subprocess discipline.

The first layer is mechanical. The second is code aesthetics. The third is product correctness. A formatting-clean program can still fail the latter two.

## Modern Rust aesthetics

### Call sites are the primary design surface

People read uses more often than declarations. Prefer APIs whose meaning is visible without jumping to a definition:

```rust
timeline.resolve(cues, Rounding::TowardZero)?;
worker.capture(frame, CaptureMode::BeginFrame)?;
```

Avoid clever generic signatures that save five lines in a declaration while forcing every caller to understand inference tricks. Standard-library vocabulary is preferred over project synonyms.

### Parse, do not repeatedly validate

Convert untrusted text once into a type that proves the invariant. `CueId::parse` returns a valid `CueId`; downstream functions do not accept `String` and check it again. Use private fields and constructors to make invalid values unconstructable.

### Typestate is for meaningful protocol states

Use distinct state types when they remove a real runtime branch or prevent a protocol violation:

```rust
CaptureSession<Launched> -> CaptureSession<Ready> -> CaptureSession<Closed>
```

Do not typestate every minor implementation step. If states share all operations and cannot be misused, an enum is clearer.

### Ownership should form a tree

Split owned and borrowed/view forms deliberately (`PathBuf`/`Path`, owned IR/view into IR). Make long-lived ownership visible, keep borrows short, and prefer indexes or stable IDs over self-referential graphs. `Cow` is useful only when both borrowed and owned paths are real.

### RAII owns cleanup

Files, temporary directories, browser sessions, encoders, permits, and tracing spans are resources. Their types should make cleanup the default through `Drop` or an explicit async shutdown guard. `Drop` must not hide an operation whose failure the caller must observe.

### Choose polymorphism by the closed/open question

- Closed set known to Onmark: enum + exhaustive `match`.
- Open compile-time behavior: generic type parameter.
- Open runtime-selected behavior: narrow trait object.
- Extra behavior on an external type: extension trait.

Do not translate class hierarchies into trait hierarchies.

### Rust 2024 features are used deliberately

- Return-position `impl Trait` captures in-scope generics and lifetimes by default. Use precise `use<...>` capture when a public API must promise that it does not retain a borrow.
- Every unsafe operation inside an `unsafe fn` still requires an explicit `unsafe {}` block.
- References to `static mut` are forbidden; prefer ownership passed from `main`, atomics, `Mutex`, `OnceLock`, or `LazyLock` according to semantics.
- Prefer exhaustive matching. Rust 2024 match ergonomics are not a reason to hide ownership changes in dense patterns.
- Async closures and modern language conveniences are welcome when they simplify ownership at the call site; novelty alone is not a reason to use them.

## Aesthetic pattern catalog

These are code-review rules, not syntax demonstrations. “Prefer” means the code exposes the domain decision at the call site and makes invalid states harder to construct.

### Code should have a rectangular silhouette

Onmark favors **rectangular code**: a straight normal path, shallow indentation, cohesive blocks, and a small number of visibly parallel phases. This is more than “use early returns.” A reader should be able to recognize the shape of a function before reading every expression.

Avoid a narrowing pyramid:

```rust
for scene in film.scenes() {
    if scene.enabled() {
        if let Some(asset) = assets.get(scene.asset_id()) {
            match probe(asset).await {
                Ok(metadata) => {
                    if metadata.duration > Duration::ZERO {
                        plans.push(build_plan(scene, metadata)?);
                    } else {
                        diagnostics.push(empty_asset(scene));
                    }
                }
                Err(error) => diagnostics.push(probe_failed(scene, error)),
            }
        } else {
            diagnostics.push(missing_asset(scene));
        }
    }
}
```

Prefer a sequence of complete, aligned blocks:

```rust
for scene in film.enabled_scenes() {
    let Some(asset) = assets.get(scene.asset_id()) else {
        diagnostics.push(missing_asset(scene));
        continue;
    };

    let metadata = match probe(asset).await {
        Ok(metadata) => metadata,
        Err(error) => {
            diagnostics.push(probe_failed(scene, error));
            continue;
        }
    };

    if metadata.duration == Duration::ZERO {
        diagnostics.push(empty_asset(scene));
        continue;
    }

    plans.push(build_plan(scene, metadata)?);
}
```

The visual rule has four consequences:

1. **Functions are rectangular.** Keep the happy path at one indentation level. Reject, skip, or translate exceptional cases at the boundary where they appear.
2. **Blocks are cohesive.** Keep recognition, validation, transformation, and emission as visible phases. Do not interleave fragments of all four inside nested closures.
3. **Branches are balanced.** A large `match` arm becomes a named operation when it contains another decision tree; tiny one-line helpers without a domain name are not an improvement.
4. **Modules form a tree, not confetti.** Keep code that changes for the same reason together. Do not scatter one operation across `utils`, extension traits, callbacks, and files merely to make each function shorter.

Top-level orchestration should read like a table of contents:

```rust
pub async fn render(request: RenderRequest) -> Result<RenderOutput, RenderError> {
    let source = load_source(&request).await?;
    let film = compile_film(source)?;
    let assets = resolve_assets(&film).await?;
    let plan = build_render_plan(film, assets)?;
    let segments = render_segments(&plan).await?;

    assemble_output(segments, &plan).await
}
```

Do not extract helpers to chase a line-count target. Extract when the block has a stable domain name, changes for a different reason, can establish a useful type boundary, or hides a self-contained mechanical detail. The desired hierarchy is: **rectangular functions, tree-shaped modules, linear pipelines**.

### Name choices instead of encoding them in booleans

Avoid:

```rust
capture_frame(frame, true, false, 3)?;
```

Prefer:

```rust
capture_frame(
    frame,
    CaptureOptions {
        trigger: CaptureTrigger::BeginFrame,
        alpha: AlphaMode::Opaque,
        retries: RetryLimit::new(3),
    },
)?;
```

The preferred call can be reviewed without opening the function definition. Use an options struct for independent controls and an enum for mutually exclusive choices.

### Parse once, then trust the type

Avoid carrying textual time through the compiler:

```rust
fn resolve_cue(name: &str, seconds: f64) -> Result<f64, String> {
    if !name.starts_with("cue:") || seconds < 0.0 {
        return Err("invalid cue".into());
    }
    Ok(seconds)
}
```

Prefer validation at the syntax boundary:

```rust
pub struct CueName(String);
pub struct TimelineTime(Duration);

impl TryFrom<&str> for CueName {
    type Error = InvalidCueName;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value
            .strip_prefix("cue:")
            .filter(|name| !name.is_empty())
            .map(|_| Self(value.to_owned()))
            .ok_or(InvalidCueName)
    }
}

fn resolve_cue(name: &CueName, at: TimelineTime) -> ResolvedCue;
```

Core code should not repeatedly ask whether an already parsed cue or time is valid.

### Let phases consume phases

Avoid one mutable object whose fields become meaningful only after certain calls:

```rust
let mut document = Document::parse(source)?;
document.resolve_assets()?;
document.solve_timeline()?;
let plan = document.plan.expect("timeline was solved");
```

Prefer phase types:

```rust
let parsed = ParsedFilm::parse(source)?;
let linked = parsed.link_assets(&assets)?;
let solved = linked.solve_timeline()?;
let plan: RenderPlan = solved.lower();
```

Consuming the previous phase prevents “lower before solve” and removes optional fields that merely encode workflow state.

### Model alternatives with variants, not bags of options

Avoid:

```rust
struct ShotTiming {
    duration: Option<Duration>,
    cue: Option<CueName>,
    voice_over: Option<AssetId>,
}
```

Prefer:

```rust
enum ShotTiming {
    Fixed(Duration),
    Until(CueName),
    FromVoiceOver(AssetId),
}
```

The struct permits empty and contradictory combinations. The enum makes every constructed value a deliberate timing rule.

### Do not clone away an ownership question

Avoid:

```rust
let encoder_frame = frame.clone();
let metrics_frame = frame.clone();
encoder.send(encoder_frame).await?;
metrics.observe(metrics_frame.len());
```

Prefer observing before transferring the one large buffer:

```rust
metrics.observe(frame.len());
encoder.send(frame).await?;
```

For frames and media buffers, a clone is a pipeline decision, not borrow-checker punctuation. If two consumers truly need ownership, make the copy explicit and measure it.

### Diagnostics are output; machinery failures are errors

Avoid erasing both into one early-returning error:

```rust
fn compile(source: &str) -> anyhow::Result<RenderPlan>;
```

Prefer preserving the product distinction:

```rust
pub struct CompileReport {
    pub plan: Option<RenderPlan>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn compile(source: &str) -> Result<CompileReport, CompilerFailure>;
```

Unknown cues and conflicting durations are authored mistakes that can be accumulated. An unreadable asset index or violated internal invariant is compiler failure.

### Earn a trait before introducing it

Avoid a Java-shaped wrapper around every implementation:

```rust
trait TimelineSolver {
    fn solve(&self, film: Film) -> Result<RenderPlan, SolveError>;
}

struct TimelineSolverImpl;
```

Prefer the concrete domain operation first:

```rust
pub struct Solver {
    policy: SolvePolicy,
}

impl Solver {
    pub fn solve(&self, film: LinkedFilm) -> SolveReport {
        // ...
    }
}
```

Traits such as `AssetStore` and `FrameSink` belong at boundaries whose implementations genuinely vary. Stable internal algorithms do not need an interface costume.

### Choose clarity over iterator density

One clean transformation suits an iterator:

```rust
let ids = shots.iter().map(Shot::id).collect::<Vec<_>>();
```

State, diagnostics, and ordering usually deserve a named loop:

```rust
let mut cursor = TimelineTime::ZERO;

for shot in &scene.shots {
    let timing = solve_shot(shot, cursor, cues, &mut diagnostics);
    cursor = timing.end;
    solved.push(timing);
}
```

A chain containing `scan`, `filter_map`, side effects, and nested closures is not more idiomatic than a loop that names the timeline cursor.

### Flatten exceptional paths

Avoid nesting the happy path:

```rust
if let Some(cue) = cues.get(name) {
    if cue.at <= film.end {
        return Some(cue.at);
    } else {
        diagnostics.push(out_of_bounds(name));
    }
} else {
    diagnostics.push(unknown_cue(name));
}
```

Prefer `let ... else` and early exits:

```rust
let Some(cue) = cues.get(name) else {
    diagnostics.push(unknown_cue(name));
    return None;
};

if cue.at > film.end {
    diagnostics.push(out_of_bounds(name));
    return None;
}

Some(cue.at)
```

The normal path should be the least-indented path.

### Give mutable state one owner

Avoid a renderer built as a graph of locks:

```rust
struct Runtime {
    queue: Arc<Mutex<VecDeque<Frame>>>,
    encoder: Arc<Mutex<Encoder>>,
    progress: Arc<Mutex<Progress>>,
}
```

Prefer ownership transfer through bounded messages:

```rust
enum EncoderCommand {
    Frame(Frame),
    Finish(oneshot::Sender<EncodeSummary>),
}

let (commands, inbox) = mpsc::channel::<EncoderCommand>(8);
tokio::spawn(run_encoder(inbox, encoder));
```

The encoder task owns the encoder. Capacity `8` is part of the backpressure design and must be justified and observable.

### Make cleanup structural

Avoid cleanup skipped by `?` or cancellation:

```rust
let child = Command::new("ffmpeg").spawn()?;
render_frames().await?;
child.kill().await?;
```

Prefer an explicit resource lifecycle:

```rust
let mut encoder = EncoderProcess::spawn(config).await?;
let render_result = render_frames(&mut encoder).await;
let shutdown_result = encoder.shutdown().await;

render_result?;
shutdown_result?;
```

`Drop` is only a best-effort forced-termination safety net. Fallible asynchronous shutdown remains explicit so its failure is observable.

### Time has units and a rounding policy

Avoid:

```rust
let frame = (seconds * fps as f64) as u64;
```

Prefer:

```rust
let frame = timebase.frame_at(timestamp, Rounding::Floor)?;
```

`TimelineTime`, `FrameIndex`, `FrameCount`, and `Timebase` may share integer storage but are not interchangeable. Rounding lives in one time module and is named at the call site.

## Preamble: scope

The repository will contain different kinds of Rust. Apply the shared rules everywhere, then the relevant supplement.

| Kind | Typical responsibility | Dominant concern |
| --- | --- | --- |
| Pure compiler core | parse, validate, solve time, build IR | total functions and diagnostics |
| Planning core | dependency graph, cache keys, render plan | determinism and canonical data |
| Worker | Chromium/FFmpeg control, capture, encode | bounded resources and cleanup |
| Orchestrator | schedule, retry, assemble | idempotency and cancellation |
| CLI | commands, reporting, exit codes | stable public behavior |
| Browser bridge | typed messages to the TypeScript runtime | protocol compatibility |

`unsafe` is forbidden by default. A future crate that genuinely requires it must be isolated behind a safe API and approved as a constitutional exception.

---

## Architecture

### 1. Types are the pipeline

Each phase produces a different type. Do not keep mutating one universal structure.

```rust
pub fn parse(source: &SourceDocument) -> ParseReport<ParsedFilm>;
pub fn resolve(parsed: ParsedFilm, assets: &AssetCatalog) -> ResolveReport<ResolvedFilm>;
pub fn plan(film: ResolvedFilm, profile: &RenderProfile) -> RenderPlan;
```

`ParsedFilm` cannot be rendered. `ResolvedFilm` cannot contain an unknown cue. `RenderPlan` cannot contain an unresolved asset. Illegal phase combinations should fail to compile.

Use newtypes for values that share a representation but not a meaning:

```rust
pub struct FrameIndex(u64);
pub struct FrameCount(u64);
pub struct CueId(String);
pub struct NodeId(String);
pub struct ContentHash([u8; 32]);
```

Never represent timeline truth with unlabelled `f64`. Preserve exact frame rates and time bases with integer or rational types.

### 2. Dependencies flow inward

Pure crates know nothing about filesystems, subprocesses, networks, Chromium, or FFmpeg. IO crates may depend on pure crates; the reverse is forbidden.

```text
syntax → compiler → plan
                    ↑
         worker / cli / orchestrator
```

No `utils`, `common`, or `shared` dumping-ground crate. A type lives with the concept that owns it. Cross-crate consumers use public, fact-shaped APIs rather than reaching into another crate's AST.

### 3. Traits mark real boundaries

Introduce a trait when it represents a stable capability boundary and at least one is true:

- there are two real implementations, or runtime selection is an actual requirement;
- a test needs to replace an external boundary;
- the trait is the intentional public extension point.

Do not create a trait for every struct, and do not demand two implementations mechanically when one real external boundary already needs a stable contract. Static dispatch is the default; `dyn Trait` is reserved for runtime-selected implementations. Keep traits narrow and name them by capability (`AssetStore`, `FrameSink`), not by architecture suffix (`AssetStoreInterface`).

### 4. Wiring stays visible

Long-lived resources are constructed at a process boundary and passed explicitly. No service locator, mutable global registry, or hidden singleton.

`main.rs` should read as the process graph: load configuration, construct resources, run the command, shut resources down.

---

## Data and ownership

### 5. Borrow at the edge; own across time

- Accept `&str`, `&Path`, and slices when the callee only observes data.
- Return owned domain values when ownership transfers.
- Use `Arc<T>` only for genuinely shared immutable state or a measured cross-task need.
- Do not add `clone()` merely to satisfy the borrow checker. First reconsider ownership and lifetime boundaries.
- Never expose a lock guard across a public API.

Large frame and media buffers must have one obvious owner. Copies across Chromium, queues, and encoders are architectural events and should be measurable.

### 6. Enums beat boolean blindness

Do not write:

```rust
render(frame, true, false)?;
```

Write:

```rust
render(frame, CaptureMode::BeginFrame, AlphaMode::Opaque)?;
```

Prefer discriminated enums over structs with mutually exclusive `Option` fields. Match exhaustively; wildcard arms are forbidden for domain enums unless forward compatibility is the explicit contract.

### 7. Conversions say whether they can fail

- `From` / `Into`: lossless and infallible.
- `TryFrom` / `TryInto`: validation or failure is possible.
- `as_`: borrowed view.
- `to_`: allocates or computes a new value.
- `into_`: consumes the receiver.

Parsing external text ends at a boundary. Beyond that boundary, pass validated domain types rather than strings and `serde_json::Value`.

---

## Errors and diagnostics

### 8. Authored mistakes are data; broken machinery is error

An invalid screenplay is expected product input. Return diagnostics and aggregate them:

```rust
pub struct Diagnostic {
    code: DiagnosticCode,
    primary: SourceSpan,
    message: Box<str>,
    help: Option<Box<str>>,
    related: Vec<RelatedDiagnostic>,
}
```

Fields are exposed through read-only accessors. Constructors reject blank
messages, help, and related explanations. Severity is determined by the
stable diagnostic code rather than chosen independently at each call site.

Do not stop at the first unknown cue if five independent errors can be reported safely.

Filesystem failure, a crashed encoder, corrupt IPC, and violated internal invariants are execution errors. Libraries return typed error enums. Binaries may attach human-readable context at their outer boundary.

No stringly typed errors, no `Box<dyn Error>` in library APIs, and no using `panic!` for recoverable input or infrastructure failures.

### 9. `unwrap` proves an invariant or does not exist

`unwrap()` and `expect()` are allowed in tests. In production code, `expect()` is allowed only when the invariant is established locally and the message explains the invariant. A comment or type-level construction is preferable.

`panic!`, `unreachable!`, and `unimplemented!` indicate programmer defects, never user mistakes.

### 10. Error translation happens once

Translate third-party errors at the boundary that owns the dependency. The compiler must not leak XML parser errors; the worker must not leak raw subprocess exit structures. Preserve sources for debugging while exposing stable Onmark codes.

---

## Control flow and APIs

### 11. Top-level functions read like orchestration

A top-level operation should be a short sequence of named phases. Variant-heavy logic uses an exhaustive `match` or a table of handlers. Avoid long necklaces of `if kind == ...` branches that mix recognition, validation, mutation, and reporting.

Early returns are welcome when they remove nesting. Dense iterator chains are not automatically more idiomatic than a readable loop.

### 12. Public APIs are small and unsurprising

- Default to private; use `pub(crate)` before `pub`.
- Constructors validate invariants.
- Getters do not repeat `get_`.
- Functions with an obvious receiver are methods.
- Avoid output parameters.
- Use option structs when a function has several independent controls.
- Builder APIs are justified only when they make invalid intermediate states impossible or materially improve construction.
- Public types implement useful standard traits (`Debug`, equality, hashing, display) when semantics permit.

Public rustdoc explains why an API exists, includes a realistic example, and documents `# Errors`, `# Panics`, and `# Safety` where applicable.

Non-trivial implementation modules start with concise inner rustdoc that names
their responsibility, boundary, and principal invariant. Private state types
document information that their fields cannot express alone: recovery meaning,
resource ownership, concurrency obligations, and protocol trade-offs. Do not
add comments merely to narrate control flow or restate a precise type name.

### 13. Protocol values have one owner

Message names, diagnostic codes, filenames, environment variables, JSON field names, cache-key components, and browser globals are protocol. Define each once in its owner crate. Never copy a protocol string to avoid an import.

Wire enums and persisted formats require explicit versioning. Internal enum renames must not silently alter serialized output.

Expanding protocol enums use `#[non_exhaustive]` so external consumers must tolerate later variants. Local validation-reason enums remain exhaustive when they describe the closed failure contract of one constructor; adding a reason is then an intentional API change rather than silently falling through a wildcard.

---

## Determinism

### 14. Stable output is designed, not hoped for

- Never depend on `HashMap` iteration order when producing bytes or diagnostics. Sort explicitly or use an ordered map.
- Do not read wall-clock time, locale, timezone, environment variables, or randomness inside pure compilation.
- Randomness used for rendering must be seeded and included in the plan hash.
- Canonical serialization has one implementation.
- Cache keys hash the bytes actually consumed, plus every relevant runtime and environment version.
- Equivalent input produces byte-identical IR and stable diagnostic ordering.

Time conversion and rounding live in one module. Frame boundaries must have named rounding semantics; scattered casts from seconds to integers are forbidden.

### 15. Idempotency is observable

Every render task has a deterministic identity. Repeating the same task may reuse work, but it must not append, duplicate, or mutate shared output. Temporary output is written separately and committed atomically.

---

## Async, concurrency, and processes

### 16. Concurrency is bounded

- No unbounded task spawning or unbounded channels.
- Every queue has a capacity, backpressure policy, and owner.
- Do not hold a mutex guard across `.await`.
- CPU-heavy work does not run on the async executor.
- Cancellation propagates into child tasks and subprocesses.
- Cleanup is structured and idempotent.

Prefer message passing and ownership transfer over a graph of `Arc<Mutex<_>>`. Shared mutable state requires a written invariant.

### 17. Subprocesses are typed resources

Chromium and FFmpeg are spawned with argument arrays, never through a shell string. Capture stdout/stderr with explicit size limits. Define startup, readiness, graceful shutdown, forced termination, and process-tree cleanup.

An exit code is not a diagnostic by itself. Translate it into a stable error with the command role, bounded stderr tail, and relevant artifact paths.

### 18. Backpressure reaches the producer

When the encoder is slower than capture, the renderer must slow capture rather than accumulate frames in memory. Buffer count and byte count are both bounded and observable.

---

## Performance and unsafe code

### 19. Measure before making code less obvious

Optimize pipeline boundaries before micro-optimizing pure functions. Record allocation volume, copied bytes, queue depth, Chromium capture time, encoder time, and cache hit rate.

No speculative object pools, custom allocators, lock-free queues, SIMD, or `unsafe`. A benchmark and profile must identify the bottleneck first.

### 20. Unsafe code has a quarantine rule

Safe crates use `unsafe_code = "forbid"`. This is intentionally not a universal workspace lint: `forbid` cannot be lowered by a future audited native bridge. A crate that contains approved unsafe code does not inherit the safe-crate lint profile and instead denies `unsafe_op_in_unsafe_fn` and undocumented unsafe blocks. If a measured requirement cannot be met safely:

1. isolate unsafe code in a dedicated crate or module;
2. expose a safe API;
3. document every unsafe block with `// SAFETY:` and the maintained invariant;
4. add boundary, property, sanitizer/Miri, and concurrency tests as applicable;
5. register a constitutional exception.

---

## Testing

### 21. Conformance is the product contract

The merge gate is built around fixtures, not implementation details:

- source → canonical parsed form;
- source + assets → resolved timeline IR;
- invalid source → stable diagnostics;
- render plan → stable canonical bytes and hash;
- task → deterministic frame/video artifact under a pinned environment.

The first step of a bug fix is a failing regression fixture.

### 22. Test at the right level

- Unit tests: pure transforms and edge cases.
- Property tests: time algebra, interval relations, canonicalization, DAG invariants.
- Golden tests: diagnostics, IR, and plans.
- Integration tests: filesystem, Chromium, FFmpeg, cancellation, cleanup.
- Concurrency-model tests: only for genuinely subtle shared-state code.
- Benchmarks: stable representative compositions, not toy loops.

Tests use public APIs where possible. Mock external boundaries, not in-house pure functions.

---

## Toolchain and formatting law

The repository pins the exact current baseline rather than floating on `stable`:

```toml
# rust-toolchain.toml
[toolchain]
channel = "1.97.0"
profile = "minimal"
components = ["clippy", "rustfmt"]
```

Language edition and MSRV are explicit and separate from formatting style:

```toml
[workspace.package]
edition = "2024"
rust-version = "1.97"
```

Formatting follows the official Rust 2024 style edition with almost no taste knobs:

```toml
# rustfmt.toml
edition = "2024"
style_edition = "2024"
max_width = 100
use_small_heuristics = "Default"
```

Consequences include four-space block indentation, trailing commas in multiline lists, 100-column code, 80-column comments where practical, and Rust 2024 version-aware sorting. Do not hand-align fields or arguments; default `rustfmt` wins.

### Lints are scoped by crate kind

Workspace defaults contain only high-signal rules. `pedantic`, `restriction`, `cargo`, and `nursery` are never enabled wholesale as hard errors. The compiler core may forbid panicking APIs; tests may use them; the CLI may write to its injected terminal writer; a library may not print.

Required merge gates:

```bash
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo check --workspace --all-targets --all-features --keep-going
cargo clippy --workspace --all-targets --all-features
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Baseline workspace policy:

```toml
[workspace.lints.rust]
missing_debug_implementations = "warn"
unsafe_op_in_unsafe_fn = "deny"

[workspace.lints.clippy]
all = { level = "deny", priority = -1 }
dbg_macro = "deny"
todo = "deny"
unimplemented = "deny"
```

Each crate opts into its profile in `lib.rs` or `main.rs`:

```rust
// Pure library crate.
#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![deny(clippy::unwrap_used, clippy::print_stdout, clippy::print_stderr)]
```

Binary and test targets use different explicit profiles. Warnings from `pedantic` remain warnings: CI's Cargo warning policy applies to compiler build warnings, while Clippy runs with the levels declared in manifests and crate roots. We do not append `-D warnings` to Clippy and accidentally turn every opinionated lint into a hard law.

Use `#[expect(lint, reason = "...")]` for a narrow, known occurrence when supported; an unfulfilled expectation then becomes visible. Stable crate-wide disagreements may use a documented crate-level `allow`. Any Onmark policy exception also uses:

```rust
// onmark-exception: R<clause> <one-sentence reason>
```

and is listed in the pull request.

---

## Verdict-level anti-patterns

Any of the following fails review unless explicitly justified:

- timeline seconds stored as free-floating numbers;
- domain IDs passed as interchangeable strings;
- one mutable struct representing every compilation phase;
- `pub` used as a substitute for package design;
- a `utils`, `common`, or `shared` module with no domain owner;
- a trait with one implementation and no boundary role;
- cloning to silence ownership errors;
- `Arc<Mutex<_>>` as the default architecture;
- locks held across `.await`;
- unbounded channels or spawned tasks;
- shell command construction for Chromium or FFmpeg;
- `serde_json::Value` beyond an external parse boundary;
- nondeterministic map iteration in persisted output;
- direct environment reads outside the configuration boundary;
- `unwrap`, `panic!`, or first-error abort for authored input;
- an optimization without a representative benchmark;
- comments that narrate syntax instead of explaining invariants and trade-offs.

---

## Sources

Audited on 2026-07-11 against [Rust 1.97.0](https://blog.rust-lang.org/2026/07/09/Rust-1.97.0/), the official [Rust Style Guide](https://doc.rust-lang.org/nightly/style-guide/), [Rust 2024 Edition Guide](https://doc.rust-lang.org/edition-guide/rust-2024/), [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/), [Rust Compiler Development Guide conventions](https://rustc-dev-guide.rust-lang.org/conventions.html), official [Clippy guidance](https://doc.rust-lang.org/stable/clippy/), and Google's actively maintained [Idiomatic Rust](https://google.github.io/comprehensive-rust/idiomatic/welcome.html). Project-specific rules on deterministic time, media buffers, subprocesses, and render plans are Onmark requirements rather than universal Rust conventions.
