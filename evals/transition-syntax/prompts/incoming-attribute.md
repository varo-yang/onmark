You are generating minimal Onmark screenplays for ten independent
shot-transition tasks.

Return one JSON result for every case ID below. Each `screenplay` must be a
complete, well-formed HTML fragment with this structure:

```html
<om-film>
  <om-scene>
    <om-shot id="first">
      <video src="media/first.mp4"></video>
    </om-shot>
    <om-shot id="second" transition-in="500ms">
      <video src="media/second.mp4"></video>
    </om-shot>
  </om-scene>
</om-film>
```

`transition-in` represents an overlap from the immediately preceding shot into
the shot carrying the attribute. Its value is local to that boundary. Omit the
attribute for a hard cut. Use exactly the requested shot IDs, sources, order,
transition boundaries, and durations.

Do not calculate or author film positions. Do not use `start`, `end`, shot
`duration`, `delay`, `data-start`, `data-duration`, tracks, frame numbers, cues,
scripts, CSS, classes, extra attributes, or extra elements. Do not use tools.

Cases:

1. `one-boundary`: `intro` (`media/intro.mp4`) overlaps into `product`
   (`media/product.mp4`) for 500ms.
2. `middle-boundary-only`: order `problem`, `bridge`, `answer` with matching
   sources under `media/`; keep a hard cut from `problem` to `bridge`, then a
   350ms overlap from `bridge` to `answer`.
3. `two-boundaries`: order `first`, `second`, `third` with matching sources
   under `media/`; use 250ms from `first` to `second` and 600ms from `second` to
   `third`.
4. `hard-cuts-only`: order `a`, `b`, `c` with matching sources under `media/`;
   all boundaries are hard cuts.
5. `insert-shot`: the final order is `before`, newly inserted `inserted`, then
   `after`, with matching sources under `media/`. The former transition into
   `after` remains attached to that incoming shot, so only `inserted` to
   `after` overlaps for 400ms.
6. `remove-transition`: order `open`, `detail`, `close` with matching sources
   under `media/`. Keep the 300ms overlap from `open` to `detail`; the transition
   from `detail` to `close` has been removed and must be a hard cut.
7. `retime-transition`: `question` then `reveal`, with matching sources under
   `media/`; change their overlap to 750ms.
8. `reorder-shots`: the final order is `beta`, `alpha`, `gamma`, with matching
   sources under `media/`; only `alpha` to `gamma` overlaps for 300ms.
9. `separate-accents`: order `one`, `two`, `three`, `four`, with matching
   sources under `media/`; use 200ms from `one` to `two`, a hard cut from `two`
   to `three`, and 550ms from `three` to `four`.
10. `short-transition`: `flash` then `mark`, with matching sources under
    `media/`; overlap them for 100ms.
