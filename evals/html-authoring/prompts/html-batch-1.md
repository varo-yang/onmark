Do not use tools or inspect the filesystem; this prompt is the complete spec.
Generate five small Onmark video projects. Return only the JSON object required
by the output schema.

Candidate Onmark HTML profile:
- Emit exactly one `film.html` per case.
- The semantic root is `<om-film>`, containing `<om-scene>` and
  `<om-shot>`.
- A shot may contain native `<video src="..."></video>`,
  `<om-title>...</om-title>`, and `<om-cta>...</om-cta>`.
  Source media determines shot duration.
- Film-absolute cues use
  `<om-cues><om-cue id="name" time="2s"></om-cue></om-cues>`;
  an overlay may use `cue="name"`. Shot-local placement uses `delay`.
- Never author `start`, `end`, `until`, frame numbers, or timeline tracks.
- Native `class`, `id`, `style`, `<span>`, and non-semantic decorative DOM are
  preserved in the browser. Only onmark elements, native video, and their
  semantic attributes contribute compiler facts.
- Include CSS in one `<style>` child of `<om-film>`.
- Optional motion goes in one
  `<script type="module" data-om-motion>` child. It may import
  `gsapMotion` from `onmark/motion/gsap` and export `motion`. A `shot`, `title`,
  or `callToAction` handler receives `{durationSeconds, element, timeline}`.
  Selector handlers may be placed under `selectors`.
- Non-void HTML elements must have explicit closing tags. Do not self-close
  `<video>` or Onmark elements.
- Emit no prose or Markdown fences.

Cases:
1. `simple-hero`: one shot with `media/hero.mp4` and title `Compile intent.`.
   Style a large white lower-left title over a darkened full-bleed video. Animate
   the title from opacity 0 and y 36 over 0.45 seconds.
2. `three-beats`: three shots using `media/a.mp4`, `media/b.mp4`, and
   `media/c.mp4`, with IDs `story`, `compile`, and `render`, and titles `Story`,
   `Compile`, and `Render`. Give each shot a distinct background accent. Animate
   every shot with a smooth clip-path reveal lasting 0.55 seconds. Do not switch
   on element IDs.
3. `cue-cta`: declare cue `offer` at 2s. One shot contains `media/offer.mp4`,
   title `30% off`, and CTA `Buy now` at that cue. Style the CTA as a rounded
   red button and animate it from scale 0.8 and opacity 0.
4. `nested-emphasis`: one shot contains `media/native.mp4`. Display the title
   `Write native. Render exact.` with only `native.` colored lime via a real
   nested `<span class="accent">`; preserve that authored DOM identity for CSS
   and motion. Animate the nested accent separately from the rest of the title.
5. `decorative-layer`: one shot contains `media/grid.mp4`, title `No tracks`,
   and a real decorative `<div class="grid" aria-hidden="true"></div>` behind
   the title. Style the grid with two CSS linear gradients and animate its
   opacity independently.

Return every case exactly once and preserve the case IDs.
