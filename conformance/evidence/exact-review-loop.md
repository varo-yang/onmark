# Exact review loop evidence

> Measured on 2026-07-29 from the working tree based on `ca8fe18`, before the
> review-loop change received its final commit identity.

## Contract under test

`onmark review` must:

- derive deterministic semantic checkpoints from solved Timeline IR and the
  production Partition Plan;
- capture ordinary complete Render Units through the existing artifact cache;
- read selected frames from verified `FrameArtifact` values without rescanning
  one artifact for every checkpoint;
- publish lossless PNGs, a static contact sheet, and one versioned manifest;
- reuse only artifact identities already admitted by the production cache; and
- compare prior and current region identities without authorizing reuse.

The command is local. It does not create a player, preview server, sparse worker
task, second timing solver, approximate frame, or capture fallback.

## Locked local sample

- host: macOS arm64
- capture mode: portable screenshot
- graphics backend: Metal
- profile: 640 × 360, opaque, 30 fps
- showcase: `showcases/exact-system.html`
- duration: 450 frames
- checkpoint policy result: 3 frames in 1 region
- capture-environment seed:
  `sha256:1111111111111111111111111111111111111111111111111111111111111111`

| Run | Total | Bundle | Capture | Reused regions | Reused frames |
| --- | ---: | ---: | ---: | ---: | ---: |
| cold | 46,792 ms | 447 ms | 46,336 ms | 0 / 1 | 0 / 450 |
| warm | 972 ms | 104 ms | 862 ms | 1 / 1 | 450 / 450 |

Both reports had review identity `f4707bfe9a85`. The warm report compared its
prior manifest as one unchanged region and no changed, added, or removed
regions.

The large cold/warm difference is expected: cold review deliberately captures
the complete production region so its artifact can be shared with rendering.
It does not buy a fast first contact sheet by creating a weaker sparse cache
identity.

## Edit isolation

`conformance/cli/review/before.html` and `after.html` contain two independent
one-second shot regions. Only the second shot's title text changes.

| Run | Total | Capture | Reused regions | Reused frames |
| --- | ---: | ---: | ---: | ---: |
| before | 4,927 ms | 4,827 ms | 0 / 2 | 0 / 60 |
| after | 2,603 ms | 2,526 ms | 1 / 2 | 30 / 60 |

The comparison reported:

```text
unchangedRegions = 1
changedRegions   = 1
addedRegions     = 0
removedRegions   = 0
```

The first region retained artifact identity
`sha256:328f73583c2d60e2ce7b84fb39c29bc316a67e3030a0844659bcfb280f9c1758`.
The changed second region received
`sha256:ba6e372987da6a46da88d459ad7a3051f970e9502d4fa14e5be430bdc8063b01`.

## Report integrity

The edited review retained six lossless checkpoints. Each manifest entry
recorded:

- absolute frame and artifact-relative position;
- region evaluation/output intervals and shot dependencies;
- semantic reason, element kind and optional authored ID;
- source spans and exact start/end timing provenance;
- frame-artifact identity;
- encoded PNG byte length and SHA-256; and
- canonical raw-RGBA SHA-256.

Repeating the default content-addressed review checked the existing manifest,
contact sheet, and every named PNG, then reported `publication = reused`.

## Local and worker symmetry

Review does not introduce an origin-specific frame type. Its only pixel input is
`onmark_render::FrameArtifact`, the same immutable contract produced by local
cache misses and `onmark worker capture`. Existing worker conformance verifies
portable request capture, whole-film/partition raw-RGBA equivalence, environment
rejection, and shared assembly in `crates/cli/tests/worker.rs`. The review path
adds no worker branch and cannot distinguish or reinterpret those pixels.
