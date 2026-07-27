# Transparent-output admission

This record admits one alpha-preserving output rather than a general codec
matrix. `.mov` selects ProRes 4444 with PCM audio; `.mp4` remains opaque H.264
with AAC. Alpha is a render-unit fact, not a late encoder switch.

## Candidate check

The codec isolation used FFmpeg 8.1.2 on macOS arm64 with four 64×64 RGBA
frames containing zero and 127 alpha values:

| Candidate | Encoded pixel format | Decoded alpha values | Decision |
| --- | --- | --- | --- |
| ProRes 4444 | `yuva444p10le` | 0, 128 | admitted |
| QuickTime Animation | `argb` | 0, 127 | rejected |

QuickTime Animation retained the eight-bit alpha value exactly, but ProRes 4444
is the interoperable editing profile and its one-level alpha quantization is
stable and explicit. VP9/WebM was not admitted because its cross-platform alpha
decode contract was not strong enough to replace the existing MOV profile.

## End-to-end proof

The retained ignored test renders one two-shot document with transparent and
translucent authored pixels through both production paths:

1. whole-film Chromium capture directly into ProRes 4444;
2. independent whole and partition frame artifacts;
3. partition artifact assembly through the same ProRes 4444 encoder.

It requires the whole and partition raw-RGBA sequences to be exactly equal,
both MOV files to probe as `prores` / `yuva444p12le`, both decoded outputs to
contain zero and partial alpha, and the two decoded frame sequences to match.
The transparent `RenderProfile` participates in worker JSON, frame-artifact
identity, and the fixed artifact header, so an opaque cache entry cannot satisfy
the request.

Local admission environment:

- date: 2026-07-27
- browser: Playwright Chromium headless shell 150.0.7871.0, portable screenshot
- FFmpeg / ffprobe: 8.1.2
- host: macOS arm64

Run the retained proof:

```bash
ONMARK_BUNDLER=/path/to/onmark-bundle \
ONMARK_PORTABLE_CHROME=/path/to/chrome-headless-shell \
ONMARK_FFMPEG=/path/to/ffmpeg \
ONMARK_FFPROBE=/path/to/ffprobe \
cargo test -p onmark-render --test render \
  preserves_alpha_across_whole_partitioned_and_worker_output \
  -- --exact --ignored --nocapture
```

The shared locked-browser CI remains the release authority. A new alpha codec,
container, hardware encoder, or browser surface must repeat this evidence; it
cannot reuse the ProRes 4444 admission by analogy.
