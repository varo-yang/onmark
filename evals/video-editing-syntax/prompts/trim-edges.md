You are generating minimal Onmark screenplays for ten independent video-editing tasks.

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

Use one shot per requested video, in the requested order. A source-local edit uses
these attributes on `<video>`:

- `trim-in="12s"` selects the first source time to use;
- `trim-out="18s"` selects the exclusive source time at which to stop;
- `speed="2x"` plays the selected source interval at twice its natural rate.

Omit any attribute whose value is not requested. The media determines the shot
duration after trimming and speed are applied.

Do not calculate or author film-level positions. Do not use `start`, `end`,
`duration`, `delay`, `data-start`, `data-duration`, tracks, frame numbers, cues,
scripts, CSS, or extra elements. Do not use tools.

Cases:

1. `trim-head`: use `media/interview.mp4` from source time 12s to its natural end.
2. `trim-tail`: use `media/reveal.mp4` from its beginning until source time 7.5s.
3. `extract-subclip`: use `media/demo.mp4` from source time 4s until 10s.
4. `double-speed`: use all of `media/walkthrough.mp4` at 2x speed.
5. `slow-subclip`: use `media/detail.mp4` from 250ms until 2.25s at 0.5x speed.
6. `two-local-edits`: sequentially use `media/opening.mp4` from 1s until 4s,
   then `media/product.mp4` from 8s until 12s at 2x speed.
7. `change-only-in-point`: the final clip uses `media/take.mp4` from 3.5s until
   the already-correct out point 9s.
8. `change-only-out-point`: the final clip uses `media/cutaway.mp4` from the
   already-correct in point 2s until 6.75s.
9. `untreated-video`: use all of `media/plain.mp4` at natural speed.
10. `three-sequential-clips`: sequentially use `media/a.mp4` from its beginning
    until 2s; `media/b.mp4` from 5s until 8s at 0.5x speed; and `media/c.mp4`
    from 1s to its natural end at 3x speed.
