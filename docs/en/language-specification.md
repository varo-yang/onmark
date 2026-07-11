# Onmark Language Specification

> Status: semantic draft. Stable semantics are separated from provisional surface spelling.

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

The initial vocabulary is `film`, `scene`, `shot`, `vo`, `title`, `cta`, and `cue`. Scenes and shots are sequential containers. Titles and CTAs are overlays owned by a shot and do not participate in sibling sequencing.

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

The semantic commitment is alignment to a named cue. The final attribute spelling remains provisional until generation experiments validate it. Free-form `begin/end` expressions are not part of the default language.

## Time

A shot obtains duration from probed media, probed voice-over, a restricted explicit duration when content provides none, or an ending event. Multiple primary content sources extend the shot to the longest source. Overlay elements do not silently extend their shot.

Two explicit relationships exist initially:

- `delay` is relative to the owning shot's start;
- a named cue aligns an element to an authored event.

Initial cues use absolute film time. Later cue sources may include beats, media markers, semantic node boundaries, or a frozen upstream event table while sharing the same internal `EventRef` model.

All resolutions preserve provenance in `TimingReason`, allowing the compiler to explain not only where an element landed but why.

## Voice-over

`vo` pairs authored inscription with a frozen media artifact. Text supports reading, review, subtitles, and editing; `src` supplies rendered audio and measured duration. TTS belongs upstream. The compiler is offline and deterministic, and content hashes detect stale text/artifact pairs.

## IDs and references

Explicit IDs are non-empty, case-sensitive, and unique within one film. In keeping with the HTML `id` constraint, they may not contain ASCII whitespace. Non-ASCII characters are preserved exactly; the compiler does not silently normalize authored IDs.

## Diagnostics

Diagnostics contain a stable code, severity, source span, message, actionable help, and related spans. They use screenplay vocabulary rather than solver internals and aggregate independent authored errors when safe.

Good:

```text
ONM-TIME-004 “Buy now” starts at 13s, but its shot ends at 12s.
Help: extend shot “closing” or align the CTA to an earlier cue.
```

Bad:

```text
constraint graph node 17 is unsatisfied
```

## Deferred capabilities

Free `begin/end` expressions, negative offsets, general flex constraints, runtime branches, speed ramps, reverse playback, audio-reactive behavior, cross-scene persistence, content-aware transitions, and online media generation remain outside v0 until their semantics and generation reliability are tested.

## Admission rule

New syntax must represent a real domain concept, compose orthogonally with existing semantics, preserve readability, avoid contradictory states, improve or maintain generation reliability in controlled tests, and support local actionable diagnostics. Paper elegance is insufficient.

## Architecture boundary

The language ends at Timeline IR. It does not select Chromium, workers, partitions, tracks, codecs, or cache boundaries. Render planning may evolve without changing screenplay meaning; language spelling may evolve through explicit IR versioning and migration.
