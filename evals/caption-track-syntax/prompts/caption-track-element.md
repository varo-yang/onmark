Generate minimal Onmark screenplays for eight independent caption-track tasks.

Return one JSON result for every case ID below. Each `screenplay` must be a
complete, well-formed HTML document using this caption-track spelling:

```html
<om-film>
  <om-caption-track
    id="en"
    src="captions/en.vtt"
    lang="en"
  ></om-caption-track>
  <om-scene>
    <om-shot>
      <video src="media/example.mp4"></video>
    </om-shot>
  </om-scene>
</om-film>
```

Each direct `om-caption-track` child declares one external caption track. `id`
is the stable, case-sensitive track identity, `src` is a
screenplay-relative SRT, WebVTT, or ASS file, and `lang` is its language tag.
Preserve declaration order. A render selects tracks outside the screenplay by
their IDs; return that ordered selection in `selectedTracks`. Selecting more
than one track burns them in together.

Projected cues become `om-caption` elements carrying the declaration identity
as `data-track`, so authored CSS may style a track with
`om-caption[data-track="en"]`. Do not author cue text or timing in the HTML.

Every screenplay must retain exactly one scene and one shot containing the
requested video. Use no scripts, timing attributes, cues, ordinary track
elements, extra caption attributes, or unrequested elements. Do not use tools.

Cases:

1. `single-default`: declare `en` from `captions/en.vtt`, language `en`; select
   `en`; use `media/intro.mp4`.
2. `localized-choice`: declare `en` from `captions/en.srt`, language `en`, then
   `zh` from `captions/zh.srt`, language `zh-CN`; select only `zh`; use
   `media/product.mp4`.
3. `bilingual-open`: declare `en` from `captions/en.ass`, language `en`, then
   `ja` from `captions/ja.ass`, language `ja`; select `en` then `ja`; use
   `media/interview.mp4`.
4. `replace-source`: retain track `en` and language `en` but change its source
   to `captions/final-en.vtt`; select `en`; use `media/cut.mp4`.
5. `change-language`: declare track `pt` from `captions/pt-br.vtt` and correct
   its language to `pt-BR`; select `pt`; use `media/launch.mp4`.
6. `remove-track`: an obsolete Spanish track was removed. Declare only `en`
   from `captions/en.vtt`, language `en`; select `en`; use
   `media/story.mp4`.
7. `three-track-order`: declare `fr`, `de`, and `en` in that order from their
   matching `.vtt` files and matching language tags; select `de` then `en`;
   use `media/demo.mp4`.
8. `styled-track`: declare `narration` from `captions/narration.srt`, language
   `en`; select it; use `media/editorial.mp4`. Add exactly this authored CSS:
   `om-caption[data-track="narration"] { color: #ffd24a; }`.

Return every case exactly once and preserve the case IDs.
