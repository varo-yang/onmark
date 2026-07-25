// Release workflow tests pin the protected revision that may reach npm.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import test from "node:test";

const WORKFLOW = fileURLToPath(
  new URL("../../../../.github/workflows/desktop-release.yml", import.meta.url),
);

test("releases the merged main revision from the protected base context", async () => {
  const source = await readFile(WORKFLOW, "utf8");
  const mergedRevision = "github.event.pull_request.merge_commit_sha";

  assert.match(source, /^  pull_request_target:$/mu);
  assert.equal(source.split(mergedRevision).length - 1, 2);
  assert.doesNotMatch(source, /ref: \$\{\{ github\.sha \}\}/u);
  assert.match(source, /EVENT_NAME" == pull_request_target/u);
});
