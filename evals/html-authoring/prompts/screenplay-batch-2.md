Do not use tools or inspect the filesystem; this prompt is the complete spec.
Modify or repair five Onmark projects. Return each complete resulting project
as the JSON object required by the output schema.

Current Onmark authoring profile:
- `film.onmark` is strict XML-like screenplay markup:
  `<film><scene><shot>...</shot></scene></film>`.
- A shot may contain `<video src="..."/>`, `<title>text</title>`, and
  `<cta>text</cta>`. Source media determines duration.
- A `<cues>` block is a direct child of `<film>`, never of a scene or shot.
  Cues are `<cues><cue id="name" time="2s"/></cues>` and overlays refer with
  `cue`. Shot-local placement uses `delay`. Never author `start`, `end`,
  `until`, frame numbers, or timeline tracks.
- `film.css` styles generated semantic classes. `film.motion.ts` exports
  `motion = gsapMotion(...)`.
- Preserve every byte of unaffected authored behavior as closely as practical.
  Do not replace working code wholesale.
- Emit no prose or Markdown fences.

Cases:
1. `edit-copy`: Starting files:
   `film.onmark` =
   `<film><scene><shot id="hero"><video src="hero.mp4"/><title>Old message</title></shot></scene></film>`
   `film.css` =
   `.onmark-title { color: white; font-size: 80px; }\n#hero { background: #111; }\n`
   Change only the title to `Compile what you mean.` and its color to
   `#d7ff43`. Preserve the font size and hero background.
2. `append-shot`: Starting `film.onmark` =
   `<film><scene><shot id="one"><video src="one.mp4"/><title>One</title></shot><shot id="two"><video src="two.mp4"/><title>Two</title></shot></scene></film>`
   Starting `film.css` =
   `#one { background: red; }\n#two { background: blue; }\n`
   Append a third shot with ID `three`, video `three.mp4`, title `Three`, and
   green background. Preserve the first two shots and rules.
3. `add-cued-cta`: Starting `film.onmark` =
   `<film><scene><shot><video src="offer.mp4"/><title>Offer</title></shot></scene></film>`
   Add cue `buy` at 1500ms and CTA `Buy now` bound to it. Do not change the
   video or title and do not add explicit shot duration.
4. `repair-markup`: Starting `film.onmark` =
   `<film><scene><shot><video src="clip.mp4"><title>Exact</scene></film>`
   Repair only the malformed markup. Preserve the one video and title and do not
   invent timing or wrapper elements.
5. `reject-coordinate-edit`: Starting `film.onmark` =
   `<film><scene><shot><video src="clip.mp4"/><title>Later</title></shot></scene></film>`
   The author asks: “make the title start at absolute 3s.” Express this with one
   cue named `title-at-three` at 3s and bind the title to it. Do not author a
   `start`, `begin`, `end`, `until`, frame number, or track.

Return every case exactly once and preserve the case IDs.
