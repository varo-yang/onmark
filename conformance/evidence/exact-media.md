# Exact-media admission

This record admits complete VFR timing, exact source-local video treatment, and
native trim/speed execution as separate claims. It does not treat two different
decoder/color paths as pixel-interchangeable.

## Authored language

Two retained live-model comparisons cover twenty editing requests each:

| Comparison | Correct outputs | Authored bytes | Decision |
| --- | ---: | ---: | --- |
| separate trim edges | 20/20 | 3,482 | rejected |
| `trim="start..end"` | 20/20 | 3,288 | admitted |
| `loop="count"` + `hold` | 20/20 | 2,946 | rejected |
| `plays="count"` + `hold-last` | 20/20 | 3,000 | admitted |

The second winner is slightly longer but keeps `plays` unambiguously equal to
the total number of passes and leaves HTML's boolean `loop` spelling untouched.
Cases, prompts, grader, settings, baselines, and raw outputs live under
`evals/video-editing-syntax/` and `evals/media-continuity-syntax/`.

Run both comparisons:

```bash
cargo xtask eval video
```

## Exact source timing

`ffprobe` first freezes normalized stream facts, then requests the complete
best-effort timestamp sequence for the selected visual stream. Equal intervals
prove an exact rational CFR; unequal intervals produce a `VideoFrameMap` with
one rational media timebase and every half-open frame boundary. Rust transports
that map unchanged; the browser runtime performs a bounded binary search
without allocating a converted copy on each frame. The probe retains at most
sixteen MiB from each process pipe, matching the browser contract's
100,000-boundary ceiling without making capture unbounded. The selected
rational frame interval is projected into browser seconds once; its neighboring
boundaries derive the callback tolerance, and an interval with no representable
interior seek second is rejected instead of accepting an adjacent frame.

The retained media-seek test generated 30 fps, `30000/1001`, 24-to-30,
30-to-24, and alternating-interval VFR H.264 inputs. For the non-monotonic
request sequence `17 → 3 → 29 → 17`, two independent browser sessions and two
independent native extractions each selected stable repeated and distinct source
frames. A local macOS arm64 run on 2026-07-27 used Playwright Chromium headless
shell 150.0.7871.0 and FFmpeg 8.1.2:

| Case | Browser seek/capture | Native extraction |
| --- | ---: | ---: |
| 30 fps CFR | 79.91 ms | 19.69 ms |
| `30000/1001` CFR | 77.12 ms | 19.51 ms |
| 24-to-30 CFR | 75.51 ms | 19.19 ms |
| 30-to-24 CFR | 77.80 ms | 20.23 ms |
| alternating VFR | 76.03 ms | 19.27 ms |

These are four-seek totals, not an end-to-end throughput comparison. Chromium
and `FFmpeg` use different decode/color paths and therefore are checked for
source-frame identity and independent repeatability rather than equal RGBA
hashes.

## Native trim and speed

The native layered path realizes the Rust-owned source interval and exact
playback ratio in one closed FFmpeg selection formula. Repeated playback and
final-frame hold deliberately remain on browser composition.

The retained real-process test uses two independently materialized partition
sets over disjoint half-second trims at half speed. It requires every partition
to enter the admitted native path, compares local and worker frame-artifact
sequences by canonical raw RGBA, renders the local partition sequence, assembles
the worker artifacts through the same final encoder, and compares decoded video
and audio outputs.

Run the proofs:

```bash
ONMARK_BUNDLER=/path/to/onmark-bundle \
ONMARK_PORTABLE_CHROME=/path/to/chrome-headless-shell \
ONMARK_FFMPEG=/path/to/ffmpeg \
ONMARK_FFPROBE=/path/to/ffprobe \
cargo test -p onmark-render --test render \
  preserves_source_edits_across_local_and_worker_partition_execution \
  -- --exact --ignored --nocapture
```

```bash
ONMARK_HEADLESS_SHELL=/path/to/chrome-headless-shell \
ONMARK_FFMPEG=/path/to/ffmpeg \
ONMARK_FFPROBE=/path/to/ffprobe \
cargo test -p onmark-render --test media_seek \
  validates_admission_and_cfr_decode_paths \
  -- --exact --ignored --nocapture
```

## Browser repetition and final-frame hold

The retained `media-continuity.html` fixture keeps two one-second shots while
combining aligned trims, exact speed, four complete plays, and a 200 ms
final-frame hold. The production executor renders it once as a whole film and
again as two independently materialized Render Units, then requires equal
decoded video hashes, equal final audio hashes, and the same 60-frame result.
This proves that `plays` and `hold-last` cross the real Chromium/FFmpeg boundary
without creating a second timeline or losing partition equivalence.

```bash
ONMARK_BUNDLER=/path/to/onmark-bundle \
ONMARK_PORTABLE_CHROME=/path/to/chrome-headless-shell \
ONMARK_FFMPEG=/path/to/ffmpeg \
ONMARK_FFPROBE=/path/to/ffprobe \
cargo test -p onmark-render --test render \
  renders_repeated_and_held_media_equally_as_one_or_two_units \
  -- --exact --ignored --nocapture
```

A new browser-only treatment must prove source selection and whole/partition
equivalence. A new native treatment, codec, decoder, or native VFR path must
also supply independent local/worker artifact evidence. No capability inherits
these admissions by analogy.
