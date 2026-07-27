# Media continuity syntax evaluation

This frozen Gate-eight evaluation compares two spellings for complete source
passes and a final-frame hold:

- `loop="3"` with `hold="500ms"`;
- `plays="3"` with `hold-last="500ms"`.

Both arms scored 20/20 across ten tasks and two repetitions. The shorter arm
used 2,946 authored bytes; the admitted arm used 3,000. Onmark admits `plays`
and `hold-last` because they name a total count and the held frame explicitly.
This avoids assigning an integer meaning to HTML's boolean `loop` attribute.
Neither arm exposed a film-level coordinate or required duration arithmetic.

Calls used the same isolated local Codex configuration as the source-editing
evaluation:

```bash
codex exec --ephemeral --ignore-user-config --ignore-rules \
  --skip-git-repo-check --sandbox read-only --model gpt-5.6-sol \
  --config 'model_reasoning_effort="low"' \
  --output-schema output-schema.json --output-last-message raw.json \
  - < prompt.md
```

Run the frozen offline grader from the repository root:

```bash
cargo xtask eval video
```
