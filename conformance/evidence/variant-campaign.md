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

## Initial pixel result

The stronger cold-repeatability criterion did not pass for this arbitrary mixed
browser-composited workload. Independent Metal snapshots differed at frames
60, 180, and 270, while frame 380 matched. Under SwiftShader, frames 180, 270,
and 380 matched, while frame 60 retained a small one-code-value difference.
Removing the SVG grain did not eliminate that difference.

Independently encoded MP4 video streams therefore also differed. Their decoded
comparison measured 52.50 dB average PSNR and 0.996505 aggregate SSIM; audio
frame hashes were equal. Those similarity metrics are useful diagnosis, not an
identity oracle.

This result admitted batch dependency isolation, but not persistent reuse for
arbitrary mixed browser composition. Similarity metrics did not weaken the
raw-RGBA requirement.

## Exact-raster follow-up

> Measured on 2026-07-30 from the working tree based on `db9d9af`.

The follow-up used an Apple M5, Chrome for Testing 149.0.7827.55, the portable
screenshot backend, and the canonical `SwiftShader` graphics contract at
1,920 × 1,080. It used the repository's functional substitute media:

- video: 10 seconds,
  `sha256:29855dd6e6e2a6847814be5b421d2d0fc10f644353c6200ff07fe5ada293f651`;
- music: 10 seconds,
  `sha256:851919421ed3790ed5c586fa9e3fa4ce3fdc0a1b95405d1749f5974948feffbe`.

Full artifact comparison localized the cold drift to Chromium's tiled raster
path. Differences clustered at 256-pixel tile boundaries. Disabling partial
raster alone still left three of five regions with differing frames. The
admitted contract additionally disables GPU rasterization and
runtime-selected Skia optimizations, locks sRGB, and drains all compositor
stages before readback.

Two independent browser processes with separate cache directories then
captured the complete seven-region campaign in 256.70 and 306.37 seconds.
Every one of the 435 canonical raw-RGBA frame fingerprints was equal. A
separate 75-frame, five-region CSS/GSAP boundary fixture also reproduced every
frame; its two captures took 32.63 and 33.16 seconds.

## Decision

The locked `SwiftShader` exact-raster contract admits persistent reuse across
ordinary CLI and batch processes. Its code-owned composition version changes
whenever the launch contract changes. An explicit browser path and a native
graphics override remain ephemeral because neither belongs to this evidence.
Local and distributed execution continue to consume the same Render Unit and
artifact contracts; no hidden pixel fallback or campaign-specific executor was
added.
