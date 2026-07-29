<h1 align="center">Onmark</h1>

<p align="center"><strong>Write the film. Own every frame.</strong></p>

<p align="center">
  <a href="https://www.npmjs.com/package/@onmark/cli"><img src="https://img.shields.io/npm/v/%40onmark%2Fcli?color=111214" alt="npm version"></a>
  <a href="https://github.com/varo-yang/onmark/actions/workflows/ci.yml"><img src="https://github.com/varo-yang/onmark/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-f04d32" alt="MIT license"></a>
</p>

<p align="center">
  <a href="https://onmark-cdn-1313593665.cos.ap-guangzhou.myqcloud.com/onmark-hero-compiled-draft-v2.mp4">
    <img src="docs/onmark-hero.gif" alt="Onmark screenplay compiling into a rendered composition" width="720">
  </a>
</p>

Onmark is a screenplay-first video compiler for people and agents. Write one
HTML document with semantic film elements, ordinary CSS, and optional seekable
motion. Onmark compiles the authored intent into an exact timeline, renders it
through Chromium, mixes media with FFmpeg, and writes a delivery-ready video.

```text
screenplay → Timeline IR → Render Units → browser frames + audio → video
```

## One timeline. Three visual languages.

Each film is one authored HTML document rendered by the same deterministic CLI.

<table>
  <tr>
    <td width="33%" align="center">
      <a href="https://onmark-cdn-1313593665.cos.ap-guangzhou.myqcloud.com/onmark-luminous-story.mp4">
        <img src="docs/showcase-luminous.gif" alt="Luminous Story: chromatic light and kinetic typography">
      </a>
    </td>
    <td width="33%" align="center">
      <a href="https://onmark-cdn-1313593665.cos.ap-guangzhou.myqcloud.com/onmark-exact-system.mp4">
        <img src="docs/showcase-system.gif" alt="Exact System: black-and-lime deterministic interface">
      </a>
    </td>
    <td width="33%" align="center">
      <a href="https://onmark-cdn-1313593665.cos.ap-guangzhou.myqcloud.com/onmark-printed-motion.mp4">
        <img src="docs/showcase-printed.gif" alt="Printed Motion: monochrome editorial choreography">
      </a>
    </td>
  </tr>
  <tr>
    <td align="center"><strong>Luminous Story</strong><br><sub>Chromatic atmosphere</sub></td>
    <td align="center"><strong>Exact System</strong><br><sub>Deterministic interface</sub></td>
    <td align="center"><strong>Printed Motion</strong><br><sub>Editorial choreography</sub></td>
  </tr>
</table>

## Quick start

Install the desktop CLI on macOS arm64, Linux x64, or Windows x64:

```bash
npm install --global @onmark/cli
```

Create `film.html`:

```html
<om-film>
  <om-scene>
    <om-shot duration="3s">
      <om-title>Hello, motion.</om-title>
    </om-shot>
  </om-scene>
</om-film>
```

Render it:

```bash
onmark render film.html
```

The command writes `renders/film.mp4` without overwriting an existing file.
Validate or inspect the same production plan without launching Chromium:

```bash
onmark check film.html
onmark inspect film.html --json
onmark snapshot film.html --frame 42
onmark review film.html
onmark doctor
onmark benchmark film.html --runs 3 --json
```

`snapshot` writes a lossless production frame to
`renders/film-frame-42.png`. It uses the same render region, browser/native
path, and verified pixel contract as the complete film; it is not a preview
approximation.

`review` reuses the production region cache and writes lossless checkpoints,
`manifest.json`, and a static contact sheet under a content-addressed
`reviews/film-*/` directory. Pass `--against <prior-manifest>` to compare exact
region identities after an edit.

Declare typed presentation fields when one film needs many data variants:

```html
<om-film>
  <om-fields>
    <om-field name="headline" type="text" default="Summer edit"></om-field>
    <om-field name="accent" type="color" default="#ff4d36"></om-field>
  </om-fields>
  <om-scene>
    <om-shot duration="3s">
      <om-title data-om-text="headline">Summer edit</om-title>
      <div data-om-css="accent" style="--accent:#ff4d36"></div>
    </om-shot>
  </om-scene>
</om-film>
```

Override only the values you need:

```json
{"headline":"Autumn edit","accent":"#c56cff"}
```

```bash
onmark check film.html --variant autumn.json
onmark render film.html --variant autumn.json --output autumn.mp4
```

For a bounded campaign, `onmark batch batch.json` resolves the screenplay,
freezes its assets, and bundles its presentation once. Paths are relative to
the manifest:

```json
{
  "version": 1,
  "screenplay": "film.html",
  "renders": [
    {"variant": "variants/summer.json", "output": "renders/summer.mp4"},
    {"variant": "variants/autumn.json", "output": "renders/autumn.mp4"}
  ]
}
```

Unchanged render regions are reused across items; variant values cannot change
timing, media sources, dimensions, or output profiles.

Use the default opaque H.264/AAC MP4 for delivery, or select the
alpha-preserving ProRes 4444/PCM MOV profile by filename:

```bash
onmark render film.html --output film.mov
```

Resolution, exact rational frame rates, and imported subtitles stay explicit:

```bash
onmark render film.html \
  --output launch.mp4 \
  --width 1920 \
  --height 1080 \
  --fps 30000/1001 \
  --subtitle captions.vtt
```

Install the optional agent skill for a product-owned create/check/inspect/render
loop:

```bash
npx skills add varo-yang/onmark --skill onmark-video
```

## Why screenplay-first

- **Intent before coordinates.** Source describes scenes, shots, content, cues,
  and relationships. Rust owns absolute frame placement and timing provenance.
- **The browser remains the canvas.** Use HTML, CSS, Canvas, WebGL, Three.js,
  GSAP, media, fonts, and SVG without introducing a second layout engine.
- **One execution model.** Whole-film, partitioned, local, Lambda, and
  incremental renders consume the same verified Render Units and assembler.
- **Failures stay explainable.** Authored mistakes are diagnostics with source
  spans; browser, media, and infrastructure faults remain typed errors.

## Authoring

An Onmark film is one self-contained HTML document. Custom elements carry
screenplay meaning; ordinary HTML and inline CSS carry presentation. Optional
`data-om-motion` code exports seekable animation through adapters such as
`onmark/motion/gsap`.

The compiler supports authored video, audio, voice-over, music, sound effects,
titles, calls to action, cues, and imported SRT, WebVTT, or ASS captions.
Unknown browser components remain sequential until conformance proves that they
are safe to seek or partition.

Browse [`showcases/`](showcases/) for complete films using native media, CSS 3D,
GSAP, Canvas 2D, raw WebGL, Three.js, captions, and audio.

## Architecture

Rust owns compilation, planning, media normalization, browser control, encoding,
and artifact verification. TypeScript owns authoring bindings and browser
presentation. The compiler core has no filesystem, network, clock, Chromium,
FFmpeg, or cloud dependency.

The repository contains four Rust crates:

- `onmark-core` — model, parser, diagnostics, compiler, IR, and wire contracts;
- `onmark-media` — bounded media and subtitle normalization;
- `onmark-render` — Chromium, FFmpeg, Render Units, and verified artifacts;
- `onmark-cli` — the desktop command and composition root.

Browser packages live under [`packages/`](packages/). Rust wire types generate
the checked-in schemas and TypeScript codecs; CI rejects generated drift.

## Development

Rust 1.97.0, Node.js 26.4.0, and pnpm 11.9.0 are pinned.

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
cargo fmt --all -- --check
pnpm install --frozen-lockfile
pnpm format:check
pnpm lint
pnpm typecheck
pnpm test
cargo xtask schema --check
```

Design contracts:

- [Architecture](docs/en/architecture.md)
- [Language specification](docs/en/language-specification.md)
- [Presentation contract](docs/en/presentation-contract.md)
- [Competitive pipeline review](docs/en/competitive-pipeline-review.md)
- [中文文档](docs/zh-CN/)

Onmark is available under the [MIT License](LICENSE).
