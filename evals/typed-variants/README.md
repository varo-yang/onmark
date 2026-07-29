# Typed-variant authoring evaluation

This frozen evaluation compares three presentation-only input surfaces:
declarative HTML bindings, an explicit typed module binding, and typed source
placeholders. Six generation, local-edit, and repair tasks run twice per arm.
All arms use the same four value types, defaults, override values, semantic
video structure, and prohibition on authored timing.

All three arms scored 12/12. Across the twelve outputs, placeholders used 5,443
authored bytes, declarative bindings used 5,708, and module bindings used
8,723. The 265-byte placeholder advantage did not justify expanding values into
authored source before parsing: that would make the default document
unreadable, change bundle bytes across variants, and add a second source
transformation boundary. Module binding preserved readable HTML but required
arbitrary DOM code, so the compiler could not prove field-to-region
dependencies or literal-text sinks.

An initial prompt left default boolean visibility implicit. Both declarative
pilot outputs omitted `hidden` for a field whose default was false, exposing a
gap between the claimed readable fallback and the graded contract. Those pilot
outputs are excluded. All three prompts now require authored or expanded text,
CSS properties, and visibility to match declared defaults, and every arm was
rerun.

The admitted declarative arm keeps defaults as ordinary readable content.
Rust owns field declarations, value validation, canonical encoding, and binding
placement. The browser receives typed immutable values and applies only
compiler-approved text, CSS-property, and visibility operations. This
evaluation admits that authoring direction; production conformance must still
prove every architectural claim.

Calls ran in this directory with personal configuration and repository rules
disabled. The prompts prohibited tool use. `variantJson` is a structured-output
harness detail: the grader decodes it as the same flat JSON object that the
candidate contract consumes. The service fell back from WebSocket to HTTPS;
transport retries were not counted as language failures. Each scored call used:

```bash
codex exec --ephemeral --ignore-user-config --ignore-rules \
  --skip-git-repo-check --sandbox read-only \
  --model gpt-5.6-sol --config 'model_reasoning_effort="low"' \
  --output-schema output-schema.json --output-last-message raw.json \
  - < prompt.md
```

Run the frozen offline grader from the repository root:

```bash
cargo xtask eval variant
```

## Execution evidence

`measurements.json` records one implementation check on an Apple M5 MacBook Air
with Chrome 150 and FFmpeg 8.1.2. The 320×180 fixture contains 52 frames in
three dependency regions. A cold render took 3,552 ms. Repeating the exact
variant reused 3/3 regions and 52/52 frames, reducing capture from 3,179 ms to
3 ms and total time to 378 ms. Changing only the opening-shot text reused the
unaffected closing region and 22/52 frames. The transition region correctly
changed because it evaluates both adjacent shots.

These are observed measurements, not a cross-machine performance threshold.
The durable contract is enforced separately: the Render Unit test proves that
the changed field alters only dependent distributed artifact identities, while
the real batch run proves the local cache reports the same region reuse.
