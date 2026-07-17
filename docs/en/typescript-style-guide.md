# The Onmark TypeScript Code Constitution

> Baseline: TypeScript 7.0.2, Node.js 26.4.0, pnpm 11.9.0. Language:
> [中文](../zh-CN/typescript-style-guide.md)

Beautiful TypeScript makes ownership, protocol state, asynchronous boundaries,
and browser effects visible. Onmark uses TypeScript for authoring, bundling, and
the browser runtime; it does not use TypeScript to re-solve Rust-owned timing or
planning facts.

## Scope

Different code has different dominant risks:

| Kind                  | Examples                       | Dominant concern                          |
| --------------------- | ------------------------------ | ----------------------------------------- |
| Browser runtime       | clock, session, DOM adapters   | deterministic state and bounded readiness |
| Node toolchain        | authoring, bundler, generators | explicit IO and reproducible output       |
| Wire boundary         | generated types and codecs     | one source of truth and compatibility     |
| Tests and conformance | fakes, fixtures, smoke tests   | public behavior rather than internals     |

Generated files follow their generator and are never hand-edited. Review the
Rust wire type, schema, generator, and drift check instead of polishing
generated mechanics by hand.

## Architecture

### 1. Rust owns timeline facts; TypeScript owns browser effects

Rust is the sole owner of authored semantics, timing, intervals, partitioning,
and execution planning. TypeScript consumes versioned facts and applies them to
DOM, CSS, Canvas, WebGL, and browser media APIs. It may validate a wire contract
or derive a browser API argument from an exact frame fact; it must not invent a
second timing solver, cue resolver, or partition policy.

### 2. One concept has one source of truth

- Rust wire types generate checked-in JSON Schema and TypeScript types/codecs.
- Protocol unions, codes, versions, and field names are not retyped by hand.
- Cross-file protocol strings are named once in the package that owns them.
- Committed generated artifacts have a deterministic `--check` gate in CI.
- A copied constant is a defect unless the boundary makes sharing impossible and
  the duplication is tested explicitly.

### 3. Dependencies flow through package facades

Packages import another package through its public `exports`, never through its
`src/` tree. `@onmark/runtime` never imports authoring or bundler. Production
runtime modules do not import Node built-ins. Generated internals may be
consumed inside runtime, but external consumers use `src/index.ts` exports.

Do not create `utils.ts`, `helpers.ts`, `shared.ts`, or `common.ts`. A function
lives beside the concept whose invariant it protects.

### 4. Effects enter through narrow capabilities

Pure recognition, validation, state transition, and formatting do not touch the
DOM, filesystem, environment, network, or subprocesses. Browser and Node effects
enter through narrow interfaces passed explicitly. Long-lived resources are
constructed at a process or session boundary; there is no service locator,
mutable global registry, or decorator-based injection.

One real external boundary plus a test fake is enough to justify an interface.
Stable internal algorithms do not receive interface costumes.

## Types and ownership

### 5. Illegal states should be difficult to represent

Use discriminated unions for protocol phases and result alternatives. Avoid a
single interface whose optional fields encode several incompatible states.
Narrow `unknown` at parse boundaries; `any` is forbidden. Exhaustive `switch`
statements are preferred for closed protocol variants.

Use `readonly` for borrowed facts and public views. When mutable input crosses
an asynchronous lifetime, take an owned snapshot before the first `await`. Do
not retain caller-owned arrays or objects and hope they remain unchanged.

### 6. Classes must earn identity and lifetime

Functions are the default for stateless transformations. A class is appropriate
only when identity or lifecycle is real:

- an `Error` subtype;
- a protocol session or browser resource with owned mutable state;
- a platform-mandated browser type such as `HTMLElement`.

Do not create `XxxService`, `XxxManager`, or `XxxRepository` classes as
dependency bags. Platform classes remain thin glue: read browser state, call a
focused operation, apply the result.

### 7. Names expose units and choices

Names read aloud at call sites. File names name their concept; functions name an
action; types name a noun; errors end in `Error`. Frames, seconds, hashes,
request IDs, and intervals must not become interchangeable generic `value`
parameters.

Use an options object for independent controls and a discriminated union or
enum-like string union for mutually exclusive choices. Avoid boolean blindness.

## Control flow and failures

### 8. Control flow is block-shaped

Top-level orchestration should read like a table of contents. Make the main
variant axis visible with an exhaustive `switch`, then give each substantial
variant one rectangular operation. Prefer guard clauses and `if ... else`
narrowing over nested pyramids or dense iterator chains with side effects.

Do not extract a helper merely to reduce line count. Extract when the block has
a stable domain name, protects an invariant, or isolates a mechanical boundary.

### 9. Expected failures are data; unexpected failures throw

Invalid authored input and protocol rejections are normal product output. Return
diagnostics or a typed failure event and aggregate independent failures where
safe. Infrastructure faults, impossible states, and violated internal
preconditions throw.

`try/catch` belongs only where code can translate meaningfully:

- a protocol or process boundary;
- third-party exceptions translated into typed Onmark failure data;
- resource cleanup;
- concurrent aggregation.

Do not catch an error merely to rename it or silently continue. Untyped adapter
exceptions are contained at the runtime session boundary and are not leaked as
vendor-dependent protocol messages.

### 10. Async work and cleanup are bounded

Every wait has an owner and, when it can depend on external state, a deadline.
Every queue has an explicit capacity or rejects concurrent work. Do not hide an
unbounded promise chain behind a friendly API. Unknown browser components remain
sequential until random seekability is proven.

Cleanup is explicit and terminal. A failed disposal remains observable, but a
partially disposed session cannot return to service. Fire-and-forget promises
must be structurally owned and deliberately marked; accidental floating promises
are defects.

### 11. Determinism is a type-level concern

Browser output is driven by frame index and rational timebase, never wall time,
`Date.now()`, ambient animation progress, or unseeded randomness. Iteration that
affects wire output, hashes, diagnostics, or generated bytes uses a stable
order. Hash the bytes actually consumed by the browser, not a nearby source file
that looks equivalent.

## Files, comments, and public surfaces

### 12. Files carry their own navigation

Every handwritten TypeScript or JavaScript source file starts with a short
header explaining what it owns and why it exists. A file over 200 lines uses
section dividers for its major conceptual blocks. Generated files need only a
generated banner naming their source of truth.

Modules form a tree rather than confetti. Keep code that changes for the same
reason together; split a file when sections have different owners, not when a
line-count target is crossed.

### 13. Comments explain constraints, not syntax

Comments are required for ownership across asynchronous boundaries, concurrency
races, cleanup decisions, non-obvious browser behavior, and deliberate protocol
trade-offs. Comments must not narrate a loop, repeat a name, preserve history,
or contain a bare `TODO`. A deferred item names its owner and activation
condition.

Public entry points re-export a narrow, intentional surface. Tests and other
packages consume that surface instead of reaching into implementation files.

## Testing and generated code

### 14. Tests assert behavior through public APIs

Use real pure functions and fake only external capabilities such as a browser
adapter, filesystem, or process. Do not patch internal functions with a mocking
framework. Package behavior tests live under `test/` with the same concept name
as their source; cross-language behavior belongs under root `conformance/`.

The first step of a bug fix is a failing focused test or fixture. Snapshot-style
goldens are reserved for conformance and generated artifacts, not used as a
substitute for behavioral assertions.

### 15. Scripts are production-quality build code

Generators and CI scripts use named constants, deterministic ordering, explicit
exit status, actionable stderr, and read-only check modes. They obey the same
header, naming, failure, and formatting rules as package code. A check command
must not mutate the repository.

## Tooling baseline

`tsc --noEmit` is mandatory with `strict`, `noUncheckedIndexedAccess`,
`exactOptionalPropertyTypes`, `noImplicitOverride`,
`noPropertyAccessFromIndexSignature`, `isolatedModules`, and
`verbatimModuleSyntax`. Linting rejects explicit `any`, default exports in
product code, `console`, inconsistent type imports, and direct `process.env`
access. Formatting is mechanical and checked in CI. Handwritten browser sources
under root `conformance/` receive the same strict typecheck, lint, shape, and
format gates as package source; a successful bundler build is not a substitute
for typechecking. Generated output is excluded from hand-formatting and checked
through regeneration instead.

## Verdict-level anti-patterns

- TypeScript timing, cue, or partition logic duplicating Rust.
- Handwritten copies of generated protocol types or codes.
- `any` outside generated third-party output.
- Unbounded queues, waits, retained buffers, or promise chains.
- Free-running browser time determining a captured frame.
- A mutable object representing every runtime phase without a discriminant.
- Caller-owned mutable data retained across `await`.
- Service-locator globals, DI containers, or dependency-bag classes.
- `utils`, `helpers`, `shared`, or `common` dumping grounds.
- Default exports in product modules, `console.log`, or ambient `process.env`.
- Hand-edited generated files or a mutating drift check.
- Comments that merely translate the following line into English.

## Provenance

This constitution was adapted for Onmark after reviewing uiku's TypeScript code
constitution, runtime, toolchain, tests, lint configuration, and style drift
check on 2026-07-11. Onmark deliberately keeps its own rules where the products
differ: Rust owns video timing, `RuntimeSession` is a legitimate lifecycle
class, Node's native test runner is sufficient, and browser/runtime dependency
budgets follow Onmark's delivery gates.
