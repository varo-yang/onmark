# Objective visual-feedback admission

> Measured on 2026-07-31 from the working tree based on `c348dcd`, before the
> visual-feedback change received its final commit identity.

## Contract under test

Gate nine may report a browser observation only when it is objective, bounded,
and captured inside the existing production transaction. The admitted slice is:

- an active shot or semantic overlay with no positive rendered area;
- an active overlay whose own `overflow: hidden` or `overflow: clip` box hides
  content horizontally; and
- the corresponding vertical clipping condition.

The runtime does not inspect arbitrary DOM, score aesthetics, infer author
intent, mutate source, or launch a second renderer. `Confirm(frame)` measures
the already-staged semantic elements after exact-frame effects and returns a
maximum of 256 canonical findings. If a frame contains more, inspection retains
the first 256 in node-and-issue order instead of failing the frame. Every worker
artifact frame record keeps its verified PNG, raw-RGBA fingerprint, and findings
together. Local capture, distributed capture, cache reuse, and `review`
therefore consume the same evidence.

Admission required all three of these thresholds:

| Criterion | Required |
| --- | ---: |
| injected-defect detection | at least 90% |
| clean-corpus false positives | at most 2% |
| median review overhead | at most 10% |

## Locked local measurement

- host: macOS arm64
- browser: Chrome for Testing 148.0.7778.97
- capture mode: portable screenshot
- profile: 1920 × 1080
- corpus: all 25 checked-in showcases
- corpus checkpoints: 29 semantic shot/overlay midpoints
- repetitions per A/B arm: 3

The focused browser fixture applied one defect at a time after exact-frame
effects. The runtime detected all three defect kinds: 3 / 3, or 100%.

Every showcase passed through the production bundler, runtime, media probe, and
Chromium session. The clean corpus produced 0 findings across 29 checkpoints:
0% observed false positives.

The overhead arm used the admitted inspection. The control used the identical
task with only `inspectPresentation` returning an empty collection; no other
source, bundle, browser, or test policy changed.

| Arm | Runs | Median |
| --- | --- | ---: |
| inspection disabled | 15.52 s, 15.61 s, 15.61 s | 15.61 s |
| inspection enabled | 15.84 s, 15.68 s, 15.64 s | 15.68 s |

The observed median increase was 0.45%. These host measurements justify this
narrow DOM measurement only; they are not a portable renderer-performance
claim.

## Production and reuse evidence

`conformance/cli/visual-defect.html` enters through the released CLI shape. Its
first `review` captures an ordinary production region and publishes horizontal
and vertical clipping findings for authored node `clipped-title`. A second
review uses the same persistent cache, reports 1 / 1 reused regions, and
publishes checkpoint pixels and findings identical to the cold report.

The artifact reader validates the full frame payload, visual-evidence extent,
canonical node/issue order, and one shared SHA-256 checksum before either cold
or reused evidence reaches the report. Corrupt, oversized, unknown, duplicated,
or out-of-order records are typed artifact failures rather than silently
discarded observations.
