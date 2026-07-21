# Gate-seven layered-media admission

This record admits the layered native-media candidate for production
integration. It does not itself change the authoritative render path. The
explicit capability, versioned plan fact, executor branch, and whole/partition
conformance remain separate implementation work.

## Reviewed implementation

- commit: `abb391bb88efb97b436583245e71488e4da489b2`
- fixture: `sha256:8a1ed5d5e93f975da60b1d9632ca56c3ececdf17d34bea5cc5cea3d833ecd515`
- profile: 1,920×1,080, 30 fps, 60 frames
- samples: five alternating Chromium-media baseline and layered-media runs
- environment: `sha256:17d745740548cfdedd317bd2a336d4796d339164b0b3a21fbf5fc5e7eed72246`

The environment identity is the SHA-256 of this newline-delimited manifest,
without a trailing newline:

```text
onmark-layered-admission-v1
host-model=Mac17,3
host-chip=Apple-M5
host-memory=32GiB
virtualizer=colima-0.10.3-lima-2.1.4-vz
vm-image-sha256=b0992ab88f5a3c0c436bbb3065c01466f20dc1dd0eb0a60299d410176f21a1c3
vm=ubuntu-24.04.4-arm64-4cpu-8gib-30gib
kernel=6.8.0-117-generic
commit=abb391bb88efb97b436583245e71488e4da489b2
browser-archive-sha256=b4b0beecc6559db6f3b8bdddaae27ebb173dfb96968cbfa60e815bbb9280c0d8
browser-binary-sha256=4cb6b41db8a47bf2ce15138223ca444e9d82add7942ec56ce49b27bdce84cb93
browser-version=Chromium-149.0.7827.0
ffmpeg=ffmpeg version 6.1.1-3ubuntu5 Copyright (c) 2000-2023 the FFmpeg developers
encoder-threads=1
launch=isolated-worker
profile=1920x1080-30fps-60frames
```

## Raw performance samples

Times are end-to-end milliseconds. Memory is incremental process-tree peak RSS
in KiB.

| run | baseline ms | layered ms | baseline KiB | layered KiB |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 22,094.87 | 5,695.73 | 741,020 | 592,300 |
| 1 | 21,261.38 | 5,291.32 | 733,684 | 585,012 |
| 2 | 20,917.23 | 5,189.67 | 732,752 | 584,180 |
| 3 | 22,219.95 | 5,252.45 | 730,644 | 584,192 |
| 4 | 28,926.15 | 6,567.89 | 732,736 | 584,196 |
| median | 22,094.87 | 5,291.32 | 732,752 | 584,196 |

The layered path uses 23.95% of the baseline wall time and 79.73% of its peak
RSS. Both frozen thresholds pass: wall time is below 50%, and peak RSS is below
85%. Every run also produced the same canonical raw-RGBA sequence within its
path.

## Rejected result and correction

The first locked run left x264 thread count host-dependent. It reached 25.27%
of baseline wall time but 87.74% of baseline RSS, so admission failed. Limiting
only the candidate passed, but review rejected that comparison because it mixed
pipeline gains with candidate-only encoder tuning. Commit `abb391b` instead
bounds both the production baseline and candidate to one encoder thread. The
table above is the subsequent clean-commit result.

## Correctness evidence

Shared Linux CI run
[`29827261114`](https://github.com/varo-yang/onmark/actions/runs/29827261114)
passed cold repeatability, whole-versus-partition equality, the four-level
BT.709 patch bound, and exact CFR selection including 24-to-30 conversion and
nonzero partition starts. The performance machine repeated the 1,920×1,080
whole/partition check before admission; all 60 canonical frames matched.
