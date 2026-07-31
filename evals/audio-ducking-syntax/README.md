# Audio-ducking syntax evaluation

This Gate-eight evaluation compares two local spellings for lowering a music
bed while solved voice-over is active:

- `duck-to="25%"` on `om-music`;
- `voice-gain="25%"` on `om-music`.

Both candidates name an absolute linear music gain rather than a reduction
ratio. Neither asks the author to calculate voice-over intervals, attack, or
release positions. The ten cases cover default and authored base gain,
selective treatment of parallel beds, voice-over across shots, delayed
narration, composition with fades, removal and retargeting, an untreated
control, and retention of a policy when the current film has no voice-over.

Both arms scored 20/20 across ten tasks and two independent repetitions.
`duck-to` produced 5,724 authored bytes versus 5,772 for `voice-gain`. Onmark
admits `duck-to` because it is shorter and its domain verb cannot be mistaken
for the amplitude of the voice-over track itself.

The four calls ran in an empty logical workspace with a read-only sandbox,
personal configuration and repository rules disabled, and structured output
enforced by `output-schema.json`. The checked raw JSON files are the model's
final outputs. WebSocket transport was unavailable during the experiment, so
Codex used its built-in HTTPS fallback without changing the model, prompt,
sandbox, or output contract. CI only parses and regrades those files; it never
calls a live model.

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
cargo xtask eval audio-ducking
```
