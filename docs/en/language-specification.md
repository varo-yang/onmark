# Onmark Language Specification

> Status: current screenplay language through the active Gate eight and
> distributed incremental rendering. Gate eight admits only spelling backed by
> checked-in generation evidence. Deferred language capabilities are listed
> explicitly.

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

The current vocabulary is `film`, `fields`, `field`, `cues`, `cue`, `captions`,
`scene`, `shot`, `transition`, `video`, `vo`, `music`, `sfx`, `title`, and
`cta`. A film may contain at most one direct `fields` child and one direct
`cues` child.
`fields` declares presentation-only typed input; `cues` declares absolute time
events. Neither participates in scene sequencing. Repeatable film-level
`captions` declarations name external caption tracks and also remain outside
scene sequencing. A film may own `music` with the same structural role. Scenes
own sequential shots and explicit transition boundaries. A renderable film
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
  <om-captions
    id="en"
    src="captions/en.vtt"
    lang="en"
  ></om-captions>
  <om-scene id="sale">
    <om-shot id="hero">
      <video src="product.mp4"></video>
      <om-sfx src="reveal.wav" delay="250ms"></om-sfx>
      <om-title cue="offer">30% OFF</om-title>
    </om-shot>
    <om-transition id="reveal" duration="500ms"></om-transition>
    <om-shot id="closing" duration="3s">
      <om-cta cue="cta">Buy now</om-cta>
    </om-shot>
  </om-scene>
</om-film>
```

`cue="offer"` aligns an overlay to a named cue.
Free-form `begin`, `end`, and `until` expressions are not part of the language.

An HTML-head declaration such as
`<meta name="onmark:visual-capability" content="separableBackdrop">` belongs to
the presentation build contract, not this screenplay vocabulary. It can select
an already-admitted pixel-ownership path but cannot change structure, timing,
Timeline IR, or render dependencies.

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
nanosecond representation. Shot and transition durations must be greater than
zero; cue times and delays may name zero. Frame units and floating-point
approximations are not part of the language.

Video may select a source-local half-open interval with
`trim="start..end"`. Either bound may be omitted, but not both: an omitted
start means source zero and an omitted end means the probed source end. Both
bounds use the duration grammar above and must satisfy
`start < end <= source duration`.

Video may play its selected source interval at an exact positive rate with
`speed="integer[.fraction]x"`. The fraction admits at most six digits and the
default is `1x`. `plays="positive integer"` repeats the complete selected
interval that many times in total and defaults to one play. `hold-last` uses a
positive duration to retain the selected interval's final frame after all
plays. The compiler derives local duration as
`selected duration / speed * plays + hold-last`.

None of these attributes expresses a film-timeline coordinate. They apply to
the selected visual stream only; authored `music`, `sfx`, and `vo` remain
separate audio roles and do not inherit a video's source treatment.

The compiler maps exact nanosecond values onto a rational frame grid with
integer arithmetic. Every conversion names either floor or ceiling rounding at
its call site; no implicit cast or ambient default may choose a frame boundary.
Authored starts, delays, cue times, and durations select the first
frame boundary that is not earlier than the exact value (`Ceil`), so a positive
sub-frame value never silently becomes zero frames. `Floor` remains available
only for rules that explicitly require attribution to an earlier boundary.

A shot obtains duration from probed media, probed voice-over, or a restricted
explicit duration when content provides none. Multiple primary content sources
extend the shot to the longest source. The current language does not allow a shot to end at
a cue. Overlay elements do not silently extend their shot.

The current language has two explicit relationships:

- `delay` on shot content, including `sfx`, is relative to the owning shot's start;
- a named cue aligns an overlay to an authored absolute film event.

An overlay starts at its resolved relationship, or at the owning shot's start
when none is authored, and remains active until that shot's exclusive end. The
current language gives overlays no independent default duration. An overlay therefore cannot
extend its shot, and a resolved start outside the owning shot is an authored
timing error.

Current cues use authored absolute film time. No other cue source is part of
the current language.

All resolutions preserve provenance in `TimingReason`, allowing the compiler to
explain not only where an element landed but why.

## Transition boundaries

`<om-transition duration="500ms"></om-transition>` may appear only between two
adjacent shots in one scene. It is a relation between those shots, not a
renderable child, a third shot, or a free timeline coordinate. The compiler
overlaps the incoming shot with the outgoing shot's tail for the exact authored
duration. Consequently, the scene and film are shorter by that overlap.

The duration must fit inside both adjacent shots. Two transitions around one
middle shot must not consume overlapping portions of that shot. Violations are
authored timing errors; the compiler never clips or silently shortens a
transition. Cross-scene transitions and transitions whose window is inferred
from presentation code are not part of the language.

The overlap is visual; it does not silently choose a narration-mixing policy.
When the solved voice-over intervals of the adjacent shots intersect inside the
transition, the compiler reports `ONM-AUDIO-001` at that boundary. The author
must shorten the transition or delay the incoming voice-over. Onmark does not
trim speech or invent a crossfade.

The empty element's `id`, `class`, and ordinary presentation attributes remain
available to CSS and motion code. `duration` is compiler-owned and is removed
from the browser projection. The language does not prescribe an effect enum or
a built-in visual template. A transition motion handler receives the solved
overlap and both adjacent shot elements, then realizes that fact with
exact-frame browser effects.

## Voice-over

`vo` pairs authored inscription with a frozen media artifact. Text supports
reading, review, subtitles, and editing; `src` supplies rendered audio and
measured duration. The reference is a screenplay-relative portable path: it uses
`/` separators and cannot be absolute, contain `..`, empty or `.` components,
backslashes, or a platform prefix. The referenced artifact must expose an audio
stream; otherwise solving reports `ONM-ASSET-002` at `src`. TTS belongs
upstream. The compiler is offline and deterministic, and content hashes detect
stale text/artifact pairs. The renderer materializes each solved voice-over into the
private render root and mixes it outside browser capture at its solved frame
interval. The presentation does not play, delay, or mix voice-over audio.
Voice-over may use the exact `fade-in` and `fade-out` envelope described below.

## Caption tracks

Each direct `<om-captions>` child declares one external caption track. The
element is repeatable and requires `id`, `src`, and `lang`:

```html
<om-captions id="en" src="captions/en.vtt" lang="en"></om-captions>
<om-captions id="zh" src="captions/zh.srt" lang="zh-CN"></om-captions>
```

`id` uses the film-wide ID namespace. `src` is a screenplay-relative portable
path with the same rules as authored media. `lang` is preserved for HTML
language metadata and admits non-empty ASCII-alphanumeric subtags separated by
hyphens. SRT, WebVTT, and the admitted ASS subset are parsed under fixed input,
cue-count, text, and diagnostic bounds. File syntax and unsupported
presentation semantics remain source-located caption diagnostics; missing or
unreadable files remain typed infrastructure errors.

All declared tracks are selected in authored order by default. Authoring
commands may choose an ordered subset with `--captions en,zh`; an unknown or
repeated selection is rejected. Multiple selected tracks are burned in
together. The compiler projects every cue onto the exact film frame grid,
clips it to the film interval, and retains track identity and language in
Timeline IR and Browser Plan.

The browser creates one film-level `om-caption` element per active cue, sets
`data-track` to the declaration ID and `lang` to its language, and leaves
layout and styling to authored CSS. The source track does not supply a second
clock or hidden layout engine. Soft-subtitle muxing, automatic translation,
word-level karaoke, and unrestricted ASS styling are not part of the current
language.

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

`vo`, `music`, and `sfx` admit optional `fade-in` and `fade-out` durations.
The fade-in begins at the solved placement start; the fade-out ends at its
exclusive end. Both are linear-amplitude ramps between silence and the authored
or default gain. Fades change amplitude only: they never move, shorten, or
extend the placement. Their sum must fit the actual solved placement, including
music clipped at the film end, or solving reports `ONM-AUDIO-002`. Rust converts
the durations to exact frame facts and then to integer output-sample
boundaries; browser code does not own an audio envelope.

The spelling was admitted by the checked-in `audio-envelope-syntax`
experiment. Flat `fade-in`/`fade-out` attributes and an `om-envelope` child both
retained 20/20 semantic accuracy across two independent repetitions. The flat
attributes used 4,914 authored bytes versus 5,868 and avoided mixing a
treatment child into voice-over inscription, so the child spelling is not part
of the language.

## IDs and references

Explicit IDs, including cue IDs and caption-track IDs, are non-empty,
case-sensitive, and globally unique within one film. In keeping with the HTML
`id` constraint, they may not contain ASCII whitespace. Non-ASCII characters
are preserved exactly; the compiler does not silently normalize authored IDs.
Later typed identities distinguish cue and caption references without creating
another declaration namespace.

## Attributes and resolution

Structural binding is followed by attribute and reference resolution. `film`,
`cues`, and `scene` admit no non-ID compiler attributes. `cue` requires `id`
and `time`; `captions` requires `id`, `src`, and `lang`. `shot` admits optional
`duration`; `transition` requires `duration`. `video` admits optional `src`,
`delay`, `trim`, `speed`, `plays`, and `hold-last`; `vo` admits optional `src`,
`delay`, `fade-in`, and `fade-out`;
`music` requires `src` and admits optional `gain`, `fade-in`, and `fade-out`;
`sfx` requires `src` and admits optional `delay`, `gain`, `fade-in`, and
`fade-out`; `title` and `cta` admit optional `cue` or `delay`. `cue` and
`delay` cannot appear together on one overlay because they define competing
start rules. Missing `src` on `video` or `vo` remains valid for static
analysis; `music` and `sfx` require it during resolution. An authored empty
`src` is always invalid. Unknown compiler attributes are errors; the closed
set of global HTML presentation attributes remains presentation-owned.

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
| `ONM-STRUCT-007` | a transition does not appear between two adjacent shots               |
| `ONM-TIME-001`   | an authored duration is invalid or outside the exact range            |
| `ONM-TIME-002`   | a shot has no media-derived or explicit duration source               |
| `ONM-TIME-003`   | explicit and media-derived shot durations compete                     |
| `ONM-TIME-004`   | resolved shot content starts or ends outside its owning shot          |
| `ONM-TIME-005`   | an exact time does not fit in the selected frame domain               |
| `ONM-TIME-006`   | a film has no shot with a positive solved duration                    |
| `ONM-TIME-007`   | a selected video source interval lies outside its frozen artifact     |
| `ONM-TIME-008`   | a transition cannot fit inside both adjacent shot intervals           |
| `ONM-AUDIO-001`  | a transition makes adjacent solved voice-over intervals overlap        |
| `ONM-AUDIO-002`  | audio fades overlap or exceed their solved placement                    |
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
| `ONM-VARIANT-001` | a field declaration has an invalid name, kind, default, or shape      |
| `ONM-VARIANT-002` | a film declares the same field name more than once                    |
| `ONM-VARIANT-003` | a presentation binding names a field that is not declared             |
| `ONM-VARIANT-004` | a presentation sink is incompatible with the bound field kind         |
| `ONM-VARIANT-005` | authored fallback markup does not equal the field default             |
| `ONM-VARIANT-006` | an external variant document is malformed, nested, or too large       |
| `ONM-VARIANT-007` | an external variant document contains an undeclared field             |
| `ONM-VARIANT-008` | an external value has the wrong kind, spelling, or bounded value      |
| `ONM-VARIANT-009` | a declared field has no presentation binding                          |

`ONM-REF-002` and `ONM-VARIANT-009` are warnings; the other initial binding,
resolution, timing, and variant diagnostics are errors.

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
`presents` attribute, `definePresentation` declaration, arbitrary props object,
source placeholder substitution, or module-owned input schema. Solved facts and
the closed typed values defined below reach the document only as the Rust-owned
`BrowserPlan` delivered through `Load(plan)`.

The Browser Plan also retains film, scene, shot, and content ownership. The
compiler assigns every projected node a dense unit-local identity and carries
only the admitted authored ID, semantic role, text, ownership, and solved
interval. Authored IDs provide semantic identity across projections. This is
not a general screenplay props channel or a second presentation timeline.

Compiler attributes are not presentation props. The browser projection removes
cue declarations, native audio elements, and the `src`, `duration`, `delay`,
`cue`, `gain`, `trim`, `speed`, `plays`, and `hold-last` spellings after
compilation. A transition marker remains as an empty presentation target only
in a projection that contains both adjacent shots. Presentation code consumes
solved Browser Plan facts where applicable; it must not derive timing from
authored compiler spelling. IDs, classes, ordinary HTML attributes, nested
markup, inline styles, and authored overlay text remain presentation inputs.

An ordinary `<img src>` is presentation markup, not a screenplay image element
or duration source. Its local AVIF, GIF, JPEG, PNG, SVG, or WebP bytes are
frozen into the browser bundle and must decode before capture. Remote URLs and
`srcset` are rejected. An image nested inside one shot belongs only to that
shot's projected region; an image outside all shots remains in every projected
region that retains its presentation wrapper.

For independent Render Graph regions, the browser document contains only the
selected shot and its owning scene and film shells. Presentation-global style,
motion, and imported resources remain inputs to every region. Authored IDs
survive this projection; protocol node IDs are dense unit-local binding keys.
Code must not use the presence of semantic siblings as an implicit cross-region
communication channel. A style inside one shot belongs to that region; a style
inside a scene but outside its shots belongs to every region in that scene; a
film- or document-level style outside all scenes belongs to every region.

### Canonical typed variants

A film may declare one closed presentation-input schema:

```html
<om-fields>
  <om-field name="headline" type="text" default="Summer edit"></om-field>
  <om-field name="accent" type="color" default="#ff4d36"></om-field>
  <om-field name="progress" type="integer" default="72"></om-field>
  <om-field name="featured" type="boolean" default="false"></om-field>
</om-fields>
```

`om-fields` is an optional direct child of `om-film`, appears at most once,
contains only `om-field`, and is removed from the browser projection.
`om-field` is empty and accepts exactly `name`, `type`, and `default`. A film
declares at most 256 fields. A field name is a lower-camel ASCII identifier
matching `[a-z][A-Za-z0-9]{0,63}` and is unique within the film.

The four admitted kinds are deliberately closed:

- `text` is decoded Unicode text, preserved byte-for-byte after HTML character
  reference decoding and limited to 16 KiB of UTF-8;
- `integer` is a canonical base-ten integer in JavaScript's exact signed integer
  range, with no leading plus or redundant leading zero;
- `boolean` is exactly `true` or `false`;
- `color` is exactly lowercase `#rrggbb` or `#rrggbbaa`.

Defaults use those canonical spellings. The compiler parses them once into
typed values; presentation code never reparses a JSON string or an authored
attribute.

Presentation markup binds fields only through these literal sinks:

```html
<om-shot
  data-om-css="accent progress"
  style="--accent:#ff4d36;--progress:72">
  <om-title data-om-text="headline">Summer edit</om-title>
  <span data-om-show="featured" hidden>Featured</span>
</om-shot>
```

- `data-om-text` names one `text` field. Its element contains direct text only,
  and that text equals the declared default.
- `data-om-css` names one or more `color` or `integer` fields separated by ASCII
  whitespace. The same element initializes each `--<field>` inline custom
  property to the canonical default.
- `data-om-show` names one `boolean` field. The element has `hidden` exactly when
  the default is `false`.

Bindings must be descendants of the semantic film. They are forbidden inside
`om-fields`, cue declarations, native audio declarations, `style`, and
`script`. A field with no binding is a warning. Unknown fields, duplicate
bindings on one sink, incompatible field kinds, and fallback markup that does
not equal the declared default are errors. These rules keep the authored
document truthful and directly viewable before Onmark runs.

One optional external variant document is a flat JSON object whose keys are
declared field names and whose JSON scalar types match the schema. Missing keys
use declared defaults. Duplicate or unknown keys, nested values, non-canonical
numbers, and values outside the bounds above are errors. The document is limited
to 1 MiB. Resolution produces one immutable, name-sorted canonical value vector;
source JSON spelling does not enter rendering identity.

Typed variants may change only the literal presentation sinks above. They cannot
change element structure, timing, cues, media sources, treatment, frame rate,
dimensions, output profile, capabilities, motion modules, or resource imports.
They are immutable for one render. They do not create a mutable global, URL
parameter, source rewrite, template language, or second scheduler.

Field bindings follow presentation ownership. A binding in a shot affects that
shot's Render Graph region; a binding in a scene shell affects every retained
region in that scene; a binding in the film shell affects every region. A
transition binding affects only a region that retains both adjacent shots.
Timeline IR records these scopes. Each Browser Plan carries only the canonical
values required by its region, so changing a field invalidates exactly the
regions that depend on it. The immutable bundle remains reusable.

This is the only admitted screenplay-to-presentation value channel. Stylesheet
rules and static TypeScript imports remain presentation code, not variant
values. The browser authoring contract is specified separately in
[the presentation contract](presentation-contract.md).

## Deferred capabilities

Free `begin/end/until` expressions, shots ending at cues, arbitrary or
module-owned props, generated cues from media analysis or typed semantic
boundaries, negative offsets, general flex constraints, runtime branches, speed
ramps, reverse playback, audio-reactive behavior, cross-scene persistence,
inferred or cross-scene transition windows, and online media generation remain
unsupported until their semantics and generation reliability are tested. A
future typed semantic boundary must still produce a named event; it does not
reintroduce free timing attributes.

## Admission rule

New syntax must represent a real domain concept, compose orthogonally with
existing semantics, preserve readability, avoid contradictory states, improve or
maintain generation reliability in controlled tests, and support local
actionable diagnostics. Paper elegance is insufficient.

Language evaluations are repository data rather than an informal result. A
syntax proposal cannot change the current language surface until its cases, prompts,
grader, raw outputs, model settings, and comparison baseline are checked in and
reproducible. CI may validate and rescore those frozen assets without calling a
live model.

## Architecture boundary

The language ends at Timeline IR. It does not select Chromium, workers,
partitions, tracks, codecs, or cache boundaries. Render planning may evolve
without changing screenplay meaning; language spelling may evolve through
explicit IR versioning and migration.
