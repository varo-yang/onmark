Do not use tools or inspect the filesystem; this prompt is the complete spec.
Modify or repair five Onmark projects. Return each complete resulting project
as the JSON object required by the output schema.

Candidate Onmark HTML profile:
- Emit exactly one `film.html` per case.
- Semantic structure uses `<om-film>`, `<om-scene>`, and
  `<om-shot>`. Content uses native `<video></video>`,
  `<om-title>...</om-title>`, and `<om-cta>...</om-cta>`.
- `<om-cues>` is a direct child of `<om-film>`, never of a scene or
  shot. It contains
  `<om-cue id="name" time="2s"></om-cue>`.
- Native HTML classes and nested DOM are preserved. CSS lives in `<style>`;
  optional motion lives in `<script type="module" data-om-motion>`.
- Non-void elements require explicit closing tags.
- Never author `start`, `end`, `until`, frame numbers, or timeline tracks.
- Preserve every byte of unaffected authored behavior as closely as practical.
  Do not replace working code wholesale.
- Emit no prose or Markdown fences.

Cases:
1. `edit-copy`: Starting `film.html` =
   `<om-film><style>.headline { color: white; font-size: 80px; }\n#hero { background: #111; }</style><om-scene><om-shot id="hero"><video src="hero.mp4"></video><om-title class="headline">Old message</om-title></om-shot></om-scene></om-film>`
   Change only the title to `Compile what you mean.` and its color to
   `#d7ff43`. Preserve the class, font size, and hero background.
2. `append-shot`: Starting `film.html` =
   `<om-film><style>#one { background: red; }\n#two { background: blue; }</style><om-scene><om-shot id="one"><video src="one.mp4"></video><om-title>One</om-title></om-shot><om-shot id="two"><video src="two.mp4"></video><om-title>Two</om-title></om-shot></om-scene></om-film>`
   Append a third shot with ID `three`, video `three.mp4`, title `Three`, and
   green background. Preserve the first two shots and rules.
3. `add-cued-cta`: Starting `film.html` =
   `<om-film><om-scene><om-shot><video src="offer.mp4"></video><om-title>Offer</om-title></om-shot></om-scene></om-film>`
   Add cue `buy` at 1500ms and CTA `Buy now` bound to it. Do not change the
   video or title and do not add explicit shot duration.
4. `repair-markup`: Starting `film.html` =
   `<om-film><om-scene><om-shot><video src="clip.mp4"></video><om-title>Exact</om-scene></om-film>`
   Repair only the malformed markup. Preserve the one video and title and do not
   invent timing or wrapper elements.
5. `reject-coordinate-edit`: Starting `film.html` =
   `<om-film><om-scene><om-shot><video src="clip.mp4"></video><om-title>Later</om-title></om-shot></om-scene></om-film>`
   The author asks: “make the title start at absolute 3s.” Express this with one
   cue named `title-at-three` at 3s and bind the title to it. Do not author a
   `start`, `begin`, `end`, `until`, frame number, or track.

Return every case exactly once and preserve the case IDs.
