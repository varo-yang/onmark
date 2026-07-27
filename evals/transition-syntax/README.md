# Shot-transition syntax evaluation

This Gate-eight evaluation compares two ways to declare an exact overlap
between adjacent shots:

- an explicit boundary element,
  `<om-transition duration="500ms"></om-transition>`;
- an attribute on the incoming shot, `transition-in="500ms"`.

Both candidates leave visual realization to presentation code. Neither names a
built-in effect, exposes a film coordinate, or asks the author to adjust either
shot duration. The compiler would derive the overlap window and the resulting
film duration.

The cases cover initial generation and local edits: adding, removing, moving,
and retiming transitions; inserting and reordering shots; preserving hard cuts;
and expressing two independent boundaries. A candidate must keep each
transition attached to the requested adjacent pair after the edit.

Calls run in this directory with personal configuration and repository rules
disabled. Use this shape for each prompt and repetition:

```bash
codex exec --ephemeral --ignore-user-config --ignore-rules \
  --skip-git-repo-check --sandbox read-only --model gpt-5.6-sol \
  --config 'model_reasoning_effort="low"' \
  --output-schema output-schema.json --output-last-message raw.json \
  - < prompt.md
```

Both arms scored 20/20 across ten tasks and two repetitions. The incoming
attribute produced 6,112 authored bytes versus 6,794 for the boundary element.
Onmark admits the boundary element despite the 682-byte difference: a
transition is a relationship between two shots, not a property of either shot.
The element gives that relationship its own source span, identity, classes, and
future presentation binding surface. It also makes orphaned, adjacent, or
oversized transitions locally diagnosable without inventing a second attribute
namespace on `om-shot`.

Run the frozen offline grader from the repository root:

```bash
cargo xtask eval transition
```
