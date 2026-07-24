# Native HTML authoring evaluation

This frozen evaluation compares the current screenplay-plus-presentation
surface with a strict native HTML profile. It tests five from-scratch projects
and five local edit or repair tasks, twice per arm. The HTML profile keeps
Onmark's semantic elements and timing rules while allowing native classes,
nested DOM, CSS, and an inline motion module in one file.

The HTML arm scored 20/20. The current screenplay arm scored 16/20: both
repetitions invented nonexistent `definePresentation` and `h` APIs when real
nested or decorative DOM required the advanced presentation boundary. The
ordinary screenplay cases remained reliable. Across all outputs, HTML used 20
authored files instead of 46 and 13,618 authored bytes instead of 14,054. The
result therefore supports replacing the surface rather than adding HTML as a
second permanent language.

An initial edit batch left cue-container ownership implicit and both arms placed
cues inside shots. Those pilot outputs are excluded. The checked prompts state
the shared film-level containment rule and the four affected calls were rerun.
Before freezing the HTML surface, its semantic prefix was shortened from
`onmark-` to `om-`. The same model, cases, and settings retained a 20/20 score;
the checked raw HTML outputs are from that final spelling.

Calls ran in the evaluation directory with personal configuration and
repository rules disabled. The prompt explicitly prohibited tool use. The
service repeatedly fell back from WebSocket to HTTPS; transport retries were
not counted as language failures. Each scored call used this shape:

```bash
codex exec --ephemeral --ignore-user-config --ignore-rules \
  --skip-git-repo-check --sandbox workspace-write \
  --model gpt-5.6-sol --config 'model_reasoning_effort="low"' \
  --output-schema output-schema.json --output-last-message raw.json \
  - < prompt.md
```

Run the frozen offline grader from the repository root:

```bash
cargo xtask eval html
```

This evaluation admits a language direction, not the temporary converter used
by the feasibility prototype. Production migration must parse the strict HTML
profile directly into Source AST and must preserve Rust as the sole timing
authority.
