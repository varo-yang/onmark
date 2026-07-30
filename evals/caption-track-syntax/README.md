# Caption-track syntax evaluation

This Gate-eight evaluation compares two direct film-child declarations for one
external caption track:

- `<om-captions id="en" src="captions/en.vtt" lang="en"></om-captions>`;
- `<om-caption-track id="en" src="captions/en.vtt" lang="en"></om-caption-track>`.

The eight tasks cover one track, localized selection, simultaneous bilingual
captions, source and language edits, removal, declaration order, and
track-specific CSS. A selected-track list is output-profile input rather than
film timing; the compiler still owns every imported cue interval.

Both arms scored 16/16 across two independent repetitions. The `om-captions`
arm produced 4,778 authored bytes, compared with 5,018 for
`om-caption-track`. Onmark admits `om-captions`: one external file is a
collection of caption cues, the projected cue remains singular `om-caption`,
and the shorter spelling lost no reliability.

Native `<track>` was rejected before live comparison. HTML owns it as a void
child of `audio` or `video`; repurposing it as a direct film child would create
two incompatible meanings for one native element and make browser tree
behavior part of the screenplay contract.

The four calls ran with a read-only sandbox, personal configuration and
repository rules disabled, and structured output enforced by
`output-schema.json`. WebSocket transport failed and the CLI used its HTTPS
fallback; the checked raw JSON files are the model's final outputs. CI only
parses and regrades those files.

Run the frozen grader from the repository root:

```bash
cargo xtask eval captions
```
