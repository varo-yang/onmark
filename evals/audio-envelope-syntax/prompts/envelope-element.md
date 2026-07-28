Generate minimal Onmark screenplays for ten independent audio-fade tasks.

Return one JSON result for every case ID below. Each `screenplay` must be a
complete, well-formed HTML fragment. Use this audio spelling:

```html
<om-film>
  <om-music src="audio/bed.wav" gain="25%">
    <om-envelope in="800ms" out="1s"></om-envelope>
  </om-music>
  <om-scene>
    <om-shot>
      <video src="media/example.mp4"></video>
      <om-vo src="audio/voice.wav">
        <om-envelope in="200ms"></om-envelope>
        Narration.
      </om-vo>
      <om-sfx src="audio/hit.wav" delay="500ms">
        <om-envelope out="100ms"></om-envelope>
      </om-sfx>
    </om-shot>
  </om-scene>
</om-film>
```

Each audio element may contain at most one optional `om-envelope`. Its
independent `in` and `out` attributes ramp that audio from silence or to silence
within its already solved playback interval. They never move audio, change
duration, or create a crossfade. Omit the child when neither edge has a fade,
and omit an attribute when that edge has no fade.

Use only exact `s` or `ms` durations. Preserve every requested source, text,
gain, delay, scene, shot, and authored order. Emit no unrequested envelope. Do
not calculate or author film positions. Do not use `start`, `end`, shot
`duration`, tracks, frame numbers, cues, scripts, CSS, classes, extra
attributes, or extra elements. Do not use tools.

Cases:

1. `music-both-edges`: one film-wide `audio/bed.wav` at 25% gain, fading in for
   800ms and out for 1s. One scene and shot contain `media/opening.mp4`.
2. `effect-out-only`: one shot contains `media/card.mp4` and `audio/hit.wav` at
   70% gain, delayed 250ms, with only a 120ms fade-out.
3. `voice-in-only`: one shot contains `media/story.mp4` and voice-over
   `audio/story.wav` with text `A precise story begins.` and only a 300ms
   fade-in.
4. `all-audio-roles`: film-wide `audio/theme.wav` at 20% gain fades in and out
   for 1s. One shot contains `media/demo.mp4`, voice-over
   `audio/demo-voice.wav` with text `Watch the system respond.`, 150ms fade-in,
   and 250ms fade-out, plus `audio/click.wav` at 50% gain delayed 600ms with a
   50ms fade-in and 100ms fade-out.
5. `no-fade-control`: film-wide `audio/plain-bed.wav` at 30% gain and one shot
   containing `media/plain.mp4`. Do not add an envelope.
6. `remove-fade-in`: the fade-in was removed from film-wide
   `audio/closing.wav`; retain only its 750ms fade-out. One shot contains
   `media/closing.mp4`.
7. `retime-fade-out`: one shot contains `media/answer.mp4` and delayed
   voice-over `audio/answer.wav` with text `The answer stays exact.`. Preserve
   its 100ms delay and 200ms fade-in, and change its fade-out to 400ms.
8. `independent-effects`: one shot contains `media/controls.mp4`. Add
   `audio/click.wav` delayed 100ms with only a 20ms fade-in, followed by
   `audio/chime.wav` delayed 700ms with only a 200ms fade-out.
9. `adjacent-voice-edges`: one scene has two shots. The first contains
   `media/first.mp4` and `audio/first.wav` with text `First thought.` and only a
   180ms fade-out. The second contains `media/second.mp4` and
   `audio/second.wav` with text `Second thought.` and only a 220ms fade-in.
10. `symmetric-music`: film-wide `audio/identity.wav` fades in for 500ms and
    out for 500ms. One shot contains `media/identity.mp4`.

Return every case exactly once and preserve the case IDs.
