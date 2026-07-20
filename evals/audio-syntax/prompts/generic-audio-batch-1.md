Generate five Onmark screenplays. Return only the JSON object required by the output schema.

Onmark mini-spec:
- The document root is `<film>`. A film contains optional film-wide audio and one or more `<scene>` elements.
- A scene contains one or more `<shot>` elements.
- A shot may contain `<video src="..."/>`, `<title>text</title>`, `<cta>text</cta>`, and shot-local audio. Emit only the children requested by each case.
- Absolute cues use exactly `<cues><cue id="name" time="1s"/></cues>`; an overlay refers to one with `cue="name"`.
- Source media determines duration. Never author `start`, `end`, `until`, `duration`, `volume`, timeline tracks, frame numbers, or CSS/TypeScript.
- `<audio kind="music" src="..."/>` is a direct child of `<film>`. It begins at film start and may cross scene and shot boundaries.
- `<audio kind="sound-effect" src="..."/>` is a direct child of `<shot>`. It begins at shot start, or at the optional local `delay="..."`.
- Audio accepts optional exact `gain="N%"`; omit it for the 100% default.
- Durations use exact `s` or `ms` values. Attribute names and element names are case-sensitive.
- Emit only requested facts. Do not invent IDs, cues, attributes, elements, or wrapper containers.

Cases:
1. `music-bed`: one scene with two shots. The videos are `media/opening.mp4` and `media/product.mp4`. Add film-wide `audio/bed.wav` at 25% gain.
2. `delayed-effect`: one scene and one shot with `media/card.mp4`. Add `audio/pop.wav` 250ms after that shot begins, at 40% gain.
3. `cross-scene-music`: two scenes, each with one shot using `media/a.mp4` and `media/b.mp4`. Add one film-wide `audio/theme.wav` at 30% gain. Do not duplicate the audio per scene.
4. `overlay-and-music`: declare cue `offer` at 1s in a `<cues>` block, then one scene and one shot containing `media/offer.mp4` and `<cta cue="offer">Buy now</cta>`. Add film-wide `audio/offer-bed.wav` at 20% gain.
5. `no-audio-control`: one scene and one shot containing only `media/plain.mp4`. Do not add audio.

Return every case exactly once and preserve the case IDs.
