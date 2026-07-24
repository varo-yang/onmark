# Onmark Language Specification

> Status: current screenplay language. Gate four admitted the present authored
> audio and subtitle surface; later completed gates changed presentation and
> execution behavior without adding screenplay spelling. Deferred language
> capabilities are listed explicitly.

## Purpose

Onmark does not redraw a track timeline with different tags. Authors express
content, order, ownership, and a small number of meaningful alignments. The
compiler derives absolute frames, maintains constraints, and explains failures.

The language is paired with the render architecture through one versioned
contract: Timeline IR.

## Axioms

1. Sequence is the default.
2. Content determines duration when media already has duration.
3. Explicit alignment uses named time events rather than track coordinates.
4. Local relationships stay local.
5. Structure should make common illegal states unrepresentable.
6. Remaining errors are source-located and actionable.
7. Execution concepts such as workers, cache keys, and render units never enter
   the screenplay.

## Core model

The current vocabulary is `film`, `cues`, `cue`, `scene`, `shot`, `video`,
`vo`, `music`, `sfx`, `title`, and `cta`. A film may contain at most one direct
`cues` child; that container owns only `cue` declarations and does not
participate in scene sequencing. A film may also own `music` that does not
participate in scene sequencing. Scenes own sequential shots. A renderable film
must contain at least one shot with a positive solved duration. A shot owns its
`video`, `vo`, `sfx`, `title`, and `cta` content. Titles and CTAs are overlays
and do not participate in sibling sequencing. `video` is the only current
visual media element. General audio uses the semantic `music` and `sfx`
elements; a generic `audio` element is not part of the vocabulary. Image and
other media elements remain deferred.
Structural binding retains `src` and other unparsed authored attributes for the
attribute/reference resolution phase rather than discarding them.

Illustrative syntax:

```html
<om-film>
  <om-music src="score.wav" gain="25%"></om-music>
  <om-cues>
    <om-cue id="offer" time="3s"></om-cue>
    <om-cue id="cta" time="7s"></om-cue>
  </om-cues>
  <om-scene id="sale">
    <om-shot id="hero">
      <video src="product.mp4"></video>
      <om-sfx src="reveal.wav" delay="250ms"></om-sfx>
      <om-title cue="offer">30% OFF</om-title>
      <om-cta cue="cta">Buy now</om-cta>
    </om-shot>
  </om-scene>
</om-film>
```

`cue="offer"` is the Gate-one spelling for aligning an overlay to a named cue.
Free-form `begin`, `end`, and `until` expressions are not part of the language.

## HTML syntax

A screenplay is one authored HTML document. Ordinary HTML owns layout and
presentation; the closed Onmark custom-element vocabulary owns screenplay
meaning. Its authored element namespace is `om-`; longer product, package, and
artifact names retain the full `onmark` spelling. The compiler tokenizes HTML
directly while preserving authored byte
spans and source order. It deliberately owns a strict authored-element stack
instead of adopting browser tree-recovery rules, so malformed presentation
markup cannot silently change semantic ownership. Every non-void authored
element therefore needs a matching end tag even where browser HTML would allow
that tag to be omitted.

HTML element and attribute names are ASCII case-insensitive and enter the syntax
tree in their normalized lowercase spelling. Comments are ignored. Text,
attributes, `<style>`, and `<script>` raw text retain authored spans; standard
HTML character references are decoded once. The standard `<!doctype html>` is
accepted, while non-HTML document types are rejected. A trailing solidus is
accepted only on HTML void elements; using `<om-shot />` reports malformed
syntax and keeps that non-void element open, matching browser interpretation.

Binding requires exactly one `<om-film>` semantic document root. It may be
authored directly in an HTML fragment or as a direct child of the standard
`html`/`body` document shell. Only that shell is transparent: nesting the film
inside a presentation container such as `div` does not change screenplay
ownership. Ordinary HTML siblings, document text, `head`, and presentation
descendants remain presentation-owned and do not enter the linked film. Root
cardinality, known `om-*` names, legal containment, required attributes,
IDs, and references are language semantics rather than tokenizer concerns.
Native descendants inside
`<om-title>`, `<om-cta>`, or `<om-vo>` contribute their text in
source order without becoming screenplay nodes.

Markup ingestion is bounded before semantic binding. One screenplay may contain
at most 8 MiB of UTF-8 source, 65,536 retained syntax items, and 32 simultaneously
open elements. Crossing one of these limits emits one stable resource-limit
diagnostic and stops syntax recovery; the compiler does not retain or recurse
through the rejected suffix.

## Time

Authored time values use the exact grammar `integer[.fraction](s|ms)` with no
whitespace or sign. Seconds admit at most nine fractional digits and
milliseconds at most six, so every accepted value has an exact unsigned
nanosecond representation. A shot's `duration` must be greater than zero; cue
times and delays may name zero. Frame units and floating-point approximations
are not part of the language.

The compiler maps exact nanosecond values onto a rational frame grid with
integer arithmetic. Every conversion names either floor or ceiling rounding at
its call site; no implicit cast or ambient default may choose a frame boundary.
Gate-one authored starts, delays, cue times, and durations select the first
frame boundary that is not earlier than the exact value (`Ceil`), so a positive
sub-frame value never silently becomes zero frames. `Floor` remains available
only for rules that explicitly require attribution to an earlier boundary.

A shot obtains duration from probed media, probed voice-over, or a restricted
explicit duration when content provides none. Multiple primary content sources
extend the shot to the longest source. Gate one does not allow a shot to end at
a cue. Overlay elements do not silently extend their shot.

The current language has two explicit relationships:

- `delay` on shot content, including `sfx`, is relative to the owning shot's start;
- a named cue aligns an overlay to an authored absolute film event.

An overlay starts at its resolved relationship, or at the owning shot's start
when none is authored, and remains active until that shot's exclusive end. Gate
one gives overlays no independent default duration. An overlay therefore cannot
extend its shot, and a resolved start outside the owning shot is an authored
timing error.

Gate-one cues use authored absolute film time. No other cue source is part of
the current language.

All resolutions preserve provenance in `TimingReason`, allowing the compiler to
explain not only where an element landed but why.

## Voice-over

`vo` pairs authored inscription with a frozen media artifact. Text supports
reading, review, subtitles, and editing; `src` supplies rendered audio and
measured duration. The reference is a screenplay-relative portable path: it uses
`/` separators and cannot be absolute, contain `..`, empty or `.` components,
backslashes, or a platform prefix. The referenced artifact must expose an audio
stream; otherwise solving reports `ONM-ASSET-002` at `src`. TTS belongs
upstream. The compiler is offline and deterministic, and content hashes detect
stale text/artifact pairs. Gate one materializes each solved voice-over into the
private render root and mixes it outside browser capture at its solved frame
interval. The presentation does not play, delay, or mix voice-over audio.

## General audio

`music` and `sfx` are distinct authored roles rather than a generic element
with a free-form kind. This keeps illegal role/parent combinations out of the
language and preserves narrative `vo` as a separate concept.

A film may contain any number of direct `music` children. Music begins at the
film's zero frame, uses the referenced audio stream's measured duration, and
may cross scene, shot, and Render Unit boundaries. It never extends the film:
a source longer than the solved film is clipped at the film's exclusive end; a
shorter source ends naturally. Music has no authored delay.

A shot may contain any number of direct `sfx` children. A sound effect begins
at the shot start plus its optional local `delay`, and its measured source
duration determines its exclusive end. It does not determine or extend shot
duration. An effect whose start or end lies outside its owning shot is an
authored timing error rather than a silently clipped sound.

Both elements require a screenplay-relative `src` with the same portability
rules as voice-over. Their optional `gain` uses the exact grammar `integer%`,
from `0%` through `100%` inclusive, and defaults to `100%`. Gain is a linear
amplitude ratio, not decibels. The referenced artifact must contain an audio
stream. Mixing and muxing remain native execution concerns; the browser does
not play these elements.

## IDs and references

Explicit IDs, including cue IDs, are non-empty, case-sensitive, and globally
unique within one film. In keeping with the HTML `id` constraint, they may not
contain ASCII whitespace. Non-ASCII characters are preserved exactly; the
compiler does not silently normalize authored IDs. A later typed `CueId` or
`EventRef` distinguishes cue references without creating a second declaration
namespace.

## Attributes and resolution

Structural binding is followed by attribute and reference resolution. `film`,
`cues`, and `scene` admit no non-ID attributes. `cue` requires `id` and `time`.
`shot` admits optional `duration`. `video` and `vo` admit optional `src` and
`delay`; `music` requires `src` and admits optional `gain`; `sfx` requires
`src` and admits optional `delay` and `gain`; `title` and `cta` admit optional
`cue` or `delay`. `cue` and `delay` cannot appear together on one overlay
because they define competing start rules. Missing `src` on `video` or `vo`
remains valid for static analysis; `music` and `sfx` require it during
resolution. An authored empty `src` is always invalid. Unknown attributes are
errors.

## Diagnostics

Diagnostics contain a stable code, severity, source span, message, actionable
help, and related spans. They use screenplay vocabulary rather than solver
internals and aggregate independent authored errors when safe.

Initial markup diagnostics are:

| Code             | Meaning                                                                     |
| ---------------- | --------------------------------------------------------------------------- |
| `ONM-SYNTAX-001` | malformed markup that cannot produce another trustworthy token              |
| `ONM-SYNTAX-002` | a closing tag does not match the open element                               |
| `ONM-SYNTAX-003` | an element repeats an attribute name                                        |
| `ONM-SYNTAX-004` | an invalid character or entity reference appears in text or an attribute    |
| `ONM-SYNTAX-005` | the source ends before an open element is closed                            |
| `ONM-SYNTAX-006` | a closing tag appears without an open element                               |
| `ONM-SYNTAX-007` | a non-HTML document type is unsupported                               |
| `ONM-SYNTAX-008` | screenplay markup exceeds a bounded syntax resource                         |

Initial binding, resolution, and timing diagnostics are:

| Code             | Meaning                                                               |
| ---------------- | --------------------------------------------------------------------- |
| `ONM-ID-001`     | an authored ID is empty or contains ASCII whitespace                  |
| `ONM-ID-002`     | an authored ID duplicates another ID in the same film                 |
| `ONM-STRUCT-001` | an element is outside the current screenplay vocabulary               |
| `ONM-STRUCT-002` | the document has no semantic `film` root                             |
| `ONM-STRUCT-003` | the document has more than one semantic `film` root                  |
| `ONM-STRUCT-004` | a known element appears outside its legal parent                      |
| `ONM-STRUCT-005` | a film contains more than one `cues` container                        |
| `ONM-STRUCT-006` | authored text appears in a structural or empty element                |
| `ONM-TIME-001`   | an authored duration is invalid or outside the exact range            |
| `ONM-TIME-002`   | a shot has no media-derived or explicit duration source               |
| `ONM-TIME-003`   | explicit and media-derived shot durations compete                     |
| `ONM-TIME-004`   | resolved shot content starts or ends outside its owning shot          |
| `ONM-TIME-005`   | an exact time does not fit in the selected frame domain               |
| `ONM-TIME-006`   | a film has no shot with a positive solved duration                    |
| `ONM-ASSET-001`  | renderable media has no frozen artifact reference                     |
| `ONM-ASSET-002`  | a media element references an artifact without its required track     |
| `ONM-REF-001`    | a well-formed overlay cue reference does not name a resolved cue      |
| `ONM-REF-002`    | a resolved cue is never referenced                                    |
| `ONM-ATTR-001`   | an element contains an unknown attribute                              |
| `ONM-ATTR-002`   | an element is missing a required attribute                            |
| `ONM-ATTR-003`   | an authored attribute value, including a malformed cue ID, is invalid |
| `ONM-ATTR-004`   | two authored attributes define conflicting rules                      |
| `ONM-CAPTION-001` | an imported subtitle file violates its selected format grammar       |
| `ONM-CAPTION-002` | an imported subtitle file uses unsupported presentation semantics    |
| `ONM-CAPTION-003` | an imported subtitle file exceeds a bounded ingestion limit          |

`ONM-REF-002` is a warning; the other initial binding, resolution, and timing
diagnostics are errors.

The tokenizer stops after a fatal lexical error, so lexical recovery may produce
one diagnostic. Onmark continues to aggregate independent nesting, binding, and
semantic diagnostics whenever the remaining structure is trustworthy. At end of
input, every still-open element receives one diagnostic whose primary span is
its opening name and whose related span marks the end of the screenplay. A
document type declaration produces one diagnostic even when the tokenizer
exposes its internal subset as several tokens.

Good:

```text
ONM-TIME-004 “Buy now” starts at 13s, but its shot ends at 12s.
Help: extend shot “closing” or align the CTA to an earlier cue.
```

Bad:

```text
constraint graph node 17 is unsatisfied
```

## Presentation and props

The authored HTML is also the presentation. Onmark binds solved video, title,
CTA, and caption facts onto the existing semantic elements without replacing
ordinary DOM, classes, nested markup, or inline styles. An optional
`<script type="module" data-om-motion>` exports one `motion` value and may
import admitted browser adapters such as `onmark/motion/gsap`. No other script
element is admitted by the bundling boundary.

There is no same-stem CSS or motion convention, `--presentation` escape hatch,
`presents` attribute, `definePresentation` declaration, or separate typed props
channel. Solved facts reach the document only as the Rust-owned `BrowserPlan`
delivered through `Load(plan)`.

The Browser Plan also retains film, scene, shot, and content ownership. The
compiler assigns every projected node a stable identity and carries only the
admitted authored ID, semantic role, text, ownership, and solved interval. This
is not a general screenplay props channel or a second presentation timeline.

This is a language boundary, not an undocumented implementation detail. A future
screenplay-selected presentation or props feature must define its spelling,
typed schema and defaults, canonical encoding, source-located diagnostics,
bundle/cache identity, and temporal-capability effect together; it must also
meet the admission rule below. Until then, stylesheet rules and static
TypeScript imports are presentation code, not screenplay props. The browser
authoring contract is specified separately in
[the presentation contract](presentation-contract.md).

## Deferred capabilities

Free `begin/end/until` expressions, shots ending at cues, screenplay-selected
presentations or props, generated cues from media analysis or typed semantic
boundaries, negative offsets, general flex constraints, runtime branches, speed
ramps, reverse playback, audio-reactive behavior, cross-scene persistence,
content-aware transitions, and online media generation remain outside Gate one
until their semantics and generation reliability are tested. A future typed
semantic boundary must still produce a named event; it does not reintroduce free
timing attributes.

## Admission rule

New syntax must represent a real domain concept, compose orthogonally with
existing semantics, preserve readability, avoid contradictory states, improve or
maintain generation reliability in controlled tests, and support local
actionable diagnostics. Paper elegance is insufficient.

Language evaluations are repository data rather than an informal result. A
syntax proposal cannot change the Gate-one surface until its cases, prompts,
grader, raw outputs, model settings, and comparison baseline are checked in and
reproducible. CI may validate and rescore those frozen assets without calling a
live model.

## Architecture boundary

The language ends at Timeline IR. It does not select Chromium, workers,
partitions, tracks, codecs, or cache boundaries. Render planning may evolve
without changing screenplay meaning; language spelling may evolve through
explicit IR versioning and migration.
