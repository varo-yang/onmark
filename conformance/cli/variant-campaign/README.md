# Variant campaign workload

This manual workload exercises one immutable screenplay across twenty typed
variant renders. It combines text, color, integer, and boolean fields at film,
shot, and transition scope with subtitles, music, trimmed video, GSAP motion,
and transition overlaps.

The handwritten screenplay, batch manifest, subtitle track, and variant
documents are checked in. Media bytes are deliberately not duplicated here.
For a functional local run, prepare the fixture from the repository root:

```bash
mkdir -p conformance/cli/variant-campaign/assets
cp showcases/assets/fractal.mp4 \
  conformance/cli/variant-campaign/assets/source.mp4
cp showcases/assets/pulse.wav \
  conformance/cli/variant-campaign/assets/score.wav

onmark --json batch conformance/cli/variant-campaign/batch.json \
  --subtitle conformance/cli/variant-campaign/captions.vtt
```

The checked-in source is an evidence workload, not a release smoke. Its
measured admission result and locked media facts live in
[`../../evidence/variant-campaign.md`](../../evidence/variant-campaign.md).
