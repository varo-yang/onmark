# Audio-envelope syntax evaluation

This Gate-eight evaluation compares two exact spellings for independent audio
fade-in and fade-out treatments:

- `fade-in="300ms"` and `fade-out="500ms"` on `om-music`, `om-vo`, or
  `om-sfx`;
- one optional `<om-envelope in="300ms" out="500ms"></om-envelope>` child.

Both candidates keep fades local to an audio placement. Neither moves audio,
changes its solved duration, creates a crossfade, or asks the author to
calculate a film position. The ten cases cover all three audio roles, one-sided
and two-sided fades, independent tracks, removal, retiming, preservation of
gain and delay, and a control case that must not acquire a fade.

Both arms scored 20/20 across ten tasks and two independent repetitions. The
attribute arm produced 4,914 authored bytes; the envelope arm produced 5,868.
Onmark admits the attributes because they preserve voice-over as inscription
plus media facts, avoid a new optional-child cardinality rule, and require 954
fewer bytes without reducing generation reliability.

The four calls ran in an empty directory with a read-only sandbox, personal
configuration and repository rules disabled, and structured output enforced by
`output-schema.json`. The checked raw JSON files are the model's final outputs.
CI only parses and regrades those files; it never calls a live model.

Each repetition used this command shape:

```bash
codex exec --ephemeral --ignore-user-config --ignore-rules \
  --skip-git-repo-check --sandbox read-only --model gpt-5.6-sol \
  --config 'model_reasoning_effort="low"' \
  --output-schema output-schema.json --output-last-message raw.json \
  - < prompt.md
```

Run the frozen grader from the repository root:

```bash
cargo xtask eval audio-envelope
```
