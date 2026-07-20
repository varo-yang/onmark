# Authored audio syntax evaluation

This frozen evaluation compares two viable Gate-four spellings for film music
and shot-local sound effects. It is language-product evidence, not a live-model
CI dependency.

Both arms received the same ten semantic tasks in two batches. Each batch was
run twice in an empty, read-only directory with personal configuration and
repository instructions disabled. The checked raw files are the model's final
structured outputs; `cargo xtask eval audio` parses and regrades them without
network access.

The semantic-elements arm scored 20/20. The generic-audio arm also scored
20/20. Reliability therefore did not distinguish them. Onmark admits
`<music>` and `<sfx>` because their element kinds encode role and legal
containment directly, while `<audio kind="...">` adds an invalid kind/parent
matrix and repeats semantic dispatch in attributes.

Pilot calls that exposed ambiguity in the shared cue and optional-child
instructions are intentionally excluded. The checked prompts are the corrected
prompts used for both scored repetitions.

Each scored call used this command shape, with the arm/batch prompt and raw
output path substituted explicitly:

```bash
codex exec --ephemeral --ignore-user-config --ignore-rules \
  --skip-git-repo-check --sandbox read-only --model gpt-5.6-sol \
  --config 'model_reasoning_effort="low"' \
  --output-schema output-schema.json --output-last-message raw.json - < prompt.md
```

Run the frozen grader from the repository root:

```bash
cargo xtask eval audio
```
