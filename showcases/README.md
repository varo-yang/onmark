# Onmark showcases

These compositions exercise the public HTML authoring surface and the released
`onmark` command. Each film owns a distinct visual language; this directory is
not a template library.

## Render

Render one film from the repository root after installing `@onmark/cli`:

```bash
onmark render showcases/liquid-type.html \
  --output showcases/renders/liquid-type.mp4 \
  --width 1920 \
  --height 1080
```

`caption-documentary.html` also imports the checked-in WebVTT fixture:

```bash
onmark render showcases/caption-documentary.html \
  --subtitle showcases/assets/field-notes.vtt
```

Rendered videos stay outside version control. Source, local assets, and the
exact command remain reviewable here.

## Catalog

| Film                  | Visual language                      | Public path exercised       |
| --------------------- | ------------------------------------ | --------------------------- |
| `analog-collage`      | tactile editorial collage            | HTML, CSS, GSAP             |
| `caption-documentary` | monochrome field documentary         | video, music, WebVTT, GSAP  |
| `code-morph`          | source-to-pixel transformation       | HTML, CSS, GSAP             |
| `data-river`          | animated data narrative              | Canvas 2D, GSAP             |
| `editorial-fold`      | newspaper-scale typography           | HTML, CSS, GSAP             |
| `fashion-rhythm`      | high-contrast fashion film           | HTML, CSS, GSAP             |
| `glass-product`       | spatial product interface            | CSS 3D, GSAP                |
| `isometric-machine`   | mechanical isometric scene           | CSS 3D, GSAP                |
| `kinetic-signal`      | typographic broadcast signal         | HTML, CSS, GSAP             |
| `liquid-type`         | fluid kinetic typography             | CSS gradients, GSAP         |
| `media-mosaic`        | picture-in-picture media composition | native video, music, CSS 3D |
| `neon-transit`        | deep neon tunnel                     | CSS 3D, GSAP                |
| `network-pulse`       | distributed graph visualization      | Canvas 2D, GSAP             |
| `noir-shadows`        | cinematic light and silhouette       | HTML, CSS, GSAP             |
| `onmark-manifesto`    | continuous brand manifesto           | music, HTML, CSS, GSAP      |
| `particle-current`    | 4,800-path procedural field          | Canvas 2D, GSAP             |
| `ribbon-sculpture`    | parametric line sculpture            | Canvas 2D, GSAP             |
| `shader-aurora`       | fragment-shader light field          | raw WebGL, GSAP             |
| `sports-broadcast`    | live scoreboard graphics             | HTML, CSS, GSAP             |
| `three-constellation` | spatial star system                  | Three.js, GSAP              |

All twenty films are one continuous semantic shot rather than a sequence of
presentation cards. Their checked-in inputs use no remote assets, ambient
clocks, unseeded randomness, or self-advancing image containers.
