# Gate-eight native backdrop-layout evidence

This record covers the first admitted path that places one or more native video
streams above browser-owned pixels. HTML and CSS remain the layout language.
Chromium returns bounded layout evidence during a layout-only preflight; Rust
validates that evidence and owns native composition.

## Correctness evidence

The real-process conformance uses Chrome and `FFmpeg`, not a mocked browser:

- `preserves_backdrop_layout_across_whole_local_and_worker_execution` renders
  two consecutive shots as one whole-film artifact, two independent partition
  artifacts, one local partition sequence, and one assembled worker sequence.
  Whole and partition artifacts have equal canonical raw-RGBA frames. Local and
  assembled videos decode to equal frames.
- `renders_multiple_backdrop_videos_in_one_shot` places two videos with exact
  `object-fit: cover` geometry in one shot. Independent worker captures have
  equal canonical raw-RGBA frames.
- the runtime suite proves that layout-only loading does not decode video, that
  invalid authored geometry remains a typed runtime failure, and that the
  protocol media bound is enforced before adapter work begins.

The production `inspect --json` boundary reports
`visualMode: "separableBackdrop"` and `nativeMedia: 2` for the two-video
fixture. The same render-unit and compositor contracts are used by local and
worker execution; there is no browser-composited fallback after the capability
has been declared.

This proof is intentionally scoped to the declared native pixel owner. A direct
same-fixture comparison with `browserComposite` diverged at the first raw frame:
Chromium and `FFmpeg` do not share one decoder, color-conversion, or resampling
algorithm. `separableBackdrop` therefore guarantees exact CSS geometry and
repeatable native pixels, not cross-renderer pixel identity. Authors requiring
Chromium's media rasterization keep `browserComposite`.

## Comparative measurement

The measurement compares the complete uncached release pipeline for the same
1,920×1,080, 30 fps, 60-frame HTML composition. Each shot contains two
640×360 copies of one CFR BT.709 limited-range H.264 source. The control lets
Chromium decode and composite both videos. The candidate differs only by
declaring `separableBackdrop`.

Runs alternated between paths. Times are milliseconds from `onmark benchmark
--runs 1`; memory is the maximum sampled sum of RSS for the command process and
its descendants. Shared pages may therefore be counted more than once. The
absolute RSS values are meaningful only within this same-host comparison.

| run | browser total | native total | browser capture | native capture | browser KiB | native KiB |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 5,185 | 5,257 | 4,410 | 4,480 | 5,202,308 | 5,272,548 |
| 2 | 5,098 | 5,433 | 4,350 | 4,653 | 5,207,364 | 5,274,836 |
| 3 | 5,063 | 5,357 | 4,309 | 4,579 | 5,206,980 | 5,267,252 |
| median | 5,098 | 5,357 | 4,350 | 4,579 | 5,206,980 | 5,272,548 |

The candidate median is 5.08% slower and its sampled process-tree RSS is 1.26%
higher. No performance threshold was frozen before this experiment, so these
numbers are descriptive evidence rather than a retroactive pass criterion.
They show that exact multi-video layout currently has a small cost on this
hardware, not a speed advantage. Future work may deduplicate identical source
decodes or remove repeated browser-PNG decoding, but neither optimization is
part of this admission without its own equivalence proof.

An earlier debug-build run was rejected: unoptimized Rust PNG decoding made the
candidate appear roughly twice as slow while the control delegated PNG decoding
to optimized `FFmpeg`. Performance claims must use release binaries.

## Measurement environment

```text
date=2026-07-28
base-commit=2e7e548ab5e309264477239e8c7ca1f4c15cc462
host-model=Mac17,3
host-chip=Apple-M5
host-memory=32GiB
browser=Google-Chrome-150.0.7871.187
ffmpeg=8.1.2
ffprobe=8.1.2
rustc=1.97.0
node=26.4.0
binary=target/release/onmark
encoder=libx264-medium-crf18-4threads
profile=1920x1080-30fps-60frames
```
