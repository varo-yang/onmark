# Variant campaign evidence

> Measured on 2026-07-29 from the working tree based on `49342af`, before the
> campaign fixes received their final commit identity.

## Contract under test

One `onmark batch` invocation must resolve and freeze a screenplay once, import
one shared subtitle track into every variant, preserve the ordinary Render Unit
contract, and reuse only content-addressed region artifacts whose variant
dependencies are unchanged.

The experiment also asks a separate question: does an independently captured
cold render of this complete mixed presentation produce the same canonical
raw-RGBA sequence? Cache reuse and cold pixel repeatability are different
claims and are evaluated independently.

## Locked workload

- host: macOS arm64
- capture mode: portable screenshot
- graphics backend: Metal, with an additional SwiftShader control
- profile: 1,920 × 1,080, opaque H.264/AAC MP4, 30 fps
- duration: 435 frames, 14.5 seconds
- partition plan: 7 output regions
- variants: 20 renders over 12 text, color, integer, and boolean fields
- source video: 8 seconds, 1,920 × 1,080, 30 fps H.264,
  `sha256:7bc9fb2d1ba508982cfd9a433403db0204de3e7f6dbd281219f247def899cdaf`
- music: 14.5 seconds, 48 kHz stereo PCM,
  `sha256:209baa113e97eb3de96459941e21504020a4b7d59ab8938c19a62e0a3954f7d4`

The source is checked in under `conformance/cli/variant-campaign/`. Synthetic
media is replaceable for functional reproduction; the digests above identify
the bytes used for these measurements.

## Batch result

The batch produced all twenty videos and 8,700 output-frame instances. It
served 5,790 frames from verified artifacts, a 66.6% reuse rate. Across 140
region instances, 84 were reused, a 60% reuse rate.

| Edit scope                         | Reused frames | Reused regions |
| ---------------------------------- | ------------: | -------------: |
| one hero field                     |     315 / 435 |          5 / 7 |
| one proof or feature field         |     315 / 435 |          4 / 7 |
| one call-to-action field           |     315 / 435 |          5 / 7 |
| one transition field               |     420 / 435 |          6 / 7 |
| one film-shell field               |       0 / 435 |          0 / 7 |
| two adjacent local fields          |     420 / 435 |          6 / 7 |
| previously cached composed variant |     435 / 435 |          7 / 7 |

These results match the compiler-owned dependency scopes. A film-shell value
invalidates every region; a transition value invalidates only its overlap; the
fully repeated variant reuses every artifact.

## Findings

The workload exposed four correctness gaps, each now covered by a focused
regression test:

- batch rendering projected `--subtitle` into each job but did not import the
  shared track into the solved timelines;
- authored `display` declarations could override a false boolean binding's
  `hidden` state;
- GSAP rounds timeline boundaries to seven decimal places and rejected a valid
  fractional-frame endpoint; and
- a transition-trimmed partition incorrectly required its video placement to
  equal, rather than cover, the published output interval.

No hidden fallback or campaign-specific executor was added. The fixes remain in
the ordinary subtitle, authoring, motion, and visual-admission boundaries.

## Pixel decision

The stronger cold-repeatability criterion did not pass for this arbitrary mixed
browser-composited workload. Independent Metal snapshots differed at frames
60, 180, and 270, while frame 380 matched. Under SwiftShader, frames 180, 270,
and 380 matched, while frame 60 retained a small one-code-value difference.
Removing the SVG grain did not eliminate that difference.

Independently encoded MP4 video streams therefore also differed. Their decoded
comparison measured 52.50 dB average PSNR and 0.996505 aggregate SSIM; audio
frame hashes were equal. Those similarity metrics are useful diagnosis, not an
identity oracle.

The batch and dependency-isolation behavior is accepted. This experiment does
not admit arbitrary mixed browser composition as cold pixel-repeatable, does
not widen the native-media path, and does not weaken the existing raw-RGBA
requirements. A later experiment must isolate the remaining browser-owned
pixel instability or define a mixed visual path with independent exact
evidence.
