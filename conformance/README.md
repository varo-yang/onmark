# Conformance fixtures

Authored `.onmark` inputs are maintained by hand. Expected `.ast.txt`,
`.linked.txt`, and `.diagnostics.txt` files are generated golden artifacts and
are not wire formats or protocol schemas.

Regenerate goldens after intentionally changing public behavior:

```bash
ONMARK_UPDATE_GOLDENS=1 cargo test --test syntax_conformance
ONMARK_UPDATE_GOLDENS=1 cargo test --test binding_conformance
```

Review the resulting diff before committing it. Normal test runs compare
current behavior with the checked-in artifacts and never rewrite them.
