# Source-local video editing syntax evaluation

This frozen Gate-eight evaluation compares two spellings for selecting a
source-video interval while preserving Onmark's sequence-first timeline:

- `trim-in="12s" trim-out="18s"`;
- `trim="12s..18s"`.

Both arms use `speed="2x"` for an optional exact constant playback rate. Neither
arm exposes a film-level start, end, duration, track, or frame coordinate.

The two arms each scored 20/20 across ten tasks and two repetitions. The range
arm produced 3,288 authored bytes versus 3,482 for the edge arm. Onmark admits
the range spelling because it represents one domain interval, validates both
bounds together, and remains smaller without reducing generation reliability.
The compiler derives the edited duration and every absolute frame.

Calls ran in this directory with personal configuration and repository rules
disabled. WebSocket transport failed and the CLI completed every scored call
through its HTTPS fallback. Each call used this shape:

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
