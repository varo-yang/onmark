# Onmark Language Specification

> Status: Gate-one language contract. Deferred capabilities are listed explicitly.

## Purpose

Onmark does not redraw a track timeline with different tags. Authors express content, order, ownership, and a small number of meaningful alignments. The compiler derives absolute frames, maintains constraints, and explains failures.

The language is paired with the render architecture through one versioned contract: Timeline IR.

## Axioms

1. Sequence is the default.
2. Content determines duration when media already has duration.
3. Explicit alignment uses named time events rather than track coordinates.
4. Local relationships stay local.
5. Structure should make common illegal states unrepresentable.
6. Remaining errors are source-located and actionable.
7. Execution concepts such as workers, cache keys, and render units never enter the screenplay.

## Core model

The Gate-one vocabulary is `film`, `cues`, `cue`, `scene`, `shot`, `video`, `vo`, `title`, and `cta`. A film may contain at most one direct `cues` child; that container owns only `cue` declarations and does not participate in scene sequencing. Scenes own sequential shots. A shot owns its `video`, `vo`, `title`, and `cta` content. Titles and CTAs are overlays and do not participate in sibling sequencing. `video` is Gate one's only media element and names its artifact with `src`; audio, image, and other media elements remain deferred. Structural binding retains `src` and other unparsed authored attributes for the attribute/reference resolution phase rather than discarding them.

Illustrative syntax:

```html
<film>
  <cues>
    <cue id="offer" time="3s" />
    <cue id="cta" time="7s" />
  </cues>
  <scene id="sale">
    <shot id="hero">
      <video src="product.mp4" />
      <title cue="offer">30% OFF</title>
      <cta cue="cta">Buy now</cta>
    </shot>
  </scene>
</film>
```

`cue="offer"` is the Gate-one spelling for aligning an overlay to a named cue. Free-form `begin`, `end`, and `until` expressions are not part of the language.

## Markup syntax

Screenplays use XML-compatible fragment markup. Syntax preserves a sequence of top-level nodes and validates tokens, element nesting, closing-tag matches, duplicate attributes, and character references. Binding later requires exactly one top-level `film`; root cardinality, known element names, legal containment, required attributes, IDs, and references are language semantics rather than markup well-formedness.

Element and attribute names are case-sensitive qualified names. The syntax tree owns decoded text and attribute values together with byte-accurate source spans. Comments are ignored, CDATA becomes ordinary text, and XML declarations, processing instructions, and document type declarations are not part of the screenplay surface.

Text and attribute values support the five predefined XML entities (`amp`, `lt`, `gt`, `quot`, and `apos`) plus decimal and hexadecimal references to characters allowed by XML 1.0. Other named entities, malformed references, surrogate values, out-of-range Unicode values, and XML-forbidden characters are syntax errors. Onmark does not process DTDs, custom entities, or external entities.

## Time

Authored durations use the exact grammar `integer[.fraction](s|ms)` with no whitespace or sign. Seconds admit at most nine fractional digits and milliseconds at most six, so every accepted value has an exact unsigned nanosecond representation. Frame units and floating-point approximations are not part of the language.

The compiler maps exact nanosecond values onto a rational frame grid with integer arithmetic. Every conversion names either floor or ceiling rounding at its call site; no implicit cast or ambient default may choose a frame boundary. Gate-one authored starts, delays, cue times, and durations select the first frame boundary that is not earlier than the exact value (`Ceil`), so a positive sub-frame value never silently becomes zero frames. `Floor` remains available only for rules that explicitly require attribution to an earlier boundary.

A shot obtains duration from probed media, probed voice-over, or a restricted explicit duration when content provides none. Multiple primary content sources extend the shot to the longest source. Gate one does not allow a shot to end at a cue. Overlay elements do not silently extend their shot.

Gate one has two explicit relationships:

- `delay` is relative to the owning shot's start;
- a named cue aligns an overlay to an authored absolute film event.

An overlay starts at its resolved relationship, or at the owning shot's start when none is authored, and remains active until that shot's exclusive end. Gate one gives overlays no independent default duration. An overlay therefore cannot extend its shot, and a resolved start outside the owning shot is an authored timing error.

Gate-one cues use authored absolute film time. No other cue source is part of the current language.

All resolutions preserve provenance in `TimingReason`, allowing the compiler to explain not only where an element landed but why.

## Voice-over

`vo` pairs authored inscription with a frozen media artifact. Text supports reading, review, subtitles, and editing; `src` supplies rendered audio and measured duration. The reference is a screenplay-relative portable path: it uses `/` separators and cannot be absolute, contain `..`, empty or `.` components, backslashes, or a platform prefix. The referenced artifact must expose an audio stream; otherwise solving reports `ONM-ASSET-002` at `src`. TTS belongs upstream. The compiler is offline and deterministic, and content hashes detect stale text/artifact pairs. Gate one materializes each solved voice-over into the private render root and mixes it outside browser capture at its solved frame interval. The presentation does not play, delay, or mix voice-over audio.

## IDs and references

Explicit IDs, including cue IDs, are non-empty, case-sensitive, and globally unique within one film. In keeping with the HTML `id` constraint, they may not contain ASCII whitespace. Non-ASCII characters are preserved exactly; the compiler does not silently normalize authored IDs. A later typed `CueId` or `EventRef` distinguishes cue references without creating a second declaration namespace.

## Attributes and resolution

Structural binding is followed by attribute and reference resolution. `film`, `cues`, and `scene` admit no non-ID attributes. `cue` requires `id` and `time`. `shot` admits optional `duration`. `video` and `vo` admit optional `src` and `delay`; `title` and `cta` admit optional `cue` or `delay`. `cue` and `delay` cannot appear together on one overlay because they define competing start rules. Missing media `src` remains valid for static analysis; an authored empty `src` is invalid. Unknown attributes are errors.

## Diagnostics

Diagnostics contain a stable code, severity, source span, message, actionable help, and related spans. They use screenplay vocabulary rather than solver internals and aggregate independent authored errors when safe.

Initial markup diagnostics are:

| Code | Meaning |
| --- | --- |
| `ONM-SYNTAX-001` | malformed markup that cannot produce another trustworthy token |
| `ONM-SYNTAX-002` | a closing tag does not match the open element |
| `ONM-SYNTAX-003` | an element repeats an attribute name |
| `ONM-SYNTAX-004` | an invalid character or entity reference appears in text or an attribute |
| `ONM-SYNTAX-005` | the source ends before an open element is closed |
| `ONM-SYNTAX-006` | a closing tag appears without an open element |
| `ONM-SYNTAX-007` | an XML declaration, processing instruction, or document type is unsupported |

Initial binding, resolution, and timing diagnostics are:

| Code | Meaning |
| --- | --- |
| `ONM-ID-001` | an authored ID is empty or contains ASCII whitespace |
| `ONM-ID-002` | an authored ID duplicates another ID in the same film |
| `ONM-STRUCT-001` | an element is outside the Gate-one vocabulary |
| `ONM-STRUCT-002` | the document has no top-level `film` element |
| `ONM-STRUCT-003` | the document has more than one top-level `film` element |
| `ONM-STRUCT-004` | a known element appears outside its legal parent |
| `ONM-STRUCT-005` | a film contains more than one `cues` container |
| `ONM-STRUCT-006` | authored text appears in a structural or empty element |
| `ONM-TIME-001` | an authored duration is invalid or outside the exact range |
| `ONM-TIME-002` | a shot has no media-derived or explicit duration source |
| `ONM-TIME-003` | explicit and media-derived shot durations compete |
| `ONM-TIME-004` | resolved content starts outside its owning shot |
| `ONM-TIME-005` | an exact time does not fit in the selected frame domain |
| `ONM-ASSET-001` | renderable media has no frozen artifact reference |
| `ONM-ASSET-002` | a media element references an artifact without its required track |
| `ONM-REF-001` | a well-formed overlay cue reference does not name a resolved cue |
| `ONM-REF-002` | a resolved cue is never referenced |
| `ONM-ATTR-001` | an element contains an unknown attribute |
| `ONM-ATTR-002` | an element is missing a required attribute |
| `ONM-ATTR-003` | an authored attribute value, including a malformed cue ID, is invalid |
| `ONM-ATTR-004` | two authored attributes define conflicting rules |

`ONM-REF-002` is a warning; the other initial binding, resolution, and timing diagnostics are errors.

The tokenizer stops after a fatal lexical error, so lexical recovery may produce one diagnostic. Onmark continues to aggregate independent nesting, binding, and semantic diagnostics whenever the remaining structure is trustworthy. At end of input, every still-open element receives one diagnostic whose primary span is its opening name and whose related span marks the end of the screenplay. A document type declaration produces one diagnostic even when the tokenizer exposes its internal subset as several tokens.

Good:

```text
ONM-TIME-004 “Buy now” starts at 13s, but its shot ends at 12s.
Help: extend shot “closing” or align the CTA to an earlier cue.
```

Bad:

```text
constraint graph node 17 is unsatisfied
```

## Presentation selection and props

`presentation.ts` is selected by the render command, either through
`--presentation` or same-directory discovery. The screenplay has no `presents`
attribute, `definePresentation` declaration, or typed props channel in the
current language. Its solved facts reach the browser only as the Rust-owned
`BrowserPlan` delivered through the runtime `Load(plan)` command.

This is a language boundary, not an undocumented implementation detail. A
future screenplay-selected presentation or props feature must define its
spelling, typed schema and defaults, canonical encoding, source-located
diagnostics, bundle/cache identity, and temporal-capability effect together;
it must also meet the admission rule below. Until then, static TypeScript
imports are presentation code, not screenplay props. The browser authoring
contract is specified separately in [the presentation contract](presentation-contract.md).

## Deferred capabilities

Free `begin/end/until` expressions, shots ending at cues, screenplay-selected
presentations or props, generated cues from media analysis or typed semantic
boundaries, negative offsets, general flex constraints, runtime branches,
speed ramps, reverse playback, audio-reactive behavior, cross-scene
persistence, content-aware transitions, and online media generation remain
outside Gate one until their semantics and generation reliability are tested. A
future typed semantic boundary must still produce a named event; it does not
reintroduce free timing attributes.

## Admission rule

New syntax must represent a real domain concept, compose orthogonally with existing semantics, preserve readability, avoid contradictory states, improve or maintain generation reliability in controlled tests, and support local actionable diagnostics. Paper elegance is insufficient.

Language evaluations are repository data rather than an informal result. A syntax proposal cannot change the Gate-one surface until its cases, prompts, grader, raw outputs, model settings, and comparison baseline are checked in and reproducible. CI may validate and rescore those frozen assets without calling a live model.

## Architecture boundary

The language ends at Timeline IR. It does not select Chromium, workers, partitions, tracks, codecs, or cache boundaries. Render planning may evolve without changing screenplay meaning; language spelling may evolve through explicit IR versioning and migration.
