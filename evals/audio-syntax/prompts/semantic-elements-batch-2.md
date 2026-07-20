Generate five Onmark screenplays. Return only the JSON object required by the output schema.

Onmark mini-spec:
- The document root is `<film>`. A film contains optional film-wide music and one or more `<scene>` elements.
- A scene contains one or more `<shot>` elements.
- A shot may contain `<video src="..."/>`, `<title>text</title>`, `<cta>text</cta>`, and shot-local sound effects. Emit only the children requested by each case.
- Source media determines duration. Never author `start`, `end`, `until`, `duration`, `volume`, timeline tracks, frame numbers, or CSS/TypeScript.
- `<music src="..."/>` is a direct child of `<film>`. It begins at film start and may cross scene and shot boundaries.
- `<sfx src="..."/>` is a direct child of `<shot>`. It begins at shot start, or at the optional local `delay="..."`.
- Music and sound effects accept optional exact `gain="N%"`; omit it for the 100% default.
- Durations use exact `s` or `ms` values. Attribute names and element names are case-sensitive.
- Emit only requested facts. Do not invent IDs, cues, attributes, elements, or wrapper containers.

Cases:
1. `default-gain`: one scene and one shot containing `media/logo.mp4`. Add film-wide `audio/identity.wav` at the default gain; do not spell the default explicitly.
2. `two-effects`: one scene and one shot containing `media/demo.mp4`. Add `audio/click.wav` at 100ms with 50% gain and `audio/chime.wav` at 700ms with 35% gain.
3. `nested-paths`: one scene and one shot containing `assets/video/launch/hero.mp4`. Add film-wide `assets/audio/music/launch.wav` at 15% and shot-local `assets/audio/sfx/whoosh.wav` at 300ms and 60%.
4. `local-delay-not-timeline`: one scene and one shot containing `media/five-second-card.mp4`. Add `audio/reveal.wav` two seconds after the shot begins. Express this only as a local delay; do not introduce absolute start/end coordinates.
5. `music-and-effect`: two shots using `media/intro.mp4` and `media/outro.mp4`. Add one film-wide `audio/score.wav` at 22%. Add `audio/hit.wav` at 150ms and 70% only to the second shot.

Return every case exactly once and preserve the case IDs.
