Generate minimal Onmark screenplays for ten independent audio-ducking tasks.

Return one JSON result for every case ID below. Each `screenplay` must be a
complete, well-formed HTML fragment. Use this audio spelling:

```html
<om-film>
  <om-music
    src="audio/bed.wav"
    gain="60%"
    duck-to="20%"
  ></om-music>
  <om-scene>
    <om-shot>
      <video src="media/example.mp4"></video>
      <om-vo src="audio/voice.wav">Narration.</om-vo>
    </om-shot>
  </om-scene>
</om-film>
```

`duck-to` is an optional treatment on `om-music`. While any solved `om-vo`
placement is active, that music uses the exact absolute linear gain named by
`duck-to`; otherwise it uses its authored `gain` or the default 100%. Onmark
owns the smooth transition at each voice-over boundary. Do not calculate or
author those boundaries. `duck-to` must not exceed `gain`.

Use only exact integer percentages and exact `s` or `ms` durations. Preserve
every requested source, text, gain, delay, fade, scene, shot, and authored
order. Emit no unrequested ducking treatment. Do not use `start`, `end`, shot
`duration`, tracks, frame numbers, cues, scripts, CSS, classes, extra
attributes, or extra elements. Do not use tools.

Cases:

1. `basic-duck`: film-wide `audio/bed.wav` ducks to 25% while voice-over
   `audio/opening-voice.wav` says `The story stays clear.`. One scene and shot
   contain `media/opening.mp4`.
2. `base-and-duck-gain`: film-wide `audio/theme.wav` normally uses 60% gain and
   ducks to 20% while `audio/product-voice.wav` says `Focus on the product.`.
   One shot contains `media/product.mp4`.
3. `selective-tracks`: add film-wide `audio/score.wav` at 50% gain ducking to
   15%, followed by `audio/texture.wav` at 10% gain with no ducking. One shot
   contains `media/detail.mp4` and voice-over `audio/detail-voice.wav` saying
   `Only the score moves aside.`.
4. `voices-across-shots`: film-wide `audio/campaign.wav` ducks to 30%. One
   scene contains two shots: `media/first.mp4` with `audio/first-voice.wav`
   saying `First thought.`, then `media/second.mp4` with
   `audio/second-voice.wav` saying `Second thought.`.
5. `delayed-voice`: film-wide `audio/ambient.wav` normally uses 45% gain and
   ducks to 18%. One shot contains `media/walkthrough.mp4` and voice-over
   `audio/walkthrough-voice.wav`, delayed 750ms, saying
   `The narration begins after the picture.`.
6. `fades-compose`: film-wide `audio/closing.wav` normally uses 55% gain,
   ducks to 22%, fades in for 500ms, and fades out for 1s. One shot contains
   `media/closing.mp4` and `audio/closing-voice.wav` saying
   `Every treatment remains local.`.
7. `no-duck-control`: film-wide `audio/plain.wav` uses 35% gain with no
   ducking. One shot contains `media/plain.mp4` and `audio/plain-voice.wav`
   saying `Do not invent a treatment.`.
8. `remove-duck`: ducking was removed from film-wide `audio/revised.wav`;
   retain only its 40% gain. One shot contains `media/revised.mp4` and
   `audio/revised-voice.wav` saying `The ducking treatment was removed.`.
9. `retarget-duck`: film-wide `audio/interview.wav` normally uses 70% gain;
   change its duck target to 12%. One shot contains `media/interview.mp4` and
   `audio/interview-voice.wav` saying `The new target is twelve percent.`.
10. `policy-without-current-voice`: film-wide `audio/instrumental.wav` normally
    uses 50% gain and retains a 20% duck target even though the current film has
    no voice-over. One shot contains `media/instrumental.mp4`.

Return every case exactly once and preserve the case IDs.
