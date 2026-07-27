You are generating minimal Onmark screenplays for ten independent media-continuity tasks.

Return one JSON result for every case ID below. Each `screenplay` must be a complete,
well-formed HTML fragment with this structure:

```html
<om-film>
  <om-scene>
    <om-shot>
      <video src="media/example.mp4"></video>
    </om-shot>
  </om-scene>
</om-film>
```

Use one shot per requested video, in the requested order. These source-local video
treatments are available:

- `trim="2s..5s"` selects source time 2s through exclusive source time 5s;
- `speed="2x"` plays the selected source interval at twice its natural rate;
- `plays="3"` plays the selected interval exactly three times in total;
- `hold-last="500ms"` holds the final frame after all plays for 500ms.

Omit every treatment that was not requested. Media treatments determine shot
duration. Do not calculate or author film-level positions. Do not use `start`,
`end`, `duration`, `delay`, `data-start`, `data-duration`, tracks, frame numbers,
cues, scripts, CSS, or extra elements. Do not use tools.

Cases:

1. `loop-twice`: play all of `media/logo.mp4` exactly twice.
2. `loop-four`: play all of `media/pattern.mp4` exactly four times.
3. `hold-end`: play all of `media/product.mp4`, then hold its final frame for 750ms.
4. `trim-loop`: play source time 2s through 5s of `media/demo.mp4` exactly three times.
5. `speed-loop`: play all of `media/pulse.mp4` at 2x speed exactly five times.
6. `trim-speed-hold`: play source time 250ms through 2.25s of `media/detail.mp4`
   at 0.5x speed, then hold the final frame for 1s.
7. `loop-and-hold`: play all of `media/spinner.mp4` exactly three times, then
   hold the final frame for 500ms.
8. `two-treatments`: sequentially play all of `media/opening.mp4` exactly twice;
   then play all of `media/closing.mp4` once and hold its final frame for 2s.
9. `untreated`: play all of `media/plain.mp4` once with no hold.
10. `change-loop-count`: the final video uses `media/take.mp4`, source time 1s
    through 4s at 2x speed; change only its total play count to six.
